//! Reading a stage from Lore, through the Hub as one person: its scenes, the files it names
//! and what it holds. Nothing here touches a disk or outlives the call.
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use futures::{StreamExt as _, TryStreamExt as _, stream};
use usd_bevy::{UsdSource, instance::UsdInstanceOverrides};

use crate::{
    contract::{
        BranchName, MAX_LISTED, RepositoryId, STAGE_FORMAT, StagePath, StageReport, TypeCount,
        VariantSetReport,
    },
    hub::{HubError, Reader},
};

/// How many requests to the Hub one read keeps open.
const CONCURRENT_READS: usize = 8;
/// The extensions of a USD layer.
const LAYERS: [&str; 4] = ["usd", "usda", "usdc", "usdz"];
/// A guard on a stage whose references keep naming new files: real stages settle in a few.
const MAX_ROUNDS: usize = 64;

/// A file the stage asked for, and its bytes if the repository holds it.
type Fetched = (String, Option<Arc<[u8]>>);

/// Which of the files a stage names to fetch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Its layers, which say what it holds.
    Layers,
    /// Its layers and textures, which it takes to draw it.
    Everything,
}

impl Scope {
    fn admits(self, path: &str) -> bool {
        self == Self::Everything || is_layer(path)
    }
}

fn is_layer(path: &str) -> bool {
    path.rsplit_once('.').is_some_and(|(_, extension)| {
        LAYERS
            .iter()
            .any(|layer| extension.eq_ignore_ascii_case(layer))
    })
}

/// Why a stage could not be read.
#[derive(Debug, thiserror::Error)]
pub enum StageError {
    #[error(transparent)]
    Hub(#[from] HubError),
    #[error("the stage could not be composed: {0}")]
    Compose(String),
    #[error("the stage's dependencies did not settle in {MAX_ROUNDS} rounds")]
    Unsettled,
}

/// A stage's files, by their paths in the repository.
#[derive(Debug, Clone)]
pub struct Gathered {
    pub root: StagePath,
    pub files: BTreeMap<String, Arc<[u8]>>,
    /// Files the stage names that were not fetched: absent from the repository, outside it,
    /// or outside the scope.
    pub missing: BTreeSet<String>,
    /// The root file's content address.
    pub address: String,
    /// The branch's tip when the root was read.
    pub revision: Option<String>,
}

impl Gathered {
    pub fn bytes(&self) -> u64 {
        self.files.values().map(|bytes| bytes.len() as u64).sum()
    }

    fn source(&self) -> Result<UsdSource, StageError> {
        UsdSource::from_memory(
            self.root.as_str(),
            self.files
                .iter()
                .map(|(path, bytes)| (path.as_str(), bytes.clone())),
        )
        .map_err(|error| StageError::Compose(error.to_string()))
    }
}

/// Reads `path` and then what it names under `overrides`, composing again after each round until
/// nothing it may fetch is missing.
pub async fn gather(
    reader: &Reader,
    repository: &RepositoryId,
    branch: &BranchName,
    path: &StagePath,
    scope: Scope,
    overrides: &UsdInstanceOverrides,
) -> Result<Gathered, StageError> {
    let root = reader.file(repository, branch, path.as_str()).await?;
    let mut gathered = Gathered {
        root: path.clone(),
        files: BTreeMap::from([(path.as_str().to_owned(), Arc::from(&root.bytes[..]))]),
        missing: BTreeSet::new(),
        address: root.address,
        revision: root.revision,
    };
    for _ in 0..MAX_ROUNDS {
        let wanted = wanted(&mut gathered, scope, overrides).await?;
        if wanted.is_empty() {
            return Ok(gathered);
        }
        let fetched: Vec<Fetched> = stream::iter(wanted)
            .map(|file| async move {
                match reader.file(repository, branch, &file).await {
                    Ok(answer) => Ok((file, Some(Arc::from(&answer.bytes[..])))),
                    Err(error) if error.is_not_found() => Ok((file, None)),
                    Err(error) => Err(error),
                }
            })
            .buffer_unordered(CONCURRENT_READS)
            .try_collect()
            .await?;
        for (file, bytes) in fetched {
            match bytes {
                Some(bytes) => {
                    gathered.files.insert(file, bytes);
                }
                None => {
                    gathered.missing.insert(file);
                }
            }
        }
    }
    Err(StageError::Unsettled)
}

/// What to fetch next: what the composed stage asks for that is not held and may be fetched.
/// What may not be fetched goes in `missing`.
async fn wanted(
    gathered: &mut Gathered,
    scope: Scope,
    overrides: &UsdInstanceOverrides,
) -> Result<Vec<String>, StageError> {
    let source = gathered.source()?;
    let overrides = overrides.clone();
    let asked = tokio::task::spawn_blocking(move || source.missing_dependencies_under(&overrides))
        .await
        .map_err(|error| StageError::Compose(error.to_string()))?;
    let mut wanted = Vec::new();
    for file in asked {
        if gathered.missing.contains(&file) {
            continue;
        }
        if fetchable(&file) && scope.admits(&file) {
            wanted.push(file);
        } else {
            gathered.missing.insert(file);
        }
    }
    Ok(wanted)
}

/// Whether the files route can name `path`: relative, with no empty or dot segment.
fn fetchable(path: &str) -> bool {
    path.split('/')
        .all(|segment| !matches!(segment, "" | "." | ".."))
        && !path.contains(':')
}

/// What the gathered stage holds.
pub async fn report(
    gathered: Gathered,
    repository: RepositoryId,
    branch: BranchName,
) -> Result<StageReport, StageError> {
    tokio::task::spawn_blocking(move || read_report(&gathered, repository, branch))
        .await
        .map_err(|error| StageError::Compose(error.to_string()))?
}

fn listed(list: &mut Vec<String>, truncated: &mut bool, value: String) {
    if list.len() < MAX_LISTED {
        list.push(value);
    } else {
        *truncated = true;
    }
}

fn read_report(
    gathered: &Gathered,
    repository: RepositoryId,
    branch: BranchName,
) -> Result<StageReport, StageError> {
    use openusd::usd::{PrimPredicate, PrimStatus};
    let compose = |error: openusd::Error| StageError::Compose(error.to_string());
    let source = gathered.source()?;
    let stage = source.open_stage().map_err(compose)?;
    let predicate = || {
        PrimPredicate::new(
            PrimStatus::ACTIVE.union(PrimStatus::DEFINED),
            PrimStatus::ABSTRACT,
        )
    };
    let mut prims = 0_u64;
    let mut types = BTreeMap::<String, u64>::new();
    let mut cameras = Vec::new();
    let mut variant_sets = Vec::new();
    let mut truncated = false;
    let mut paths = Vec::new();
    stage
        .traverse(predicate(), |path| paths.push(path.clone()))
        .map_err(compose)?;
    for path in &paths {
        prims += 1;
        let prim = stage
            .prim(path.clone())
            .map_err(|error| StageError::Compose(format!("{path}: {error}")))?;
        let type_name = prim.type_name().ok().flatten();
        if let Some(type_name) = &type_name {
            *types.entry(type_name.to_string()).or_default() += 1;
            if type_name.as_str() == "Camera" {
                listed(&mut cameras, &mut truncated, path.to_string());
            }
        }
        for (set, selection) in
            usd_bevy::read::variants::variant_selections(&stage, path).unwrap_or_default()
        {
            if variant_sets.len() < MAX_LISTED {
                let options = usd_bevy::read::variants::variant_options(&stage, path, &set);
                variant_sets.push(VariantSetReport {
                    prim: path.to_string(),
                    set,
                    selection: Some(selection),
                    options,
                });
            } else {
                truncated = true;
            }
        }
    }
    // A stage opened with its payloads unloaded shows which prims have one.
    let mut payloads = Vec::new();
    if let Ok(unloaded) = source.open_stage_unloaded() {
        let mut held = Vec::new();
        let _ = unloaded.traverse(predicate(), |path| held.push(path.clone()));
        for path in held {
            if unloaded
                .prim(path.clone())
                .is_ok_and(|prim| !prim.is_loaded().unwrap_or(true))
            {
                listed(&mut payloads, &mut truncated, path.to_string());
            }
        }
    }
    let mut types: Vec<TypeCount> = types
        .into_iter()
        .map(|(type_name, count)| TypeCount { type_name, count })
        .collect();
    types.sort_by(|a, b| b.count.cmp(&a.count).then(a.type_name.cmp(&b.type_name)));
    let text = |field: &str| {
        stage
            .stage_metadata(field)
            .ok()
            .flatten()
            .and_then(|value| match value {
                openusd::vt::Value::String(text) => Some(text),
                openusd::vt::Value::Token(token) => Some(token.to_string()),
                _ => None,
            })
    };
    let meters_per_unit = stage
        .stage_metadata("metersPerUnit")
        .ok()
        .flatten()
        .and_then(|value| match value {
            openusd::vt::Value::Double(meters) => Some(meters),
            openusd::vt::Value::Float(meters) => Some(f64::from(meters)),
            _ => None,
        });
    let mut missing: Vec<String> = gathered.missing.iter().cloned().collect();
    missing.truncate(MAX_LISTED);
    Ok(StageReport {
        format: STAGE_FORMAT.to_owned(),
        repository,
        branch,
        path: gathered.root.clone(),
        address: gathered.address.clone(),
        layers: gathered.files.keys().cloned().collect(),
        bytes: gathered.bytes(),
        missing,
        up_axis: text("upAxis"),
        meters_per_unit,
        default_prim: stage.default_prim().map(|name| name.to_string()),
        start_time_code: stage.start_time_code(),
        end_time_code: stage.end_time_code(),
        time_codes_per_second: stage.time_codes_per_second(),
        prims,
        types,
        cameras,
        variant_sets,
        payloads,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOT: &str = r#"#usda 1.0
(
    defaultPrim = "World"
    upAxis = "Z"
    metersPerUnit = 0.5
    startTimeCode = 2
    endTimeCode = 8
    timeCodesPerSecond = 12
)
def Xform "World" (
    prepend references = @./part.usda@</Part>
    variantSets = "look"
    variants = { string look = "red" }
)
{
    variantSet "look" = {
        "red" { }
        "blue" { }
    }
    def Camera "Cam" { }
}
"#;
    const PART: &str = "#usda 1.0\ndef Xform \"Part\" { def Sphere \"Ball\" { } }\n";

    fn gathered(files: &[(&str, &str)]) -> Gathered {
        Gathered {
            root: StagePath::parse("a/a.usda").unwrap(),
            files: files
                .iter()
                .map(|(path, text)| ((*path).to_owned(), Arc::from(text.as_bytes())))
                .collect(),
            missing: BTreeSet::new(),
            address: "root-address".to_owned(),
            revision: Some("r1".to_owned()),
        }
    }

    #[test]
    fn scope_decides_by_extension() {
        assert!(Scope::Layers.admits("a/b.USDC"));
        assert!(!Scope::Layers.admits("a/b.png"));
        assert!(Scope::Everything.admits("a/b.png"));
    }

    #[test]
    fn only_a_path_in_the_repository_can_be_fetched() {
        assert!(fetchable("a/b.usd"));
        for path in ["/a/b.usd", "a/../b.usd", "./b.usd", "anon:x", "a//b"] {
            assert!(!fetchable(path), "{path}");
        }
    }

    #[tokio::test]
    async fn composing_names_what_the_stage_still_lacks() {
        let none = UsdInstanceOverrides::default();
        let mut held = gathered(&[("a/a.usda", ROOT)]);
        assert_eq!(
            wanted(&mut held, Scope::Layers, &none).await.unwrap(),
            ["a/part.usda"]
        );
        let mut held = gathered(&[("a/a.usda", ROOT), ("a/part.usda", PART)]);
        assert!(
            wanted(&mut held, Scope::Layers, &none)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn a_chosen_variant_names_what_only_it_refers_to() {
        const VARIANTS: &str = r#"#usda 1.0
def Xform "World" (
    variantSets = "look"
    variants = { string look = "plain" }
)
{
    variantSet "look" = {
        "plain" { }
        "fancy" {
            def Xform "Extra" (
                prepend references = @./extra.usda@</Part>
            )
            {
            }
        }
    }
}
"#;
        let none = UsdInstanceOverrides::default();
        let mut held = gathered(&[("a/a.usda", VARIANTS)]);
        assert!(
            wanted(&mut held, Scope::Layers, &none)
                .await
                .unwrap()
                .is_empty()
        );
        let chosen = UsdInstanceOverrides {
            variants: vec![("/World".into(), "look".into(), "fancy".into())],
            attributes: vec![],
        };
        assert_eq!(
            wanted(&mut held, Scope::Layers, &chosen).await.unwrap(),
            ["a/extra.usda"]
        );
    }

    #[tokio::test]
    async fn the_report_says_what_the_stage_holds() {
        let held = gathered(&[("a/a.usda", ROOT), ("a/part.usda", PART)]);
        let report = report(
            held,
            RepositoryId::parse("r").unwrap(),
            BranchName::parse("main").unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(report.format, STAGE_FORMAT);
        assert_eq!(report.up_axis.as_deref(), Some("Z"));
        assert_eq!(report.meters_per_unit, Some(0.5));
        assert_eq!(report.default_prim.as_deref(), Some("World"));
        assert_eq!((report.start_time_code, report.end_time_code), (2.0, 8.0));
        assert_eq!(report.time_codes_per_second, 12.0);
        assert_eq!(report.cameras, ["/World/Cam"]);
        assert_eq!(report.layers, ["a/a.usda", "a/part.usda"]);
        assert_eq!(report.prims, 3);
        assert_eq!(report.variant_sets.len(), 1);
        assert_eq!(report.variant_sets[0].selection.as_deref(), Some("red"));
        assert_eq!(report.variant_sets[0].options, ["blue", "red"]);
        assert!(report.payloads.is_empty());
        assert!(!report.truncated);
    }
}
