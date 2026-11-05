//! The server's public types. Consumers select `default-features = false,
//! features = ["contract"]`, which builds without a runtime dependency.

mod calls;
pub use calls::UsdProvider;
mod ids;
mod permissions;
mod resource;
mod stages;

pub use ids::{BranchName, PrimPath, RepositoryId, StagePath, UsdIdError};
pub use permissions::{UsdPermission, UsdScope};
pub use resource::{UsdDocument, UsdResource, UsdResourceError};
pub use stages::{
    DEFAULT_BRANCH, InspectStageRequest, ListScenesRequest, MAX_LISTED, RENDERED_FORMAT,
    RenderCamera, RenderStageRequest, RenderedStage, RepositorySummary, STAGE_FORMAT, Scene,
    SceneListing, StageReport, StageThumbnail, ThumbnailRequest, ThumbnailSize, TypeCount,
    VariantChoice, VariantSetReport,
};
