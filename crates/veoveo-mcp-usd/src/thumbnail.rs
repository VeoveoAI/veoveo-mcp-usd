//! A stage's picture for a gallery: one frame drawn on the GPU as the caller, kept by the content
//! it was drawn from. Every cache response probes the root and contributing files as the caller,
//! so a root grant cannot expose a separately owned linked layer from a warm cache. What is kept is only
//! a small picture of a stage, under a name its content decides, and any of it can be drawn again.
use std::{
    collections::{BTreeMap, HashMap},
    future::Future,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, Weak},
    time::{Duration, SystemTime},
};

use anyhow::{Context, anyhow};
use base64::Engine as _;
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;
use usd_bevy::instance::UsdInstanceOverrides;

use crate::{
    contract::{StageThumbnail, ThumbnailRequest, ThumbnailSize},
    hub::{HubError, Probe, Reader},
    stage::{self, Scope, StageError},
    work::{self, FRAME},
};

/// Changes when what is drawn changes, so that older pictures are drawn again.
const RECIPE: &str = "3";
/// The width of a small picture. A large one is the frame as it was rendered.
const SMALL_WIDTH: u32 = 480;
/// How long a stage that could not be drawn is left before it is tried again.
const RETRY_AFTER: Duration = Duration::from_secs(60 * 60);
/// The longest one picture may take.
const DRAW_TIMEOUT: Duration = Duration::from_secs(180);
/// How many stages draw at once. Each one composes the whole stage, on a GPU others use too.
const CONCURRENT_DRAWS: usize = 4;
/// The quality of the pictures.
const JPEG_QUALITY: u8 = 82;
/// The media type of the pictures.
const MEDIA_TYPE: &str = "image/jpeg";

/// The kept pictures and the draws in progress.
#[derive(Clone)]
pub struct Thumbnails {
    directory: PathBuf,
    permits: Arc<Semaphore>,
    drawing: Arc<Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>>,
}

impl Thumbnails {
    /// Pictures are kept under `directory`, which this makes.
    pub fn new(directory: PathBuf) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&directory)
            .with_context(|| format!("making {}", directory.display()))?;
        Ok(Self {
            directory,
            permits: Arc::new(Semaphore::new(CONCURRENT_DRAWS)),
            drawing: Arc::default(),
        })
    }

    /// The picture of `request`'s stage for its caller. A refusal of the read is an error; a
    /// stage that cannot be drawn answers no picture.
    pub async fn thumbnail(
        &self,
        reader: &Reader,
        request: &ThumbnailRequest,
    ) -> Result<StageThumbnail, HubError> {
        let probe = reader
            .probe(&request.repository, &request.branch, request.path.as_str())
            .await?;
        let key = key(&probe, request.path.as_str());
        if self.reuse(reader, request, &probe, &key).await
            && let Some(found) = self.kept(&key, request.size).await
        {
            return Ok(found);
        }
        if self.failed_lately(&key).await {
            return Ok(StageThumbnail::default());
        }
        let turn = self.turn(&key);
        let _turn = turn.lock().await;
        if self.reuse(reader, request, &probe, &key).await
            && let Some(found) = self.kept(&key, request.size).await
        {
            return Ok(found);
        }
        let Ok(_permit) = self.permits.acquire().await else {
            return Ok(StageThumbnail::default());
        };
        let Ok(branch) = stage::frozen_branch(probe.revision.as_deref()) else {
            return Ok(StageThumbnail::default());
        };
        let gathered = match stage::gather(
            reader,
            &request.repository,
            &branch,
            &request.path,
            Scope::Everything,
            &UsdInstanceOverrides::default(),
        )
        .await
        {
            Ok(gathered) => gathered,
            // That the files could not be read now says nothing of the stage.
            Err(StageError::Hub(error)) => {
                tracing::warn!(%error, "a stage could not be read to draw it");
                return Ok(StageThumbnail::default());
            }
            Err(error) => {
                tracing::warn!(%error, "a stage's picture could not be drawn");
                let _ = tokio::fs::write(self.failed_path(&key), b"").await;
                return Ok(StageThumbnail::default());
            }
        };
        if !gathered.missing.is_empty() {
            return Ok(StageThumbnail::default());
        }
        match self.draw(reader, request, gathered, &probe, &key).await {
            Ok(()) => Ok(self.kept(&key, request.size).await.unwrap_or_default()),
            Err(error) => {
                tracing::warn!(error = %format!("{error:#}"), "a stage's picture could not be drawn");
                // A later call tries again once this is old. A failure to note it only costs a retry.
                let _ = tokio::fs::write(self.failed_path(&key), b"").await;
                Ok(StageThumbnail::default())
            }
        }
    }

    /// One draw of a key at a time, so that a gallery's many asks for one stage draw it once.
    fn turn(&self, key: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut drawing = self.drawing.lock().unwrap_or_else(|held| held.into_inner());
        drawing.retain(|_, turn| turn.strong_count() > 0);
        if let Some(turn) = drawing.get(key).and_then(Weak::upgrade) {
            return turn;
        }
        let turn = Arc::new(tokio::sync::Mutex::new(()));
        drawing.insert(key.to_owned(), Arc::downgrade(&turn));
        turn
    }

    fn path(&self, key: &str, size: ThumbnailSize) -> PathBuf {
        let name = match size {
            ThumbnailSize::Small => "small",
            ThumbnailSize::Large => "large",
        };
        self.directory.join(format!("{key}.{name}.jpg"))
    }

    fn failed_path(&self, key: &str) -> PathBuf {
        self.directory.join(format!("{key}.draw-failed"))
    }

    /// The small kept picture after fresh caller authorization of its contributing files, drawing nothing.
    pub async fn ready(
        &self,
        reader: &Reader,
        repository: &crate::contract::RepositoryId,
        branch: &crate::contract::BranchName,
        path: &str,
    ) -> Option<StageThumbnail> {
        let probe = reader.probe(repository, branch, path).await.ok()?;
        let request = ThumbnailRequest {
            repository: repository.clone(),
            branch: branch.clone(),
            path: crate::contract::StagePath::parse(path).ok()?,
            size: ThumbnailSize::Small,
        };
        let key = key(&probe, path);
        if !self.reuse(reader, &request, &probe, &key).await {
            return None;
        }
        self.kept(&key, ThumbnailSize::Small).await
    }

    async fn kept(&self, key: &str, size: ThumbnailSize) -> Option<StageThumbnail> {
        let bytes = tokio::fs::read(self.path(key, size)).await.ok()?;
        Some(StageThumbnail {
            media_type: Some(MEDIA_TYPE.to_owned()),
            data: Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
        })
    }

    /// A new tip need not mean a new picture. What a stage read when it was drawn is noted beside
    /// its picture, and when each of those files is unchanged, and each it looked for is still not
    /// there, the picture stands for this tip too.
    async fn reuse(
        &self,
        reader: &Reader,
        request: &ThumbnailRequest,
        probe: &Probe,
        key: &str,
    ) -> bool {
        let Ok(noted) = tokio::fs::read(self.uses_path(probe, request.path.as_str())).await else {
            return false;
        };
        let Ok(uses) = serde_json::from_slice::<Uses>(&noted) else {
            return false;
        };
        let Ok(branch) = stage::frozen_branch(probe.revision.as_deref()) else {
            return false;
        };
        let repository = &request.repository;
        let branch = &branch;
        if !uses
            .readable_unchanged(|path| async move {
                address_now(reader, repository, branch, &path).await
            })
            .await
        {
            return false;
        }
        if uses.key == key {
            return true;
        }
        for size in [ThumbnailSize::Small, ThumbnailSize::Large] {
            if tokio::fs::copy(self.path(&uses.key, size), self.path(key, size))
                .await
                .is_err()
            {
                return false;
            }
        }
        true
    }

    /// What a stage's picture depends on, by the stage's content.
    fn uses_path(&self, probe: &Probe, path: &str) -> PathBuf {
        let mut hash = Sha256::new();
        hash.update(RECIPE);
        hash.update([0]);
        hash.update(&probe.address);
        hash.update([0]);
        hash.update(path);
        let name: String = hash
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        self.directory.join(format!("{name}.uses"))
    }

    async fn failed_lately(&self, key: &str) -> bool {
        let Ok(metadata) = tokio::fs::metadata(self.failed_path(key)).await else {
            return false;
        };
        metadata
            .modified()
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age < RETRY_AFTER)
    }

    async fn draw(
        &self,
        reader: &Reader,
        request: &ThumbnailRequest,
        gathered: stage::Gathered,
        probe: &Probe,
        key: &str,
    ) -> anyhow::Result<()> {
        let read: Vec<String> = gathered.files.keys().cloned().collect();
        let absent: Vec<String> = gathered.missing.iter().cloned().collect();
        let folder = tempfile::tempdir().context("making the picture's temporary folder")?;
        let line = format!("start {}\n", folder.path().join(FRAME).display());
        anyhow::ensure!(
            line.split_whitespace().count() == 2,
            "the temporary folder's path holds a space"
        );
        let drawn = work::draw_frame(
            folder.path(),
            gathered,
            &line,
            &[],
            DRAW_TIMEOUT,
            std::future::pending(),
        )
        .await?;
        let small = self.path(key, ThumbnailSize::Small);
        let large = self.path(key, ThumbnailSize::Large);
        let output = drawn.output;
        tokio::task::spawn_blocking(move || keep(&output, &small, &large))
            .await
            .map_err(|error| anyhow!("{error}"))??;
        // Failure to authorize/note any contributor prevents future cache hits.
        let branch = stage::frozen_branch(probe.revision.as_deref())?;
        let mut addresses = HashMap::new();
        for path in read.iter().chain(absent.iter()) {
            if let Some(address) = address_now(reader, &request.repository, &branch, path).await? {
                addresses.insert(path.clone(), address);
            }
        }
        let uses = Uses::of(key, read, absent, &addresses);
        let noted = serde_json::to_vec(&uses)?;
        tokio::fs::write(self.uses_path(probe, request.path.as_str()), noted).await?;

        Ok(())
    }
}

async fn address_now(
    reader: &Reader,
    repository: &crate::contract::RepositoryId,
    branch: &crate::contract::BranchName,
    path: &str,
) -> Result<Option<String>, HubError> {
    match reader.probe(repository, branch, path).await {
        Ok(probe) => Ok(Some(probe.address)),
        Err(error) if error.is_not_found() => Ok(None),
        Err(error) => Err(error),
    }
}

/// What a drawn picture depends on: every file its stage read, with the content it had, and every
/// file its stage looked for that was not there, as none.
#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Uses {
    key: String,
    files: BTreeMap<String, Option<String>>,
}

impl Uses {
    fn of(
        key: &str,
        read: Vec<String>,
        absent: Vec<String>,
        addresses: &HashMap<String, String>,
    ) -> Self {
        let files = read
            .into_iter()
            .chain(absent)
            .map(|path| {
                let address = addresses.get(&path).cloned();
                (path, address)
            })
            .collect();
        Self {
            key: key.to_owned(),
            files,
        }
    }

    async fn readable_unchanged<F, Fut, E>(&self, mut probe: F) -> bool
    where
        F: FnMut(String) -> Fut,
        Fut: Future<Output = Result<Option<String>, E>>,
    {
        for (path, expected) in &self.files {
            // Old partial-stage pictures are not complete environment previews.
            if expected.is_none() {
                return false;
            }
            match probe(path.clone()).await {
                Ok(current) if &current == expected => {}
                _ => return false,
            }
        }
        true
    }
}

/// The name a picture is kept under: from the file's content, and for a stage that is not one
/// self-contained package, from the branch's tip too, since a layer it names may have changed.
fn key(probe: &Probe, path: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(RECIPE);
    hash.update([0]);
    hash.update(&probe.address);
    if !path.to_ascii_lowercase().ends_with(".usdz") {
        hash.update([0]);
        hash.update(probe.revision.as_deref().unwrap_or_default());
    }
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Encodes the frame at `frame` into both pictures. Each appears whole, by a rename.
fn keep(frame: &Path, small: &Path, large: &Path) -> anyhow::Result<()> {
    let image = image::open(frame).context("reading the rendered frame")?;
    let reduced = image.thumbnail(SMALL_WIDTH, u32::MAX);
    write_jpeg(&reduced, small)?;
    write_jpeg(&image, large)
}

fn write_jpeg(image: &image::DynamicImage, path: &Path) -> anyhow::Result<()> {
    let partial = path.with_extension("partial");
    {
        let mut file = std::io::BufWriter::new(std::fs::File::create(&partial)?);
        image
            .to_rgb8()
            .write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(
                &mut file,
                JPEG_QUALITY,
            ))
            .context("encoding a picture")?;
    }
    std::fs::rename(&partial, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn kept_picture_requires_every_current_contributing_read() {
        // Synthetic content-address fixtures. No renderer or image validity claim.
        let addresses = HashMap::from([
            ("root.usda".to_owned(), "root-content".to_owned()),
            ("library/site.usda".to_owned(), "private-layer".to_owned()),
        ]);
        let uses = Uses::of(
            "cached-picture",
            addresses.keys().cloned().collect(),
            Vec::new(),
            &addresses,
        );
        assert!(
            uses.readable_unchanged(|path| std::future::ready(Ok::<_, ()>(
                addresses.get(&path).cloned()
            )))
            .await
        );
        // The root remains readable and the cached image remains present. Revoked
        // child authority still invalidates the cache hit, including a warm gallery.
        assert!(
            !uses
                .readable_unchanged(|path| std::future::ready(if path.starts_with("library/") {
                    Err(())
                } else {
                    Ok(addresses.get(&path).cloned())
                }))
                .await
        );
        assert!(
            !uses
                .readable_unchanged(|path| std::future::ready(Ok::<_, ()>(
                    if path.starts_with("library/") {
                        None
                    } else {
                        addresses.get(&path).cloned()
                    }
                )))
                .await
        );
        assert!(
            !uses
                .readable_unchanged(|path| std::future::ready(Ok::<_, ()>(Some(
                    if path.starts_with("library/") {
                        "changed".into()
                    } else {
                        "root-content".into()
                    }
                ))))
                .await
        );
        // A cache entry from an older partial composition cannot become a complete preview.
        let partial = Uses::of(
            "partial",
            vec!["root.usda".into()],
            vec!["library/missing.usda".into()],
            &addresses,
        );
        assert!(
            !partial
                .readable_unchanged(|path| std::future::ready(Ok::<_, ()>(
                    addresses.get(&path).cloned()
                )))
                .await
        );
    }

    fn probe(revision: &str) -> Probe {
        Probe {
            address: "abc".to_owned(),
            revision: Some(revision.to_owned()),
        }
    }

    #[test]
    fn a_package_is_kept_by_its_content_and_a_layered_stage_by_its_branch_too() {
        assert_eq!(key(&probe("r1"), "a/a.usdz"), key(&probe("r2"), "a/a.USDZ"));
        assert_ne!(key(&probe("r1"), "a/a.usd"), key(&probe("r2"), "a/a.usd"));
    }

    #[test]
    fn a_frame_becomes_two_pictures() {
        let folder = tempfile::tempdir().unwrap();
        let frame = folder.path().join("frame.png");
        image::RgbaImage::from_pixel(1280, 720, image::Rgba([10, 20, 30, 255]))
            .save(&frame)
            .unwrap();
        let small = folder.path().join("s.jpg");
        let large = folder.path().join("l.jpg");
        keep(&frame, &small, &large).unwrap();
        assert_eq!(image::open(&small).unwrap().width(), SMALL_WIDTH);
        assert_eq!(image::open(&large).unwrap().width(), 1280);
    }

    #[tokio::test]
    async fn a_failure_is_left_for_a_while() {
        let folder = tempfile::tempdir().unwrap();
        let thumbnails = Thumbnails::new(folder.path().to_owned()).unwrap();
        assert!(!thumbnails.failed_lately("k").await);
        std::fs::write(thumbnails.failed_path("k"), b"").unwrap();
        assert!(thumbnails.failed_lately("k").await);
    }

    #[tokio::test]
    async fn one_key_takes_one_turn_at_a_time() {
        let folder = tempfile::tempdir().unwrap();
        let thumbnails = Thumbnails::new(folder.path().to_owned()).unwrap();
        let first = thumbnails.turn("k");
        let second = thumbnails.turn("k");
        assert!(Arc::ptr_eq(&first, &second));
        assert!(!Arc::ptr_eq(&first, &thumbnails.turn("other")));
    }
}
