//! Renders frames of a USD stage to PNG files on the GPU, with no window: the server's
//! `render` subcommand, which runs apart from the server because Bevy owns its process's
//! main loop. It starts from `gif_frames`, upstream's example, under the MIT license in
//! `crates/usd_bevy/LICENSE`.
//!
//! ```sh
//! veoveo-mcp-usd render STAGE FRAMES.txt [--variant PRIM SET SELECTION]...
//! ```
//!
//! Each line of `FRAMES.txt` is one frame: `TIME OUTPUT.png`, which frames the whole stage,
//! `TIME OUTPUT.png EYE_X EYE_Y EYE_Z FOCUS_X FOCUS_Y FOCUS_Z`, or `TIME OUTPUT.png @/PRIM/PATH`,
//! which looks through one of the stage's cameras. A `TIME` of `start` is the stage's start.

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Result, anyhow, bail};
use bevy::{
    app::{AppExit, ScheduleRunnerPlugin},
    camera::RenderTarget,
    core_pipeline::prepass::DepthPrepass,
    light::CascadeShadowConfigBuilder,
    prelude::*,
    render::{
        RenderPlugin,
        render_resource::{TextureFormat, TextureUsages},
        renderer::RenderAdapterInfo,
        view::screenshot::{Screenshot, ScreenshotCaptured},
    },
    window::ExitCondition,
};
use usd_bevy::{
    DisplayPurposes, UsdAssetPlugin, UsdPlugin, UsdProjectionBudget, UsdScene, UsdSceneRoot,
    UsdSceneState,
    instance::{UsdInstanceOverrides, UsdInstanceTime, UsdInstances},
    mesh::bounds::StageBounds,
    route::{
        camera::UsdCamera,
        dome_environment::UsdDomeEnvironmentPlugin,
        gpu_instancing::{UsdGpuInstancing, UsdGpuInstancingPlugin},
        gpu_skin::UsdGpuSkinningPlugin,
    },
};

/// The size of a rendered frame.
const SIZE: (u32, u32) = (1280, 720);
/// Frames drawn before the first capture, so pipelines and textures settle.
const WARMUP: u32 = 30;
/// Frames drawn after a pose change before it is captured.
const SETTLE: u32 = 3;
/// Where the framed camera looks from, relative to the stage's center.
const VIEW: Vec3 = Vec3::new(0.7, 0.45, 1.0);
/// How far the framed camera stands, in the stage's largest extents: Bevy's default field of view
/// shows a whole extent from 1.21 of them, with a margin here for its depth.
const FRAMED_DISTANCE: f32 = 1.4;
/// Frames a ready stage may show nothing before the render gives up on it.
const EMPTY: u32 = 120;

/// Where a frame looks from.
#[derive(Debug, PartialEq)]
enum FrameCamera {
    /// The whole stage.
    Stage,
    /// From an eye at a focus.
    Look(Vec3, Vec3),
    /// Through a camera of the stage.
    Prim(String),
}

/// One frame to render.
#[derive(Debug, PartialEq)]
struct Frame {
    /// The time code; the stage's start when none is named.
    time: Option<f64>,
    output: PathBuf,
    camera: FrameCamera,
}

/// The frames, and how far through them the render is.
#[derive(Resource)]
struct Recorder {
    frames: Vec<Frame>,
    next: usize,
    wait: u32,
    posed: bool,
    pending: bool,
    started: Instant,
    ready: bool,
}

/// The texture every frame draws to.
#[derive(Resource)]
struct Target(Handle<Image>);

/// The stage to load, by its name in its directory, and the variants to select.
#[derive(Resource)]
struct Stage {
    name: String,
    variants: Vec<(String, String, String)>,
}

/// The loaded stage's center and largest extent, in world space.
#[derive(Resource, Clone, Copy)]
struct Fit {
    center: Vec3,
    span: f32,
}

/// Parses `FRAMES.txt`.
fn parse(text: &str) -> Result<Vec<Frame>> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let words: Vec<&str> = line.split_whitespace().collect();
            let number = |word: &str| {
                word.parse::<f32>()
                    .map_err(|error| anyhow!("bad number {word} in: {line}: {error}"))
            };
            let (time, output, camera) = match words[..] {
                [time, output] => (time, output, FrameCamera::Stage),
                [time, output, prim] if prim.starts_with('@') => {
                    (time, output, FrameCamera::Prim(prim[1..].to_owned()))
                }
                [time, output, ex, ey, ez, fx, fy, fz] => (
                    time,
                    output,
                    FrameCamera::Look(
                        Vec3::new(number(ex)?, number(ey)?, number(ez)?),
                        Vec3::new(number(fx)?, number(fy)?, number(fz)?),
                    ),
                ),
                _ => bail!("expected TIME OUTPUT and an optional eye and focus or @camera: {line}"),
            };
            Ok(Frame {
                time: match time {
                    "start" => None,
                    time => Some(
                        time.parse()
                            .map_err(|error| anyhow!("bad time {time} in: {line}: {error}"))?,
                    ),
                },
                output: PathBuf::from(output),
                camera,
            })
        })
        .collect()
}

/// The `--variant PRIM SET SELECTION` arguments.
fn parse_variants(args: &[String]) -> Result<Vec<(String, String, String)>> {
    args.chunks(4)
        .map(|chunk| match chunk {
            [flag, prim, set, selection] if flag == "--variant" => {
                Ok((prim.clone(), set.clone(), selection.clone()))
            }
            _ => bail!("expected --variant PRIM SET SELECTION"),
        })
        .collect()
}

/// Renders `STAGE FRAMES.txt`, the subcommand's arguments.
pub fn run(args: &[String]) -> Result<()> {
    let [stage, list, variants @ ..] = args else {
        bail!("usage: render STAGE FRAMES.txt [--variant PRIM SET SELECTION]...");
    };
    let variants = parse_variants(variants)?;
    let text = std::fs::read_to_string(list).map_err(|error| anyhow!("reading {list}: {error}"))?;
    let frames = parse(&text)?;
    if frames.is_empty() {
        bail!("no frames in {list}");
    }
    let stage =
        std::path::absolute(stage).map_err(|error| anyhow!("resolving {stage}: {error}"))?;
    let (Some(directory), Some(name)) = (stage.parent(), stage.file_name()) else {
        bail!("{} names no file in a directory", stage.display());
    };
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(RenderPlugin {
                synchronous_pipeline_compilation: true,
                ..default()
            })
            .set(AssetPlugin {
                file_path: directory.to_string_lossy().into_owned(),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                ..default()
            }),
    )
    .add_plugins((
        ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(1.0 / 60.0)),
        UsdPlugin,
        UsdAssetPlugin,
        UsdGpuSkinningPlugin,
        UsdDomeEnvironmentPlugin,
        UsdGpuInstancingPlugin,
    ))
    .init_resource::<UsdGpuInstancing>()
    .insert_resource(UsdProjectionBudget(Duration::from_secs(5)))
    .insert_resource(DisplayPurposes {
        render: true,
        proxy: false,
        guide: false,
    })
    .insert_resource(Stage {
        name: name.to_string_lossy().into_owned(),
        variants,
    })
    .insert_resource(Recorder {
        frames,
        next: 0,
        wait: WARMUP,
        posed: false,
        pending: false,
        started: Instant::now(),
        ready: false,
    })
    .add_systems(Startup, (require_gpu, setup))
    .add_systems(Last, (fit, record).chain());
    match app.run() {
        AppExit::Success => Ok(()),
        AppExit::Error(code) => bail!("the render failed with code {code}"),
    }
}

/// Fails on a software adapter: a render is never a slow surprise.
fn require_gpu(adapter: Res<RenderAdapterInfo>, mut exit: MessageWriter<AppExit>) {
    println!(
        "ADAPTER {} backend={:?} type={:?} driver={}",
        adapter.name, adapter.backend, adapter.device_type, adapter.driver_info
    );
    if format!("{:?}", adapter.device_type) == "Cpu" {
        eprintln!("refusing the software adapter {}", adapter.name);
        exit.write(AppExit::error());
    }
}

fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    server: Res<AssetServer>,
    stage: Res<Stage>,
) {
    let mut image = Image::new_target_texture(SIZE.0, SIZE.1, TextureFormat::Rgba8UnormSrgb, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let target = images.add(image);
    commands.spawn((
        Camera3d::default(),
        DepthPrepass,
        Msaa::Sample4,
        RenderTarget::from(target.clone()),
        Transform::default(),
        AmbientLight {
            color: Color::srgb(0.78, 0.85, 1.0),
            brightness: 160.0,
            ..default()
        },
    ));
    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: true,
            ..default()
        },
        Transform::from_translation(Vec3::new(0.5, 1.0, 0.6)).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.insert_resource(Target(target));
    let scene: Handle<UsdScene> = server.load(stage.name.clone());
    commands.spawn((
        UsdSceneRoot(scene),
        UsdInstanceTime::default(),
        UsdInstanceOverrides {
            variants: stage.variants.clone(),
            attributes: vec![],
        },
    ));
}

/// The stage's cameras, apart from the camera that draws.
type StageCameras<'w, 's> = Query<
    'w,
    's,
    (&'static GlobalTransform, &'static Projection),
    (With<UsdCamera>, Without<Camera3d>),
>;

/// Measures the stage once it is ready, and fits the clip planes and shadows to it.
#[expect(
    clippy::too_many_arguments,
    reason = "a Bevy system names what it reads"
)]
fn fit(
    fit: Option<Res<Fit>>,
    states: Query<&UsdSceneState>,
    stage: StageBounds,
    mut empty: Local<u32>,
    mut exit: MessageWriter<AppExit>,
    mut projections: Query<&mut Projection, With<Camera3d>>,
    lights: Query<Entity, With<DirectionalLight>>,
    mut commands: Commands,
) {
    if fit.is_some() || !matches!(states.iter().next(), Some(UsdSceneState::Ready)) {
        return;
    }
    let Some((low, high)) = stage.get() else {
        *empty += 1;
        if *empty > EMPTY {
            eprintln!("the stage shows no geometry");
            exit.write(AppExit::error());
        }
        return;
    };
    let span = (high - low).max_element();
    println!(
        "BOUNDS low={low} high={high} draws={} instances={}",
        stage.draws(),
        stage.instances()
    );
    for mut projection in &mut projections {
        if let Projection::Perspective(perspective) = projection.as_mut() {
            perspective.far = span * 8.0;
            perspective.near = span * 1e-3;
        }
    }
    for light in &lights {
        commands.entity(light).insert(
            CascadeShadowConfigBuilder {
                first_cascade_far_bound: span * 0.5,
                maximum_distance: span * 6.0,
                ..default()
            }
            .build(),
        );
    }
    commands.insert_resource(Fit {
        center: (low + high) * 0.5,
        span,
    });
}

#[expect(
    clippy::too_many_arguments,
    reason = "a Bevy system names what it reads"
)]
fn record(
    mut commands: Commands,
    mut recorder: ResMut<Recorder>,
    target: Res<Target>,
    fit: Option<Res<Fit>>,
    states: Query<&UsdSceneState>,
    instances: NonSend<UsdInstances>,
    mut roots: Query<(Entity, &mut UsdInstanceTime), With<UsdSceneRoot>>,
    mut cameras: Query<(&mut Transform, &mut Projection), With<Camera3d>>,
    stage_cameras: StageCameras,
    mut exit: MessageWriter<AppExit>,
) {
    match states.iter().next() {
        Some(UsdSceneState::Ready) => (),
        Some(UsdSceneState::Failed(error)) => {
            eprintln!("load failed: {error}");
            exit.write(AppExit::error());
            return;
        }
        _ => return,
    }
    let Some(fit) = fit else {
        return;
    };
    if !recorder.ready {
        recorder.ready = true;
        println!("READY {} ms", recorder.started.elapsed().as_millis());
    }
    if recorder.pending {
        return;
    }
    if recorder.next == recorder.frames.len() {
        println!("DONE {} ms", recorder.started.elapsed().as_millis());
        exit.write(AppExit::Success);
        return;
    }
    let frame = &recorder.frames[recorder.next];
    if let FrameCamera::Prim(prim) = &frame.camera {
        // Each frame, for a stage's camera may move with the new time.
        let looked = roots
            .iter()
            .find_map(|(root, _)| instances.entity(root, prim))
            .and_then(|entity| stage_cameras.get(entity).ok());
        let Some((placed, projection)) = looked else {
            eprintln!("the stage holds no camera at {prim}");
            exit.write(AppExit::error());
            return;
        };
        for (mut transform, mut own) in &mut cameras {
            *transform = placed.compute_transform();
            *own = projection.clone();
        }
    }
    if !recorder.posed {
        let start = roots
            .iter()
            .find_map(|(root, _)| instances.stage(root))
            .map_or(0.0, openusd::usd::Stage::start_time_code);
        let time = frame.time.unwrap_or(start);
        let look = match &frame.camera {
            FrameCamera::Stage => Some((
                fit.center + VIEW.normalize() * fit.span * FRAMED_DISTANCE,
                fit.center,
            )),
            FrameCamera::Look(eye, focus) => Some((*eye, *focus)),
            FrameCamera::Prim(_) => None,
        };
        for (_, mut current) in &mut roots {
            current.current = time;
        }
        if let Some((eye, focus)) = look {
            for (mut transform, _) in &mut cameras {
                *transform = Transform::from_translation(eye).looking_at(focus, Vec3::Y);
            }
        }
        recorder.posed = true;
        recorder.wait = recorder.wait.max(SETTLE);
    }
    if recorder.wait > 0 {
        recorder.wait -= 1;
        return;
    }
    recorder.pending = true;
    commands
        .spawn(Screenshot(RenderTarget::from(target.0.clone())))
        .observe(
            |event: On<ScreenshotCaptured>,
             mut recorder: ResMut<Recorder>,
             mut exit: MessageWriter<AppExit>| {
                let output = recorder.frames[recorder.next].output.clone();
                if let Err(error) = save(&event.image, &output) {
                    eprintln!("{}: {error}", output.display());
                    exit.write(AppExit::error());
                    return;
                }
                println!(
                    "FRAME {} {} {} ms",
                    recorder.next,
                    output.display(),
                    recorder.started.elapsed().as_millis()
                );
                recorder.next += 1;
                recorder.posed = false;
                recorder.pending = false;
            },
        );
}

/// Writes a capture as a PNG.
fn save(image: &Image, output: &Path) -> Result<()> {
    image
        .clone()
        .try_into_dynamic()
        .map_err(|error| anyhow!("converting the capture: {error}"))?
        .to_rgba8()
        .save(output)
        .map_err(|error| anyhow!("writing the image: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_frames_the_stage_or_names_its_camera() {
        let frames = parse("0 a.png\n\n24.5 b.png 1 2 3 4 5 6\nstart c.png @/World/Cam\n").unwrap();
        assert_eq!(
            frames,
            [
                Frame {
                    time: Some(0.0),
                    output: "a.png".into(),
                    camera: FrameCamera::Stage,
                },
                Frame {
                    time: Some(24.5),
                    output: "b.png".into(),
                    camera: FrameCamera::Look(Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0)),
                },
                Frame {
                    time: None,
                    output: "c.png".into(),
                    camera: FrameCamera::Prim("/World/Cam".into()),
                },
            ]
        );
    }

    #[test]
    fn a_malformed_frame_is_refused() {
        assert!(parse("0").is_err());
        assert!(parse("zero a.png").is_err());
        assert!(parse("0 a.png 1 2 3").is_err());
    }

    #[test]
    fn variants_come_in_threes_after_their_flag() {
        let args = |words: &[&str]| {
            words
                .iter()
                .map(|word| (*word).to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            parse_variants(&args(&["--variant", "/A", "look", "red"])).unwrap(),
            [("/A".to_owned(), "look".to_owned(), "red".to_owned())]
        );
        assert!(parse_variants(&args(&["--variant", "/A", "look"])).is_err());
        assert!(parse_variants(&args(&["--other", "/A", "look", "red"])).is_err());
    }
}
