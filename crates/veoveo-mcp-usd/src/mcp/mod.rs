//! One typed route table binds requests, outputs, discovery and current permissions.
mod declarations;
mod requirements;
mod resources;
mod setup;

use crate::{
    contract::{CreateItemRequest, CreatedItem, ITEM_FORMAT, ItemView, UsdPermission},
    store::{ItemStore, ItemsError, stored_tenant},
};
use rmcp::{ErrorData as McpError, RoleServer, service::RequestContext};
use std::sync::LazyLock;
use veoveo_hub_sdk::{
    HubInternalIdentity,
    definition::Product,
    hosting::{DomainServer, RoutePolicy, Routes, hub_identity},
    server_contract::McpServerSetup,
};

pub use declarations::{REGISTRATION, registration};
pub use requirements::AUTHORIZATION;
use setup::SERVER_SETUP;
pub use setup::UsdContract;

pub fn force_setup() {
    LazyLock::force(&SERVER_SETUP);
}

#[derive(Clone)]
pub struct UsdMcp {
    items: ItemStore,
}
impl UsdMcp {
    pub fn new(items: ItemStore) -> Self {
        Self { items }
    }
    async fn create_item(
        &self,
        request: CreateItemRequest,
        context: RequestContext<RoleServer>,
    ) -> Result<Product<CreatedItem>, McpError> {
        let identity = hub_identity(&context)?;
        let item = self
            .items
            .create(tenant(&identity)?, &request.name)
            .await
            .map_err(items_error)?;
        let view = ItemView::from(item);
        let address = view.item_uri.clone();
        Product::new(
            &address,
            CreatedItem {
                format: ITEM_FORMAT.to_owned(),
                item: view,
            },
        )
        .map_err(|_| McpError::internal_error("item product construction failed", None))
    }
}

static ROUTES: LazyLock<Routes<UsdMcp>> = LazyLock::new(|| {
    let routes = Routes::<UsdMcp>::builder(RoutePolicy::permission(UsdPermission::ItemsRead).expect("Usd permission"))
        .tool::<CreateItemRequest, _, _>(
            "Create an item in your tenant. The product's `resultUri` is its canonical address; list your items at `usd://items`.",
            RoutePolicy::permission(UsdPermission::ItemsWrite).expect("Usd permission"),
            |domain, request, context| async move { domain.create_item(request, context).await.map_err(Into::into) },
        );
    resources::register(routes).build()
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

fn tenant(identity: &HubInternalIdentity) -> Result<veoveo_store::TenantId, McpError> {
    stored_tenant(&identity.authority.tenant).map_err(|_| {
        McpError::invalid_request("the forwarded identity names no stored tenant", None)
    })
}
fn items_error(error: ItemsError) -> McpError {
    match error {
        ItemsError::Paging(
            veoveo_types::PagingError::Cursor(_) | veoveo_types::PagingError::Limit,
        ) => McpError::invalid_params(error.to_string(), None),
        failure => {
            tracing::error!(%failure, "a usd request failed");
            McpError::internal_error("the server could not complete the request", None)
        }
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn the_setup_and_documents_admit() {
        std::sync::LazyLock::force(&super::SERVER_SETUP);
        assert!(super::setup::SERVER_DOCS.agent_manual().is_some());
    }
}
