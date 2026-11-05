//! The USD server's library. The `contract` feature holds the server's public types; the
//! default `server` feature adds the hosted server, which reads stages from Lore through
//! the Hub as the person who asked, reports them, renders them on the GPU and serves the
//! viewer the Hub opens them with.

#[cfg(feature = "contract")]
pub mod contract;

#[cfg(feature = "server")]
pub mod hub;
#[cfg(feature = "server")]
pub mod mcp;
#[cfg(feature = "render")]
pub mod render;
#[cfg(feature = "server")]
pub mod server;
#[cfg(feature = "server")]
pub mod stage;
#[cfg(feature = "server")]
pub mod viewer;
#[cfg(feature = "server")]
pub mod work;

#[cfg(feature = "contract")]
use veoveo_types::ServerSlug;

/// The component name, taken from the crate name.
pub fn component() -> &'static str {
    env!("CARGO_PKG_NAME")
}

/// The server's slug, which names its schema `usd`, its roles and its URI scheme.
#[cfg(feature = "contract")]
pub fn server() -> ServerSlug {
    ServerSlug::parse("usd").expect("the server's slug")
}

#[cfg(test)]
mod tests {
    #[test]
    fn component_is_the_crate_name() {
        assert_eq!(super::component(), "veoveo-mcp-usd");
    }
}
