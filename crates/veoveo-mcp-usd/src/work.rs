//! The `render_stage` Task's work: gather the stage as its owner, render one frame in a child
//! process on the GPU, and store the PNG in Artifact bound to the Task. One durable step does it
//! all, so a Task that loses its process runs it again with the same idempotency key.
use std::{
    io::Write as _,
    path::Path,
    process::Stdio,
    time::{Duration, Instant},
};

use anyhow::{Context, anyhow, bail};
use serde_json::{Map, json};
use tokio_util::sync::CancellationToken;
use usd_bevy::instance::UsdInstanceOverrides;
use veoveo_hub_sdk::{
    HubProfileId,
    definition::Product,
    durable_tasks::{TaskContext, TaskStop, TaskWorkError, TaskWorker, mcp_task_completion},
    hosting::task_result,
};
use veoveo_mcp_artifact::{
    client::{ArtifactClient, UploadBody},
    contract::{
        ArtifactId, ArtifactLabels, ArtifactMetadataMap, ArtifactName, ArtifactResource,
        IdempotencyKey, MediaType,
    },
};
use veoveo_task::{StepKey, TaskError, TaskFailure, TaskTransition};

use crate::{
    contract::{RENDERED_FORMAT, RenderCamera, RenderStageRequest, RenderedStage},
    hub::{Hub, Reader},
    stage::{self, Gathered, Scope},
};

/// The Task operation of `render_stage`.
pub const RENDER: veoveo_types::TaskTypeName =
    veoveo_types::TaskTypeName::from_static("usd.render");
/// The longest a child may render. A stage this slow to draw one frame is reported, not waited on.
const RENDER_TIMEOUT: Duration = Duration::from_secs(10 * 60);
/// What a finished render's Task says.
const RENDERED: &str = "Rendered.";
/// How much of a failed child's output a Task's failure carries.
const MAX_REPORTED_ERROR: usize = 2048;

/// What a render's work needs.
#[derive(Clone)]
pub struct RenderWork {
    pub hub: Hub,
    pub http: reqwest::Client,
}

/// Run the server's renders on `worker` until `stop`.
pub async fn work(
    worker: TaskWorker,
    render: RenderWork,
    stop: CancellationToken,
) -> Result<(), TaskError> {
    worker
        .run(
            move |ctx| {
                let render = render.clone();
                async move { run(ctx, render).await }
            },
            stop,
        )
        .await
}

fn keeps_the_task(error: &anyhow::Error) -> bool {
    error.downcast_ref::<TaskStop>().is_some()
        || TaskWorkError::is_parked(error)
        || TaskWorkError::is_lost(error)
        || TaskWorkError::is_interrupted(error)
}

/// One render's work. An error fails the Task under a code that names its cause, unless the
/// Task stopped with its process or lost its database connection: those go to its recovery.
async fn run(ctx: TaskContext, render: RenderWork) -> anyhow::Result<TaskTransition> {
    match attempt(&ctx, &render).await {
        Err(error) if !keeps_the_task(&error) && !ctx.stopping() => {
            let message = format!("{error:#}");
            tracing::warn!(task = %ctx.task_id(), error = %message, "a render failed");
            Ok(TaskTransition::Failed(TaskFailure::new(
                "render_failed",
                message,
            )))
        }
        outcome => outcome,
    }
}

/// What the durable step keeps: the render's facts and where its PNG is.
#[derive(serde::Serialize, serde::Deserialize)]
struct Stored {
    artifact: ArtifactId,
    rendered: RenderedStage,
}

async fn attempt(ctx: &TaskContext, render: &RenderWork) -> anyhow::Result<TaskTransition> {
    let request: RenderStageRequest = serde_json::from_value(ctx.task().request.clone())
        .context("the Task's request is not a render")?;
    let key = StepKey::parse("render").map_err(|error| anyhow!("{error}"))?;
    let stored: Stored = ctx
        .step(&key, |_| render_and_store(ctx, render, &request))
        .await?;
    let product = Product::new(
        &ArtifactResource::artifact(stored.artifact),
        stored.rendered,
    )?;
    let completion = task_result::<RenderStageRequest>(&product)
        .map_err(|error| anyhow!(error.message.to_string()))?;
    Ok(mcp_task_completion(RENDERED, completion)?)
}

async fn render_and_store(
    ctx: &TaskContext,
    render: &RenderWork,
    request: &RenderStageRequest,
) -> anyhow::Result<Stored> {
    let task = ctx.task();
    let reader = Reader {
        hub: render.hub.clone(),
        client: ctx.client()?,
        profile: HubProfileId::parse(task.owner.surface.as_str())
            .map_err(|error| anyhow!("the Task names no Hub profile: {error}"))?,
    };
    ctx.update(TaskTransition::Running {
        message: "Reading the stage.".into(),
        progress: 0.1,
    })
    .await?;
    let gathered = stage::gather(
        &reader,
        &request.repository,
        &request.branch,
        &request.path,
        Scope::Everything,
        &UsdInstanceOverrides {
            variants: request
                .variants
                .iter()
                .map(|choice| {
                    (
                        choice.prim.to_string(),
                        choice.set.clone(),
                        choice.selection.clone(),
                    )
                })
                .collect(),
            attributes: vec![],
        },
    )
    .await
    .context("reading the stage")?;
    ctx.update(TaskTransition::Running {
        message: "Rendering.".into(),
        progress: 0.4,
    })
    .await?;
    let folder = tempfile::tempdir().context("making the render's temporary folder")?;
    let drawn = draw(ctx, folder.path(), gathered.clone(), request).await?;
    ctx.update(TaskTransition::Running {
        message: "Storing the image.".into(),
        progress: 0.9,
    })
    .await?;
    let png = tokio::fs::read(&drawn.output)
        .await
        .context("reading the rendered image")?;
    let size = png.len() as u64;
    let labels = ArtifactLabels::new(task.owner.data_labels.clone())
        .map_err(|error| anyhow!("the render's labels: {error}"))?;
    let mut metadata = Map::new();
    metadata.insert("taskId".into(), json!(ctx.task_id().to_string()));
    metadata.insert("repository".into(), json!(request.repository));
    metadata.insert("branch".into(), json!(request.branch));
    metadata.insert("path".into(), json!(request.path));
    metadata.insert("revision".into(), json!(gathered.revision));
    metadata.insert("address".into(), json!(gathered.address));
    metadata.insert("time".into(), json!(request.time));
    metadata.insert("camera".into(), json!(request.camera));
    metadata.insert("variants".into(), json!(request.variants));
    let name = artifact_name(request);
    let record = ArtifactClient::new(ctx.client()?, render.http.clone())
        .upload(
            ArtifactName::parse(name.as_str()).map_err(|error| anyhow!("{name}: {error}"))?,
            MediaType::parse("image/png").expect("a valid media type"),
            UploadBody::bytes(png),
        )
        .idempotency_key(
            IdempotencyKey::parse(format!("usd:{}:render", ctx.task_id()))
                .map_err(|error| anyhow!("{error}"))?,
        )
        .labels(labels)
        .metadata(ArtifactMetadataMap::new(metadata).map_err(|error| anyhow!("{error}"))?)
        .await
        .context("storing the image in Artifact")?;
    Ok(Stored {
        artifact: record.artifact_id,
        rendered: RenderedStage {
            format: RENDERED_FORMAT.to_owned(),
            repository: request.repository.clone(),
            branch: request.branch.clone(),
            path: request.path.clone(),
            revision: gathered.revision,
            address: gathered.address,
            size,
            adapter: drawn.adapter,
            render_ms: drawn.milliseconds,
        },
    })
}

/// `hummingbird.usdz` at time 12 is `hummingbird-t12.png`.
fn artifact_name(request: &RenderStageRequest) -> String {
    let file = request.path.file_name();
    let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    match request.time {
        Some(time) => format!("{stem}-t{time}.png"),
        None => format!("{stem}.png"),
    }
}

/// What the child drew.
struct Drawn {
    output: std::path::PathBuf,
    adapter: String,
    milliseconds: u64,
}

/// The line of `FRAMES.txt` that asks for `request`'s one frame.
fn frame_line(request: &RenderStageRequest, output: &Path) -> anyhow::Result<String> {
    let output = output.display().to_string();
    anyhow::ensure!(
        !output.contains(char::is_whitespace),
        "the temporary folder's path holds a space"
    );
    let time = request
        .time
        .map_or("start".to_owned(), |time| time.to_string());
    Ok(match &request.camera {
        None => format!("{time} {output}\n"),
        Some(RenderCamera::Prim { prim }) => format!("{time} {output} @{prim}\n"),
        Some(RenderCamera::Look { eye, focus }) => format!(
            "{time} {output} {} {} {} {} {} {}\n",
            eye[0], eye[1], eye[2], focus[0], focus[1], focus[2]
        ),
    })
}

/// Writes the stage's files in their repository layout under `folder`, and draws one frame in a
/// child process, which a cancelled Task kills.
async fn draw(
    ctx: &TaskContext,
    folder: &Path,
    gathered: Gathered,
    request: &RenderStageRequest,
) -> anyhow::Result<Drawn> {
    let output = folder.join("frame.png");
    let frames = folder.join("frames.txt");
    let line = frame_line(request, &output)?;
    let files = folder.join("files");
    let stage = files.join(gathered.root.as_str());
    {
        let files = files.clone();
        tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
            for (path, bytes) in &gathered.files {
                let path = files.join(path);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::File::create(&path)?.write_all(bytes)?;
            }
            Ok(())
        })
        .await?
        .context("writing the stage's files")?;
    }
    tokio::fs::write(&frames, line).await?;
    let mut command = tokio::process::Command::new(std::env::current_exe()?);
    command.arg("render").arg(&stage).arg(&frames);
    for choice in &request.variants {
        command
            .arg("--variant")
            .arg(choice.prim.as_str())
            .arg(&choice.set)
            .arg(&choice.selection);
    }
    command
        // The child holds no credential of the server's, and a failure says only why it failed.
        .env_remove("VEOVEO_REGISTRATION_SECRET")
        .env("RUST_LOG", "error")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let started = Instant::now();
    let child = command.spawn().context("starting the render")?;
    let finished = tokio::select! {
        finished = tokio::time::timeout(RENDER_TIMEOUT, child.wait_with_output()) => finished,
        () = ctx.cancelled() => {
            ctx.checkpoint().await?;
            bail!("the render was stopped");
        }
    };
    let finished = finished
        .map_err(|_| anyhow!("the render took longer than {RENDER_TIMEOUT:?}"))?
        .context("waiting for the render")?;
    let stdout = String::from_utf8_lossy(&finished.stdout);
    if !finished.status.success() {
        let stderr = String::from_utf8_lossy(&finished.stderr);
        let start = stderr.len().saturating_sub(MAX_REPORTED_ERROR);
        let tail = stderr.get(start..).unwrap_or_default();
        bail!(
            "the render exited with {}: {}",
            finished.status,
            tail.trim()
        );
    }
    let adapter = stdout
        .lines()
        .find_map(|line| line.strip_prefix("ADAPTER "))
        .map(|line| line.split(" backend=").next().unwrap_or(line).to_owned())
        .unwrap_or_default();
    Ok(Drawn {
        output,
        adapter,
        milliseconds: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::{BranchName, PrimPath, RepositoryId, StagePath};

    fn request() -> RenderStageRequest {
        RenderStageRequest {
            repository: RepositoryId::parse("r").unwrap(),
            branch: BranchName::parse("main").unwrap(),
            path: StagePath::parse("bird/bird.usdz").unwrap(),
            time: None,
            camera: None,
            variants: vec![],
        }
    }

    #[test]
    fn a_frame_line_says_what_was_asked() {
        let output = Path::new("/t/f.png");
        assert_eq!(frame_line(&request(), output).unwrap(), "start /t/f.png\n");
        let mut asked = request();
        asked.time = Some(12.5);
        asked.camera = Some(RenderCamera::Prim {
            prim: PrimPath::parse("/World/Cam").unwrap(),
        });
        assert_eq!(
            frame_line(&asked, output).unwrap(),
            "12.5 /t/f.png @/World/Cam\n"
        );
        asked.camera = Some(RenderCamera::Look {
            eye: [1.0, 2.0, 3.0],
            focus: [4.0, 5.5, 6.0],
        });
        assert_eq!(
            frame_line(&asked, output).unwrap(),
            "12.5 /t/f.png 1 2 3 4 5.5 6\n"
        );
    }

    #[test]
    fn an_artifact_is_named_for_its_stage_and_time() {
        assert_eq!(artifact_name(&request()), "bird.png");
        let mut asked = request();
        asked.time = Some(12.0);
        assert_eq!(artifact_name(&asked), "bird-t12.png");
    }
}
