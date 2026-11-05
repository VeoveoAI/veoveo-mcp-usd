//! The tools: finding scenes, reporting what a stage holds and rendering it.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use veoveo_hub_sdk::definition::Product;

use super::{BranchName, PrimPath, RepositoryId, StagePath, UsdProvider};

/// The format of a stage's report.
pub const STAGE_FORMAT: &str = "https://veoveo.ai/formats/usd-stage";
/// The format of a finished render.
pub const RENDERED_FORMAT: &str = "https://veoveo.ai/formats/usd-render";
/// The branch `list_scenes` reads when none is named.
pub const DEFAULT_BRANCH: &str = "main";
/// The most entries one list of a report holds.
pub const MAX_LISTED: usize = 500;

/// The arguments of `list_scenes`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[veoveo_hub_sdk::tool(provider = UsdProvider, output = SceneListing, name = "list_scenes")]
pub struct ListScenesRequest {
    /// The repository whose scenes to list. Without one, lists the repositories you can read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<RepositoryId>,
    /// The branch to read; `main` by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<BranchName>,
}

/// A repository the caller can read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepositorySummary {
    pub id: RepositoryId,
    pub name: String,
}

/// A folder at a repository's top level whose root stage bears its name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Scene {
    /// The folder's name.
    pub name: String,
    /// The root stage.
    pub path: StagePath,
    /// The bytes of the folder, as the Hub lists them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

/// What `list_scenes` answers: the repositories, or the scenes of one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[derive(veoveo_hub_sdk::SingleOutput)]
pub struct SceneListing {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repositories: Vec<RepositorySummary>,
    /// The repository `scenes` belong to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<RepositoryId>,
    /// The branch `scenes` were read from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<BranchName>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scenes: Vec<Scene>,
}

/// How large a thumbnail to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ThumbnailSize {
    /// For a gallery's card: 480 pixels wide.
    #[default]
    Small,
    /// For a closer look: 1280 pixels wide.
    Large,
}

/// The arguments of `thumbnail`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[veoveo_hub_sdk::tool(provider = UsdProvider, output = StageThumbnail, name = "thumbnail")]
pub struct ThumbnailRequest {
    pub repository: RepositoryId,
    pub branch: BranchName,
    /// The root stage.
    pub path: StagePath,
    #[serde(default)]
    pub size: ThumbnailSize,
}

/// A stage's picture, or none when it could not be drawn.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[derive(veoveo_hub_sdk::SingleOutput)]
pub struct StageThumbnail {
    /// The picture's media type.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    /// The picture, in base64.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
}

/// The arguments of `inspect_stage`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[veoveo_hub_sdk::tool(provider = UsdProvider, output = StageReport, name = "inspect_stage")]
pub struct InspectStageRequest {
    pub repository: RepositoryId,
    pub branch: BranchName,
    /// The root stage, relative to the repository's top.
    pub path: StagePath,
}

/// How many prims of a schema type a stage holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TypeCount {
    pub type_name: String,
    pub count: u64,
}

/// A variant set on a prim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VariantSetReport {
    pub prim: String,
    pub set: String,
    /// The effective selection, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<String>,
    pub options: Vec<String>,
}

/// What a stage holds, read as the caller.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[derive(veoveo_hub_sdk::SingleOutput)]
pub struct StageReport {
    /// `https://veoveo.ai/formats/usd-stage`.
    pub format: String,
    pub repository: RepositoryId,
    pub branch: BranchName,
    pub path: StagePath,
    /// The root file's content address in Lore.
    pub address: String,
    /// The files read, by their paths in the repository.
    pub layers: Vec<String>,
    /// The bytes read.
    pub bytes: u64,
    /// Files the stage asks for that the repository does not hold, and files it names that
    /// this report did not need, such as textures.
    pub missing: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub up_axis: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meters_per_unit: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_prim: Option<String>,
    pub start_time_code: f64,
    pub end_time_code: f64,
    pub time_codes_per_second: f64,
    /// How many prims the composed stage holds.
    pub prims: u64,
    /// Prims by schema type, the most numerous first.
    pub types: Vec<TypeCount>,
    pub cameras: Vec<String>,
    pub variant_sets: Vec<VariantSetReport>,
    /// Prims with a payload.
    pub payloads: Vec<String>,
    /// Whether a list stopped at its limit.
    pub truncated: bool,
}

/// Where a render looks from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged, deny_unknown_fields)]
pub enum RenderCamera {
    /// Through one of the stage's cameras.
    Prim { prim: PrimPath },
    /// From an eye at a focus, in world space.
    Look { eye: [f32; 3], focus: [f32; 3] },
}

/// One variant selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VariantChoice {
    pub prim: PrimPath,
    pub set: String,
    pub selection: String,
}

/// The arguments of `render_stage`, which runs as a Task.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[veoveo_hub_sdk::tool(provider = UsdProvider, output = Product<RenderedStage>, name = "render_stage", task = true)]
pub struct RenderStageRequest {
    pub repository: RepositoryId,
    pub branch: BranchName,
    /// The root stage, relative to the repository's top.
    pub path: StagePath,
    /// The time code to draw; the stage's start by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<f64>,
    /// Where to look from; by default the whole stage is framed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<RenderCamera>,
    /// Variant selections to apply first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub variants: Vec<VariantChoice>,
}

/// What a render stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[derive(veoveo_hub_sdk::SingleOutput)]
pub struct RenderedStage {
    /// `https://veoveo.ai/formats/usd-render`.
    pub format: String,
    pub repository: RepositoryId,
    pub branch: BranchName,
    pub path: StagePath,
    /// The branch's tip when the root was read, when the Hub names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// The root file's content address in Lore.
    pub address: String,
    /// The PNG's size in bytes.
    pub size: u64,
    /// The GPU that drew it.
    pub adapter: String,
    /// How long the child took.
    pub render_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_render_camera_is_a_prim_or_a_look() {
        let prim: RenderCamera = serde_json::from_str(r#"{"prim":"/World/Cam"}"#).unwrap();
        assert_eq!(
            prim,
            RenderCamera::Prim {
                prim: PrimPath::parse("/World/Cam").unwrap()
            }
        );
        let look: RenderCamera =
            serde_json::from_str(r#"{"eye":[1,2,3],"focus":[4,5,6]}"#).unwrap();
        assert_eq!(
            look,
            RenderCamera::Look {
                eye: [1.0, 2.0, 3.0],
                focus: [4.0, 5.0, 6.0]
            }
        );
        assert!(serde_json::from_str::<RenderCamera>(r#"{"prim":"World"}"#).is_err());
    }

    #[test]
    fn a_render_request_needs_a_repository_branch_and_path() {
        let request: RenderStageRequest = serde_json::from_str(
            r#"{"repository":"r","branch":"main","path":"a/a.usda","time":3}"#,
        )
        .unwrap();
        assert_eq!(request.time, Some(3.0));
        assert!(serde_json::from_str::<RenderStageRequest>(r#"{"repository":"r"}"#).is_err());
    }
}
