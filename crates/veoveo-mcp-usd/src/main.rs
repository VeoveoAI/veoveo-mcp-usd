#[tokio::main]
async fn main() -> anyhow::Result<()> {
    veoveo_mcp_usd::server::run().await
}
