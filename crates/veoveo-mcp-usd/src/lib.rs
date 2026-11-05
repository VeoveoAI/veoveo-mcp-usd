//! The USD server's library. The `contract` feature holds the server's public
//! types; the default `server` feature adds the hosted server, which keeps each tenant's
//! items in the `usd` schema and serves them over MCP through the SDK. The domain
//! is the smallest that exercises a tool with a product, an exact read and a keyset
//! listing, so a new server starts from code that passes the contract and database
//! checks.

#[cfg(feature = "contract")]
pub mod contract;

#[cfg(feature = "server")]
pub mod mcp;
#[cfg(feature = "render")]
pub mod render;
#[cfg(feature = "server")]
pub mod server;
#[cfg(feature = "server")]
pub mod store;

#[cfg(feature = "contract")]
use veoveo_types::ServerSlug;

/// The component name, taken from the crate name.
pub fn component() -> &'static str {
    env!("CARGO_PKG_NAME")
}

/// The server's slug, which names its schema `usd`, its roles, its channel and its
/// URI scheme. Rename it with the server, together with the schema in the SQL files.
#[cfg(feature = "contract")]
pub fn server() -> ServerSlug {
    ServerSlug::parse("usd").expect("the server's slug")
}

/// The migrations in `migrations/`, which the server applies in its own schema at
/// startup and which record their history in `usd._sqlx_migrations`.
#[cfg(feature = "server")]
pub fn migrator() -> sqlx::migrate::Migrator {
    sqlx::migrate!()
}

#[cfg(test)]
mod tests {
    #[test]
    fn component_is_the_crate_name() {
        assert_eq!(super::component(), "veoveo-mcp-usd");
    }
}
