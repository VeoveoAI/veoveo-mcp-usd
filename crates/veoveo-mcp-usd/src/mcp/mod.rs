//! One typed route table binds requests, outputs, discovery and current permissions.
mod declarations;
mod requirements;
mod setup;
mod tasks;

use rmcp::{ErrorData as McpError, RoleServer, model::ClientCapabilities, service::RequestContext};
use std::sync::LazyLock;
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
        RenderedStage, RepositorySummary, SceneListing, StageReport, UsdPermission,
    },
    hub::{Hub, HubError, Reader},
    stage::{self, Scope, StageError},
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

pub fn force_setup() {
    LazyLock::force(&SERVER_SETUP);
}

#[derive(Clone)]
pub struct UsdMcp {
    hub: Hub,
    app: AppHtml,
}

impl UsdMcp {
    pub fn new(hub: Hub, app: AppHtml) -> Self {
        Self { hub, app }
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
        let Some(repository) = request.repository else {
            let mut repositories = reader
                .repositories()
                .await
                .map_err(|error| refused("listing your repositories", error))?;
            repositories.sort_by_key(|repository| repository.name.to_lowercase());
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
            });
        };
        let branch = match request.branch {
            Some(branch) => branch,
            None => BranchName::parse(DEFAULT_BRANCH).expect("the default branch's name"),
        };
        let scenes = stage::scenes(&reader, &repository, &branch)
            .await
            .map_err(|error| refused("listing the repository's scenes", error))?;
        Ok(SceneListing {
            repositories: vec![],
            repository: Some(repository),
            branch: Some(branch),
            scenes,
        })
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
            "List the Lore repositories you can read or, given a `repository`, its scenes on a branch: its top-level folders whose root stage bears their name.",
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
        .app(
            AppBuilder::new(app(), APP_NAME)
                .title("USD scenes")
                .description("The USD scenes you can read. Choosing one opens it in the viewer.")
                .policy(policy(UsdPermission::StagesRead))
                .reader(|domain: UsdMcp, _| async move { Ok(domain.app.clone()) }),
        )
        .app_tools(&app(), &[ListScenesRequest::TOOL], &[UiVisibility::Model, UiVisibility::App])
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
