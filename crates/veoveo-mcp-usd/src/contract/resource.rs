//! The server's resource addresses under the `usd` scheme. The route declaration
//! owns each shape and its typed parameters; the shared URI machinery owns encoding and
//! parsing, so no address is built or split by hand.
use std::{borrow::Cow, error::Error, fmt};

use veoveo_types::{ResourceFieldCodec, ResourceRouteError, ResourceUriError, Vocabulary};

/// A resource address that failed admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsdResourceError {
    Uri(ResourceUriError),
    UnknownResource,
}

impl fmt::Display for UsdResourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Uri(error) => error.fmt(f),
            Self::UnknownResource => f.write_str("unknown or noncanonical usd resource address"),
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

/// A resource of the server.
#[veoveo_types::resource_address(routes(UsdAddresses), schema = derived, wire, parse = borrowed, display)]
pub enum UsdResource {
    #[resource(template = "usd://docs")]
    Docs,
    #[resource(template = "usd://docs/{doc_id}")]
    Document(#[resource(variable = "doc_id", codec = UsdFieldCodec)] UsdDocument),
    #[resource(template = "usd://contract")]
    Contract,
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

    fn parse(uri: &str) -> Result<UsdResource, UsdResourceError> {
        UsdResource::parse(uri)
    }

    #[test]
    fn addresses_round_trip_through_their_routes() {
        for uri in ["usd://docs", "usd://docs/agents", "usd://contract"] {
            let address = parse(uri).unwrap_or_else(|error| panic!("{uri}: {error}"));
            assert_eq!(address.to_uri().unwrap().as_str(), uri);
        }
    }

    #[test]
    fn malformed_addresses_are_refused() {
        for uri in [
            "usd://docs/unknown",
            "usd://other",
            "usd://contract/extra",
            "media://contract",
        ] {
            assert!(parse(uri).is_err(), "{uri}");
        }
    }
}
