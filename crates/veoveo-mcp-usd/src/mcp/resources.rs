//! Tenant admission is current on every exact read and collection page.
use super::{UsdMcp, items_error, tenant};
use crate::contract::{ItemView, Items, ReadItem, ReadItems, UsdPermission};
use rmcp::{ErrorData as McpError, RoleServer, service::RequestContext};
use veoveo_hub_sdk::{
    definition::ReadCollection,
    hosting::{ReadCache, ResourceBuilder, RoutePolicy, RoutesBuilder, hub_identity},
};
use veoveo_types::Page;

pub(super) fn register(routes: RoutesBuilder<UsdMcp>) -> RoutesBuilder<UsdMcp> {
    let policy = || RoutePolicy::permission(UsdPermission::ItemsRead).expect("Usd permission");
    routes
        .template::<ReadItem, _, _>(
            ResourceBuilder::new("Item")
                .reader(read_item)
                .policy(policy())
                .cache(ReadCache::NoStore),
        )
        .collection_resource::<ReadItems, _, _>(
            Some(ReadItems),
            ResourceBuilder::new("Your items")
                .reader(read_items)
                .policy(policy())
                .cache(ReadCache::NoStore),
        )
}
async fn read_item(
    domain: UsdMcp,
    read: ReadItem,
    context: RequestContext<RoleServer>,
) -> Result<ItemView, McpError> {
    let identity = hub_identity(&context)?;
    let item = domain
        .items
        .item(tenant(&identity)?, read.item)
        .await
        .map_err(items_error)?
        .ok_or_else(|| McpError::invalid_params("no such item", None))?;
    Ok(item.into())
}
async fn read_items(
    domain: UsdMcp,
    read: ReadCollection<ReadItems>,
    context: RequestContext<RoleServer>,
) -> Result<Page<Items>, McpError> {
    let identity = hub_identity(&context)?;
    domain
        .items
        .page(tenant(&identity)?, read.selection())
        .await
        .map_err(items_error)
}
