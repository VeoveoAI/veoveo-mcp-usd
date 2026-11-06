//! One typed route table binds requests, outputs, discovery and current permissions.
mod declarations;
mod requirements;
mod setup;
mod tasks;

use futures::StreamExt as _;
use rmcp::{ErrorData as McpError, RoleServer, model::ClientCapabilities, service::RequestContext};
use std::sync::{Arc, LazyLock};
use usd_bevy::instance::UsdInstanceOverrides;
use veoveo_hub_sdk::{
    apps::{AppHtml, AppUri, UiVisibility},
    calls::ToolCall,
    definition::Product,
    hosting::{
        AppBuilder, ClientContext, DomainServer, RoutePolicy, Routes, ToolError, hub_identity,
    },
    server_contract::McpServerSetup,
};
use veoveo_mcp_artifact::contract::ArtifactPermission;

use crate::{
    contract::{
        BranchName, DEFAULT_BRANCH, InspectStageRequest, ListScenesRequest, RenderStageRequest,
        RenderedStage, RepositorySummary, Scene, SceneListing, StagePath, StageReport,
        StageThumbnail, ThumbnailRequest, UsdPermission,
    },
    hub::{Hub, HubError, Reader},
    stage::{self, Scope, StageError},
    thumbnail::Thumbnails,
};

pub use declarations::{REGISTRATION, registration};
pub use requirements::AUTHORIZATION;
use setup::SERVER_SETUP;
pub use setup::UsdContract;
pub use tasks::UsdTasks;

/// The App's address.
pub const APP_URI: &str = "ui://usd/scenes";
/// The App's programmatic resource name.
const APP_NAME: &str = "scenes";
/// The extensions the viewer takes.
pub const VIEWER_EXTENSIONS: [&str; 4] = ["usd", "usda", "usdc", "usdz"];
/// The viewer's public route under the server's mount.
pub const VIEWER_ROUTE: &str = "viewer/";
/// The viewer holds only published files, so the route needs no body and a page's few requests
/// a minute from every open browser fit.
const VIEWER_REQUESTS_PER_MINUTE: u32 = 6000;

/// The App's title.
const APP_TITLE: &str = "USD scenes";
/// What the App adds to the gallery.
const APP_ADAPTER: &str = include_str!("../../assets/scenes.js");

/// The App's document.
pub fn app_document() -> std::io::Result<AppHtml> {
    AppHtml::load(&veoveo_hub_ui::gallery_file(APP_TITLE, APP_ADAPTER)?)
}

pub fn force_setup() {
    LazyLock::force(&SERVER_SETUP);
}

#[derive(Clone)]
pub struct UsdMcp {
    hub: Hub,
    app: AppHtml,
    thumbnails: Thumbnails,
    scenes: Scenes,
}

use crate::scenes::{RepositoryScenes, Scenes};

/// How many scenes a page holds unless asked.
const PAGE: u32 = 60;
/// The most scenes one page holds.
const MOST_ON_A_PAGE: u32 = 120;
/// How many repositories are read at once.
const READS_AT_ONCE: usize = 8;
/// The most base64 one page answers. A page's other pictures are asked for afterwards.
const MOST_PAGE_BYTES: usize = 3 * 1024 * 1024;

impl UsdMcp {
    pub fn new(hub: Hub, app: AppHtml, thumbnails: Thumbnails) -> Self {
        Self {
            hub,
            app,
            thumbnails,
            scenes: Scenes::default(),
        }
    }

    fn reader(&self, context: &RequestContext<RoleServer>) -> Result<Reader, McpError> {
        let identity = hub_identity(context)?;
        Ok(Reader {
            hub: self.hub.clone(),
            client: context.client()?,
            profile: identity.profile,
        })
    }

    async fn list_scenes(
        &self,
        request: ListScenesRequest,
        context: RequestContext<RoleServer>,
    ) -> Result<SceneListing, ToolError> {
        let reader = self.reader(&context)?;
        let branch = match request.branch {
            Some(branch) => branch,
            None => BranchName::parse(DEFAULT_BRANCH).expect("the default branch's name"),
        };
        let limit = request.limit.unwrap_or(PAGE).clamp(1, MOST_ON_A_PAGE) as usize;
        let offset = match request.cursor.as_deref() {
            None => 0,
            Some(cursor) => cursor
                .parse()
                .map_err(|_| ToolError::Rejected("not a page of this listing".to_owned()))?,
        };
        let wanted = request
            .search
            .as_deref()
            .map(str::trim)
            .filter(|search| !search.is_empty())
            .map(str::to_lowercase);
        let holds = |name: &str| {
            wanted
                .as_ref()
                .is_none_or(|wanted| name.to_lowercase().contains(wanted))
        };
        let (repository, gathered) = match request.repository {
            Some(repository) => {
                let scenes = self
                    .scenes
                    .at_tip(&reader, &repository, &branch)
                    .await
                    .map_err(|error| refused("listing the repository's scenes", error))?;
                (Some(repository), vec![(None, scenes)])
            }
            None => {
                let mut repositories = reader
                    .repositories()
                    .await
                    .map_err(|error| refused("listing your repositories", error))?;
                repositories.sort_by_key(|repository| repository.name.to_lowercase());
                if !request.all {
                    return Ok(SceneListing {
                        repositories: repositories
                            .into_iter()
                            .map(|repository| RepositorySummary {
                                id: repository.id,
                                name: repository.name,
                            })
                            .collect(),
                        repository: None,
                        branch: None,
                        scenes: vec![],
                        total: None,
                        next: None,
                    });
                }
                (None, self.all_scenes(&reader, &branch, repositories).await)
            }
        };
        let matching: Vec<_> = gathered
            .iter()
            .flat_map(|(owner, scenes)| {
                scenes
                    .scenes
                    .iter()
                    .filter(|scene| holds(&scene.name))
                    .map(move |scene| (owner, scenes, scene))
            })
            .collect();
        let total = matching.len();
        let page: Vec<_> = matching.into_iter().skip(offset).take(limit).collect();
        let mut left = MOST_PAGE_BYTES;
        let mut scenes = Vec::with_capacity(page.len());
        for (owner, held, scene) in page {
            let Ok(path) = StagePath::parse(scene.path.as_str()) else {
                continue;
            };
            let picture = self
                .thumbnails
                .ready(&scene.address, held.revision.as_deref(), &scene.path)
                .await
                .filter(|picture| {
                    let bytes = picture.data.as_ref().map_or(0, String::len);
                    left.checked_sub(bytes)
                        .inspect(|rest| left = *rest)
                        .is_some()
                });
            scenes.push(Scene {
                name: scene.name.clone(),
                path,
                size: scene.size,
                repository: owner.clone(),
                picture,
            });
        }
        let end = offset.saturating_add(limit);
        Ok(SceneListing {
            repositories: vec![],
            repository,
            branch: Some(branch),
            scenes,
            total: Some(total as u64),
            next: (end < total).then(|| end.to_string()),
        })
    }

    /// Every repository's scenes: what was last known of one, which a person with a grant on it may
    /// see, and the rest read now. What was last known is looked at again after the answer.
    async fn all_scenes(
        &self,
        reader: &Reader,
        branch: &BranchName,
        repositories: Vec<crate::hub::Repository>,
    ) -> Vec<(Option<RepositorySummary>, Arc<RepositoryScenes>)> {
        let mut again = Vec::new();
        let found: Vec<_> = futures::stream::iter(repositories)
            .map(|repository| {
                let known = self.scenes.latest(&repository.id, branch);
                if let Some((_, true)) = &known {
                    again.push(repository.id.clone());
                }
                async move {
                    let scenes = match known {
                        Some((scenes, _)) => Some(scenes),
                        // A repository that cannot be read holds nothing to show here.
                        None => self
                            .scenes
                            .at_tip(reader, &repository.id, branch)
                            .await
                            .ok(),
                    };
                    let owner = RepositorySummary {
                        id: repository.id,
                        name: repository.name,
                    };
                    scenes.map(|scenes| (Some(owner), scenes))
                }
            })
            .buffered(READS_AT_ONCE)
            .collect()
            .await;
        if !again.is_empty() {
            let (scenes, reader, branch) = (self.scenes.clone(), reader.clone(), branch.clone());
            tokio::spawn(async move {
                futures::stream::iter(again)
                    .for_each_concurrent(READS_AT_ONCE, |repository| {
                        let (scenes, reader, branch) =
                            (scenes.clone(), reader.clone(), branch.clone());
                        async move {
                            let _ = scenes.at_tip(&reader, &repository, &branch).await;
                        }
                    })
                    .await;
            });
        }
        found.into_iter().flatten().collect()
    }

    async fn inspect_stage(
        &self,
        request: InspectStageRequest,
        context: RequestContext<RoleServer>,
    ) -> Result<StageReport, ToolError> {
        let reader = self.reader(&context)?;
        let gathered = stage::gather(
            &reader,
            &request.repository,
            &request.branch,
            &request.path,
            Scope::Layers,
            &UsdInstanceOverrides::default(),
        )
        .await
        .map_err(stage_refused)?;
        stage::report(gathered, request.repository, request.branch)
            .await
            .map_err(stage_refused)
    }

    async fn thumbnail(
        &self,
        request: ThumbnailRequest,
        context: RequestContext<RoleServer>,
    ) -> Result<StageThumbnail, ToolError> {
        let reader = self.reader(&context)?;
        self.thumbnails
            .thumbnail(&reader, &request)
            .await
            .map_err(|error| refused("reading the stage", error))
    }

    async fn render_stage(
        &self,
        _: RenderStageRequest,
        _: RequestContext<RoleServer>,
    ) -> Result<Product<RenderedStage>, McpError> {
        Err(McpError::missing_required_client_capability(
            ClientCapabilities::builder().enable_tasks().build(),
        ))
    }
}

fn refused(what: &str, error: HubError) -> ToolError {
    ToolError::Rejected(format!("{what}: {error}"))
}

fn stage_refused(error: StageError) -> ToolError {
    ToolError::Rejected(format!("reading the stage: {error}"))
}

fn policy(permission: UsdPermission) -> RoutePolicy {
    RoutePolicy::permission(permission).expect("the server's permission")
}

fn app() -> AppUri {
    AppUri::parse(APP_URI).expect("the App's address")
}

static ROUTES: LazyLock<Routes<UsdMcp>> = LazyLock::new(|| {
    Routes::<UsdMcp>::builder(policy(UsdPermission::StagesRead))
        .uses(ArtifactPermission::ArtifactsRead.name())
        .uses(ArtifactPermission::ArtifactsWrite.name())
        .calls_hub()
        .public_route(VIEWER_ROUTE, 1, VIEWER_REQUESTS_PER_MINUTE)
        .viewer(&VIEWER_EXTENSIONS, VIEWER_ROUTE)
        .tool::<ListScenesRequest, _, _>(
            "List the Lore repositories you can read or, given a `repository`, its scenes on a branch, at any depth: each folder with exactly one stage in it or a stage named for it, and any other stage. `all` lists every repository's scenes. Pages of at most 120, with `search` on their names.",
            policy(UsdPermission::StagesRead),
            |domain, request, context| async move { domain.list_scenes(request, context).await },
        )
        .tool::<InspectStageRequest, _, _>(
            "Report what a USD stage in a Lore repository holds: its layers, up axis, units, time range, prims by type, cameras, variant sets, payloads and missing files.",
            policy(UsdPermission::StagesRead),
            |domain, request, context| async move { domain.inspect_stage(request, context).await },
        )
        .task::<RenderStageRequest, _, _>(
            "Render one frame of a USD stage in a Lore repository on the GPU, as a Task, and store it as a PNG in Artifact, with metadata naming the repository, revision and path. The product's `resultUri` is the PNG.",
            policy(UsdPermission::StagesRender),
            |domain, request, context| async move { domain.render_stage(request, context).await.map_err(Into::into) },
        )
        .tool::<ThumbnailRequest, _, _>(
            "Draw a picture of a USD stage in a Lore repository, or say there is none, for a gallery. The first call for a stage draws it on the GPU; later ones answer from what the stage's content drew.",
            policy(UsdPermission::StagesRead),
            |domain, request, context| async move { domain.thumbnail(request, context).await },
        )
        .app(
            AppBuilder::new(app(), APP_NAME)
                .title(APP_TITLE)
                .description("The USD scenes you can read, as a gallery. Choosing one shows a larger picture and opens it in the viewer.")
                .policy(policy(UsdPermission::StagesRead))
                .reader(|domain: UsdMcp, _| async move { Ok(domain.app.clone()) }),
        )
        .app_tools(&app(), &[ListScenesRequest::TOOL], &[UiVisibility::Model, UiVisibility::App])
        .app_tools(&app(), &[ThumbnailRequest::TOOL], &[UiVisibility::App])
        .build()
});

impl DomainServer for UsdMcp {
    type Contract = UsdContract;
    fn setup() -> &'static McpServerSetup<UsdContract> {
        &SERVER_SETUP
    }
    fn routes() -> &'static Routes<Self> {
        &ROUTES
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_setup_and_documents_admit() {
        std::sync::LazyLock::force(&super::SERVER_SETUP);
        assert!(super::setup::SERVER_DOCS.agent_manual().is_some());
    }

    #[test]
    fn the_routes_declare_the_hub_artifact_and_the_viewer() {
        let authorization = super::ROUTES.authorization();
        assert!(authorization.calls_hub);
        let surfaces = super::ROUTES.surfaces().expect("the viewer's surfaces");
        let viewer = surfaces.viewer.expect("the viewer");
        assert_eq!(viewer.route.as_str(), "/usd/viewer/");
        assert_eq!(viewer.extensions.len(), 4);
    }
}
