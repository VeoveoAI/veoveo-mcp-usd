//! Typed calls and resource associations, available without the server runtime.
use super::{ItemId, ItemView, UsdResource};
use veoveo_hub_sdk::calls::ProviderContract;
use veoveo_types::{Collection, LiveKeyset, PageSize, ServerSlug};

pub struct UsdProvider;
impl ProviderContract for UsdProvider {
    fn slug() -> ServerSlug {
        crate::server()
    }
}

/// Tenant items in ascending UUIDv7 creation order, with unique identity tiebreaker.
pub struct Items;
impl Collection for Items {
    type Item = ItemView;
    type Filters = veoveo_hub_sdk::definition::NoFilters;
    type Key = ItemId;
    type Limit = PageSize<100>;
    type Policy = LiveKeyset;
    const NAME: &'static str = "usd.items";
    fn key(item: &ItemView) -> ItemId {
        item.item_id
    }
}

#[derive(Debug, Clone)]
#[veoveo_hub_sdk::collection_resource(provider = UsdProvider, address_type = UsdResource,
    collection = Items, template = "usd://items", address = UsdResource::Items,
    filters = veoveo_hub_sdk::definition::NoFilters {},
    parse = |address| matches!(address, UsdResource::Items).then_some(Self))]
pub struct ReadItems;

#[derive(Debug, Clone)]
#[veoveo_hub_sdk::resource_read(provider = UsdProvider, address_type = UsdResource,
    content = ItemView, template = "usd://item/{item_id}", address = UsdResource::Item(self.item),
    parse = |address| match address {UsdResource::Item(item) => Some(Self {item:*item}), _=>None})]
pub struct ReadItem {
    pub item: ItemId,
}
