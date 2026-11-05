//! Items: the arguments of `create_item`, an item as reads return it, the product
//! `create_item` returns and a page of the item listing.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{ItemId, ItemName, UsdResource};

/// The format of a created item.
pub const ITEM_FORMAT: &str = "https://veoveo.ai/formats/usd-item";

/// The arguments of `create_item`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[veoveo_hub_sdk::tool(provider = super::UsdProvider, output = veoveo_hub_sdk::definition::Product<CreatedItem>, name = "create_item")]
pub struct CreateItemRequest {
    /// The new item's name.
    pub name: ItemName,
}

/// One item of the caller's tenant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[derive(veoveo_hub_sdk::SingleOutput)]
pub struct ItemView {
    pub item_id: ItemId,
    /// `usd://item/{item_id}`, where the item stays readable.
    pub item_uri: UsdResource,
    pub name: ItemName,
}

/// What `create_item` returns: the new item and its address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[derive(veoveo_hub_sdk::SingleOutput)]
pub struct CreatedItem {
    /// `https://veoveo.ai/formats/usd-item`.
    pub format: String,
    pub item: ItemView,
}
