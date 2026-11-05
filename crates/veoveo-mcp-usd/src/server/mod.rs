//! The hosted process: register with the Hub, connect the Task runtime with the registration's
//! database login, then serve MCP and the viewer while the render worker runs, until SIGTERM or
//! Ctrl-C or until the worker can no longer claim Tasks. Process stop preserves native resume
//! credentials.
mod config;
mod registration;

use std::{net::SocketAddr, time::Duration};

use anyhow::{Context, Result};
use clap::Parser;
use tokio_util::sync::CancellationToken;
use veoveo_hub_sdk::{
    PublicDeployment, TelemetryGuard,
    apps::AppHtml,
    durable_tasks::{DurableTasks, TaskWorker},
    hosting::{Hosted, HostedServer, ServerCalls},
    init_server_telemetry,
    registration::{ClientMetadata, client::Registration, statement::ServerKeyPair},
};
use veoveo_task::{TaskRuntime, TaskRuntimeConfig};

use crate::{
    hub::Hub,
    mcp::{self, UsdMcp, UsdTasks},
    viewer,
    work::{self, RenderWork},
};

use config::Args;

/// How long a render's Task stays readable after it ends.
const RENDER_RETENTION: Duration = Duration::from_secs(7 * 24 * 60 * 60);

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
    let app = AppHtml::load(&args.app_html)?;
    let registration::Registered {
        client,
        registration,
        metadata,
        key,
        http,
    } = registration::register(&args, declaration).await?;
    // Process stop preserves saved native registration; explicit retirement and the
    // Hub's existing orphan grace remain separate.
    let issued = registration.clone();
    let mut served = None;
    let maintained = client
        .maintain(registration, async {
            served = Some(serve(&args, &deployment, &issued, &metadata, &key, http, app).await);
        })
        .await;
    if let Err(error) = maintained {
        tracing::warn!(%error, "registration lifecycle failed");
    }
    served.context("the registration ended before serving finished")?
}

/// Connect the Task runtime with the registration's database login and serve until SIGTERM or
/// Ctrl-C, or until the worker stops.
async fn serve(
    args: &Args,
    deployment: &PublicDeployment,
    registration: &Registration,
    metadata: &ClientMetadata,
    key: &ServerKeyPair,
    http: reqwest::Client,
    app: AppHtml,
) -> Result<()> {
    let grant = registration
        .database()
        .context("the Hub issued no database login")?;
    let runtime = TaskRuntime::connect(
        TaskRuntimeConfig::new(
            grant.url.as_str(),
            grant.username.as_str(),
            grant.password.clone(),
        )?,
        crate::server(),
        registration.access_tokens().deployment,
    )
    .await
    .context("connecting to the installation database")?;
    let instance = runtime.instance().clone();
    let calls = ServerCalls::discover(registration, metadata, key, http.clone())
        .await
        .context("discovering the Hub's token endpoint")?;
    let hub = Hub::new(
        args.hub_issuer
            .as_ref()
            .context("serving requires a Hub issuer")?,
        http.clone(),
    )?;
    let worker = TaskWorker::new(runtime.clone()).calls(calls.clone());
    let tasks = UsdTasks::new(runtime.clone(), RENDER_RETENTION);
    let domain = UsdMcp::new(hub.clone(), app);
    let stop = CancellationToken::new();
    let mut working = tokio::spawn(work::work(
        worker,
        RenderWork {
            hub,
            http: http.clone(),
        },
        stop.clone(),
    ));
    // The Hub reaches the viewer's route with its own host, the public base's.
    let public_base = registration
        .public_base()
        .context("the Hub named no public base for the viewer")?;
    let deployment = deployment
        .clone()
        .with_public_base(Some(public_base.clone()));
    let hosted = HostedServer::for_domain::<UsdMcp>()
        .deployment(&deployment, args.allow_loopback_hosts)?
        .allowed_hosts(args.allowed_hosts.iter().cloned())
        .internal_trust(registration.access_tokens(), http)?
        .handler(move || {
            Hosted::new(domain.clone()).with_tasks(DurableTasks::tasks_only(tasks.clone()))
        })
        .calls(calls)
        .public_routes(viewer::router(&args.viewer_directory))
        .build();
    let served = tokio::select! {
        served = hosted.serve(SocketAddr::from(([0, 0, 0, 0], args.port))) => served,
        worked = &mut working => match worked {
            Ok(Ok(())) => Err(anyhow::anyhow!("the render worker stopped")),
            Ok(Err(error)) => Err(anyhow::Error::from(error).context("the server stopped claiming renders")),
            Err(error) => Err(anyhow::Error::from(error).context("the render worker failed")),
        },
    };
    stop.cancel();
    if !working.is_finished() {
        let _ = working.await;
    }
    if let Err(error) = instance.shutdown().await {
        tracing::warn!(%error, "the process instance did not stop cleanly");
    }
    runtime.platform_store().close().await;
    served
}
