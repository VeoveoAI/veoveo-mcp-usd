//! The server's SQL names only the `usd` schema. Core state comes through
//! veoveo-hub-sdk and core's runtime API.

#[test]
fn sql_names_only_the_usd_schema() {
    veoveo_hub_sdk::testing::assert_sql_confined(env!("CARGO_MANIFEST_DIR"), "usd");
}
