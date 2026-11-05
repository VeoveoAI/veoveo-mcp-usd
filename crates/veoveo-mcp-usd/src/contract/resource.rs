//! The server's resource addresses under the `usd` scheme. The route declaration
//! owns each shape and its typed parameters; the shared URI machinery owns encoding and
//! parsing, so no address is built or split by hand.
use std::{borrow::Cow, error::Error, fmt};

use veoveo_types::{ResourceFieldCodec, ResourceRouteError, ResourceUriError, Vocabulary};

use super::ItemId;

/// A resource address that failed admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsdResourceError {
    Uri(ResourceUriError),
    UnknownResource,
    InvalidId,
}

impl fmt::Display for UsdResourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Uri(error) => error.fmt(f),
            Self::UnknownResource => f.write_str("unknown or noncanonical usd resource address"),
            Self::InvalidId => f.write_str("invalid item identity"),
        }
    }
}

impl Error for UsdResourceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Uri(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ResourceUriError> for UsdResourceError {
    fn from(error: ResourceUriError) -> Self {
        Self::Uri(error)
    }
}

/// A document the server embeds and serves under `usd://docs/{doc_id}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Vocabulary)]
pub enum UsdDocument {
    /// The agent work manual, `AGENTS.md`.
    Agents,
    /// The design, `DESIGN.md`.
    Design,
}

/// A resource of the server. A usable address carries only the identities and cursor of
/// its own family.
#[veoveo_types::resource_address(routes(UsdAddresses), schema = derived, wire, parse = borrowed, display)]
pub enum UsdResource {
    #[resource(template = "usd://docs")]
    Docs,
    #[resource(template = "usd://docs/{doc_id}")]
    Document(#[resource(variable = "doc_id", codec = UsdFieldCodec)] UsdDocument),
    #[resource(template = "usd://contract")]
    Contract,
    #[resource(template = "usd://items")]
    Items,
    #[resource(template = "usd://item/{item_id}")]
    Item(#[resource(variable = "item_id", codec = UsdFieldCodec)] ItemId),
}

impl UsdResource {
    /// The first page of the caller's items.
    pub fn items() -> Self {
        Self::Items
    }
}

fn route_error(error: ResourceRouteError) -> UsdResourceError {
    match error {
        ResourceRouteError::Uri(error) => UsdResourceError::Uri(error),
        _ => UsdResourceError::UnknownResource,
    }
}

struct UsdFieldCodec;

impl ResourceFieldCodec<UsdDocument> for UsdFieldCodec {
    type Error = UsdResourceError;
    fn parse(value: &str) -> Result<UsdDocument, Self::Error> {
        value.parse().map_err(|_| UsdResourceError::UnknownResource)
    }
    fn text(value: &UsdDocument) -> Cow<'_, str> {
        value.as_str().into()
    }
}

impl ResourceFieldCodec<ItemId> for UsdFieldCodec {
    type Error = UsdResourceError;
    fn parse(value: &str) -> Result<ItemId, Self::Error> {
        ItemId::parse(value).map_err(|_| UsdResourceError::InvalidId)
    }
    fn text(value: &ItemId) -> Cow<'_, str> {
        value.to_string().into()
    }
}

#[doc(hidden)]
pub struct UsdAddresses;
impl veoveo_types::ResourceProfile for UsdAddresses {
    type Error = UsdResourceError;
    const PROFILE: veoveo_types::ResourceProfileSpec<Self::Error> =
        veoveo_types::ResourceProfileSpec {
            route_error: |_, error| route_error(error),
        };
}

#[cfg(test)]
mod tests {
    use super::*;
    use veoveo_types::ResourceAddress;

    const ITEM: &str = "0190f3a2-7b1c-7d2e-8f3a-4b5c6d7e8f90";

    fn parse(uri: &str) -> Result<UsdResource, UsdResourceError> {
        UsdResource::parse(uri)
    }

    #[test]
    fn addresses_round_trip_through_their_routes() {
        for uri in [
            "usd://docs".to_owned(),
            "usd://docs/agents".to_owned(),
            "usd://contract".to_owned(),
            "usd://items".to_owned(),
            format!("usd://item/{ITEM}"),
        ] {
            let address = parse(&uri).unwrap_or_else(|error| panic!("{uri}: {error}"));
            assert_eq!(address.to_uri().unwrap().as_str(), uri);
        }
        assert_eq!(
            parse(&format!("usd://item/{ITEM}")).unwrap(),
            UsdResource::Item(ItemId::parse(ITEM).unwrap())
        );
    }

    #[test]
    fn malformed_addresses_are_refused() {
        for uri in [
            "usd://item".to_owned(),
            "usd://item/".to_owned(),
            "usd://item/not-an-item".to_owned(),
            format!("usd://item/{ITEM}/extra"),
            "usd://items?cursor=a=b".to_owned(),
            "usd://items?unknown=1".to_owned(),
            "usd://docs/unknown".to_owned(),
            "usd://other".to_owned(),
            "media://items".to_owned(),
        ] {
            assert!(parse(&uri).is_err(), "{uri}");
        }
    }
}
