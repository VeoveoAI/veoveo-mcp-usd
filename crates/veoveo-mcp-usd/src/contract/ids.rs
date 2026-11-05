//! The server's identities and checked values: items, item names and the opaque cursor of
//! the item listing. Each admits its input once, so internal code never handles a raw
//! string where an item is meant.
use std::{borrow::Cow, fmt};

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use veoveo_types::{IdFailure, IdMetadata, IdProfile, IdProfileSpec, IdSchema, UuidGrammar};

/// The most characters an item name holds.
pub const MAX_ITEM_NAME_CHARS: usize = 128;

/// A value that failed admission as one of the server's identities or values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UsdIdError {
    #[error("an item is a lowercase hyphenated UUIDv7")]
    Item,
    #[error("an item name is 1 to 128 characters without control characters or surrounding spaces")]
    ItemName,
}

/// An item: a UUIDv7 the server generates when it creates the item, so key order is
/// creation order. Its address is `usd://item/{item_id}`.
#[cfg_attr(feature = "server", veoveo_types::id(uuid(UsdIds), fresh, postgres))]
#[cfg_attr(not(feature = "server"), veoveo_types::id(uuid(UsdIds), fresh))]
pub struct ItemId(uuid::Uuid);

#[doc(hidden)]
pub struct UsdIds;
impl IdProfile for UsdIds {
    type Error = UsdIdError;
    const PROFILE: IdProfileSpec<Self::Error> = IdProfileSpec {
        schema: IdSchema::UuidPattern {
            pattern: "^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$",
        },
        ..IdProfileSpec::generated_uuid(UuidGrammar::canonical(&[7]), item_error)
    };
}

fn item_error(_: &str, _: IdMetadata, _: IdFailure) -> UsdIdError {
    UsdIdError::Item
}

/// An item's name: 1 to 128 characters, without control characters or surrounding
/// spaces.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ItemName(String);

impl ItemName {
    pub fn parse(value: impl Into<String>) -> Result<Self, UsdIdError> {
        let value = value.into();
        let admitted = !value.is_empty()
            && value.chars().count() <= MAX_ITEM_NAME_CHARS
            && value.trim() == value
            && !value.chars().any(char::is_control);
        admitted.then_some(Self(value)).ok_or(UsdIdError::ItemName)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ItemName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ItemName {
    type Error = UsdIdError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<ItemName> for String {
    fn from(value: ItemName) -> Self {
        value.0
    }
}

impl JsonSchema for ItemName {
    fn schema_name() -> Cow<'static, str> {
        "ItemName".into()
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "minLength": 1,
            "maxLength": MAX_ITEM_NAME_CHARS,
            "description": "The item's name, without surrounding spaces."
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_are_canonical_uuidv7() {
        let fresh = ItemId::new();
        assert_eq!(ItemId::parse(fresh.to_string()).unwrap(), fresh);
        for value in [
            "",
            "not-a-uuid",
            // A UUIDv4.
            "8c5a6f1e-2b7d-4c3a-9e1f-0a2b3c4d5e6f",
            // Uppercase spelling of a UUIDv7.
            "0190F3A2-7B1C-7D2E-8F3A-4B5C6D7E8F90",
        ] {
            assert_eq!(ItemId::parse(value), Err(UsdIdError::Item), "{value:?}");
        }
    }

    #[test]
    fn names_are_trimmed_bounded_and_printable() {
        for name in [
            "a",
            "Quarterly report",
            "ünïcode",
            &"x".repeat(MAX_ITEM_NAME_CHARS),
        ] {
            assert_eq!(ItemName::parse(name).unwrap().as_str(), name);
        }
        let long = "x".repeat(MAX_ITEM_NAME_CHARS + 1);
        for name in ["", " leading", "trailing ", "line\nbreak", long.as_str()] {
            assert_eq!(ItemName::parse(name), Err(UsdIdError::ItemName), "{name:?}");
        }
    }
}
