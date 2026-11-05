//! The hosted process: register with the Hub, connect with the registration's
//! database login, hold a process instance, migrate, then serve MCP until SIGTERM or
//! Ctrl-C or until the instance loses its lock. Process stop preserves native resume credentials.
mod config;
mod registration;

use std::{net::SocketAddr, sync::Arc};

use anyhow::{Context, Result};
use clap::Parser;
use veoveo_hub_sdk::{
    PublicDeployment, TelemetryGuard,
    hosting::{Hosted, HostedServer},
    init_server_telemetry,
    registration::client::Registration,
};
use veoveo_store::{Instance, PlatformStore, StoreConfig};

use crate::{
    mcp::{self, UsdMcp},
    store::ItemStore,
};

use config::Args;

pub async fn run() -> Result<()> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let _ = dotenvy::dotenv();
    let args = Args::parse();
    let deployment = args.public_deployment()?;
    let declaration = mcp::registration(&deployment, args.deployment_id)?;
    mcp::REGISTRATION
        .set(declaration.clone())
        .map_err(|_| anyhow::anyhow!("the registration was declared twice"))?;
    mcp::force_setup();
    if args.describe_registration {
        let metadata = registration::prepare(&args, declaration)?;
        serde_json::to_writer_pretty(std::io::stdout().lock(), &metadata)?;
        println!();
        return Ok(());
    }
    let _telemetry: TelemetryGuard =
        init_server_telemetry("veoveo-mcp-usd", "info,veoveo_mcp_usd=debug")?;
    let (client, registration, http) = registration::register(&args, declaration).await?;
    // Process stop preserves saved native registration; explicit retirement and the
    // Hub's existing orphan grace remain separate.
    let issued = registration.clone();
    let mut served = None;
    let maintained = client
        .maintain(registration, async {
            served = Some(serve(&args, &deployment, &issued, http).await);
        })
        .await;
    if let Err(error) = maintained {
        tracing::warn!(%error, "registration lifecycle failed");
    }
    served.context("the registration ended before serving finished")?
}

/// Connect with the registration's database login, migrate the server's schema and
/// serve until SIGTERM or Ctrl-C, or until the process instance loses its lock.
async fn serve(
    args: &Args,
    deployment: &PublicDeployment,
    registration: &Registration,
    http: reqwest::Client,
) -> Result<()> {
    let grant = registration
        .database()
        .context("the Hub issued no database login")?;
    let store = PlatformStore::connect(
        StoreConfig::builder(grant.url.as_str(), grant.password.clone())
            .username(grant.username.as_str())
            .build()?,
    )
    .await
    .context("connecting to the installation database")?;
    let instance = Arc::new(
        Instance::start(&store, &crate::server())
            .await
            .context("starting the process instance")?,
    );
    store
        .migrate_server(&crate::server(), crate::migrator())
        .await
        .context("applying the server's migrations")?;
    let items = ItemStore::new(store.clone());
    let ready_instance = instance.clone();
    let hosted = HostedServer::for_domain::<UsdMcp>()
        .deployment(deployment, args.allow_loopback_hosts)?
        .allowed_hosts(args.allowed_hosts.iter().cloned())
        .internal_trust(registration.access_tokens(), http)?
        .handler(move || Hosted::new(UsdMcp::new(items.clone())))
        // The SDK's existing anonymous /readyz is installed only after native
        // registration, database connection and schema initialization succeeded.
        .readiness(move || {
            let instance = ready_instance.clone();
            async move { !instance.is_lost() }
        })
        .build();
    let served = tokio::select! {
        served = hosted.serve(SocketAddr::from(([0, 0, 0, 0], args.port))) => served,
        () = instance.lost() => Err(anyhow::anyhow!("the process instance lost its lock")),
    };
    if let Err(error) = instance.shutdown().await {
        tracing::warn!(%error, "the process instance did not stop cleanly");
    }
    store.close().await;
    served
}
