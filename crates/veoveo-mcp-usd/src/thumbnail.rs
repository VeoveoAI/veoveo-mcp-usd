//! A stage's picture for a gallery: one frame drawn on the GPU as the caller, kept by the content
//! it was drawn from. Every call first reads a byte of the stage as the caller, so Lore decides
//! each one and a person without a grant learns nothing from what is kept. What is kept is only
//! a small picture of a stage, under a name its content decides, and any of it can be drawn again.
use std::{
    collections::HashMap,
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
    stage::{self, Scope},
    work::{self, FRAME},
};

/// Changes when what is drawn changes, so that older pictures are drawn again.
const RECIPE: &str = "2";
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
        if let Some(found) = self.kept(&key, request.size).await {
            return Ok(found);
        }
        if self.failed_lately(&key).await {
            return Ok(StageThumbnail::default());
        }
        let turn = self.turn(&key);
        let _turn = turn.lock().await;
        if let Some(found) = self.kept(&key, request.size).await {
            return Ok(found);
        }
        let Ok(_permit) = self.permits.acquire().await else {
            return Ok(StageThumbnail::default());
        };
        match self.draw(reader, request, &key).await {
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
        self.directory.join(format!("{key}.failed"))
    }

    async fn kept(&self, key: &str, size: ThumbnailSize) -> Option<StageThumbnail> {
        let bytes = tokio::fs::read(self.path(key, size)).await.ok()?;
        Some(StageThumbnail {
            media_type: Some(MEDIA_TYPE.to_owned()),
            data: Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
        })
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
        key: &str,
    ) -> anyhow::Result<()> {
        let gathered = stage::gather(
            reader,
            &request.repository,
            &request.branch,
            &request.path,
            Scope::Everything,
            &UsdInstanceOverrides::default(),
        )
        .await
        .context("reading the stage")?;
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
            .map_err(|error| anyhow!("{error}"))?
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
