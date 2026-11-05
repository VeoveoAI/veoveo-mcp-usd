//! Loads the stage its page's `file` query names, by its path on the page's own origin, frames
//! it, orbits it, and shows its panels, on WebGPU. It tells its parent page how it is doing with
//! `{"usdViewer": ...}` messages.

use bevy::{
    asset::AssetMetaCheck,
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    light::CascadeShadowConfigBuilder,
    prelude::*,
    render::renderer::RenderAdapterInfo,
    window::WindowResolution,
};
use bevy_egui::{
    EguiPlugin, EguiPrimaryContextPass, input::EguiWantsInput, render::EGUI_SHADER_HANDLE,
};
use usd_bevy::{
    DisplayPurposes, UsdAssetPlugin, UsdPlugin, UsdScene, UsdSceneRoot, UsdSceneState,
    instance::{UsdInstanceOverrides, UsdInstanceTime, UsdPlayback},
    mesh::bounds::StageBounds,
    route::{
        dome_environment::UsdDomeEnvironmentPlugin,
        gpu_instancing::{UsdGpuInstancing, UsdGpuInstancingPlugin},
        gpu_skin::UsdGpuSkinningPlugin,
    },
};
use wasm_bindgen::{JsCast, JsValue};

use crate::{
    panels::{self, CameraChoice, Facts, Selection},
    scenes,
};

/// Where the framed camera starts, as yaw and pitch around the stage's center.
const VIEW: (f32, f32) = (0.6, -0.45);
/// How far the framed camera stands, in the stage's largest extents: Bevy's default field of view
/// shows a whole extent from 1.21 of them, with a margin here for its depth.
const FRAMED_DISTANCE: f32 = 1.4;
/// How long the viewer counts frames, once the stage is ready, before reporting a rate.
const MEASURE_SECONDS: f32 = 5.0;
/// Frames drawn after a new time before the viewer says it shows: one to sample, one to draw.
const SEEK_FRAMES: u32 = 2;

/// The stage to load.
#[derive(Resource)]
struct Scene(String);

/// Whether the stage has been framed, and how fast it has drawn since.
#[derive(Resource, Default)]
struct Progress {
    framed: bool,
    frames: u32,
    measured: f32,
    reported: bool,
}

/// A new time on its way to the screen.
#[derive(Resource, Default)]
pub struct Seek {
    /// When the panel asked for it, in the page's milliseconds.
    pub asked: Option<f64>,
    pub frames: u32,
}

/// Drag to orbit around `focus`, scroll to zoom.
#[derive(Component)]
pub struct Orbit {
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
    pub range: (f32, f32),
    pub focus: Vec3,
}

pub fn run() {
    let scene = query_file();
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(AssetPlugin {
                file_path: "/".into(),
                meta_check: AssetMetaCheck::Never,
                ..default()
            })
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "USD viewer".into(),
                    resolution: WindowResolution::default(),
                    fit_canvas_to_parent: true,
                    ..default()
                }),
                ..default()
            }),
    )
    .add_plugins((
        UsdPlugin,
        UsdAssetPlugin,
        UsdGpuSkinningPlugin,
        UsdDomeEnvironmentPlugin,
        UsdGpuInstancingPlugin,
        EguiPlugin {
            bindless_mode_array_size: None,
        },
    ))
    .init_resource::<UsdGpuInstancing>()
    .init_resource::<Progress>()
    .init_resource::<Seek>()
    .init_resource::<Facts>()
    .init_resource::<Selection>()
    .init_resource::<CameraChoice>()
    .insert_resource(scenes::start())
    .insert_resource(DisplayPurposes {
        render: true,
        proxy: false,
        guide: false,
    })
    .insert_resource(Scene(scene))
    .add_systems(Startup, setup)
    .add_systems(
        Update,
        (
            report_state,
            frame_stage,
            orbit,
            panels::read_facts,
            panels::look_through,
        ),
    )
    .add_systems(EguiPrimaryContextPass, panels::show)
    .add_systems(Last, (measure, report_seek));
    app.world_mut()
        .resource_mut::<Assets<Shader>>()
        .insert(
            EGUI_SHADER_HANDLE.id(),
            Shader::from_wgsl(include_str!("egui.wgsl"), "usd-viewer/egui.wgsl"),
        )
        .expect("the panels' shader has a stable handle");
    app.run();
}

/// The `file` query of the page's URL: the path of a file on this origin, percent-encoded as a
/// URL's path is, without its leading `/`. Bevy asks for an asset by this path as it stands.
fn query_file() -> String {
    web_sys::window()
        .and_then(|window| window.location().search().ok())
        .and_then(|search| web_sys::UrlSearchParams::new_with_str(&search).ok())
        .and_then(|params| params.get("file"))
        .unwrap_or_default()
        .trim_start_matches('/')
        .to_owned()
}

pub fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map_or(0.0, |performance| performance.now())
}

/// The bytes of the viewer's WASM memory.
pub fn memory_bytes() -> f64 {
    wasm_bindgen::memory()
        .dyn_into::<js_sys::WebAssembly::Memory>()
        .ok()
        .and_then(|memory| memory.buffer().dyn_into::<js_sys::ArrayBuffer>().ok())
        .map_or(0.0, |buffer| f64::from(buffer.byte_length()))
}

/// Tells the parent page, and the console, how the viewer is doing. `fields` is JSON.
pub fn tell(state: &str, fields: &str) {
    let message = format!(
        r#"{{"usdViewer":{{"state":"{state}","atMs":{:.0},"wasmMemoryBytes":{:.0}{fields}}}}}"#,
        now_ms(),
        memory_bytes()
    );
    info!("{message}");
    if let Some(Ok(Some(parent))) = web_sys::window().map(|window| window.parent()) {
        let _ = parent.post_message(&JsValue::from_str(&message), "*");
    }
}

fn setup(
    mut commands: Commands,
    assets: Res<AssetServer>,
    scene: Res<Scene>,
    adapter: Res<RenderAdapterInfo>,
) {
    tell(
        "started",
        &format!(
            r#","backend":"{:?}","adapter":"{}","file":{:?}"#,
            adapter.backend, adapter.name, scene.0
        ),
    );
    commands.spawn((
        Camera3d::default(),
        Transform::default(),
        Orbit {
            yaw: VIEW.0,
            pitch: VIEW.1,
            distance: 10.0,
            range: (1.0, 200.0),
            focus: Vec3::ZERO,
        },
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
    let stage: Handle<UsdScene> = assets.load(scene.0.clone());
    commands.spawn((
        UsdSceneRoot(stage),
        UsdInstanceTime::default(),
        UsdPlayback::default(),
        UsdInstanceOverrides::default(),
    ));
}

fn report_state(roots: Query<&UsdSceneState, Changed<UsdSceneState>>, mut told: Local<bool>) {
    for state in &roots {
        match state {
            UsdSceneState::Failed(error) => {
                error!("{error}");
                tell("failed", &format!(r#","error":{error:?}"#));
            }
            UsdSceneState::Loading if !*told => {
                *told = true;
                tell("loading", "");
            }
            UsdSceneState::Loading | UsdSceneState::Ready => (),
        }
    }
}

/// Frames the stage once it is ready, and fits the clip planes and shadows to it.
fn frame_stage(
    mut progress: ResMut<Progress>,
    states: Query<&UsdSceneState>,
    stage: StageBounds,
    mut cameras: Query<(&mut Orbit, &mut Projection)>,
    lights: Query<Entity, With<DirectionalLight>>,
    images: Res<Assets<Image>>,
    mut commands: Commands,
) {
    if progress.framed || !matches!(states.iter().next(), Some(UsdSceneState::Ready)) {
        return;
    }
    let Some((low, high)) = stage.get() else {
        return;
    };
    let span = (high - low).max_element();
    for (mut orbit, mut projection) in &mut cameras {
        orbit.focus = (low + high) * 0.5;
        orbit.distance = span * FRAMED_DISTANCE;
        orbit.range = (span * 0.02, span * 8.0);
        if let Projection::Perspective(perspective) = projection.as_mut() {
            perspective.far = span * 16.0;
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
    progress.framed = true;
    tell(
        "ready",
        &format!(
            r#","draws":{},"instances":{},"span":{span},"images":{},"imageBytes":{}"#,
            stage.draws(),
            stage.instances(),
            images.len(),
            images
                .iter()
                .filter_map(|(_, image)| image.data.as_ref())
                .map(Vec::len)
                .sum::<usize>(),
        ),
    );
}

fn orbit(
    mut cameras: Query<(&mut Orbit, &mut Transform)>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    wants: Res<EguiWantsInput>,
    mut choice: ResMut<CameraChoice>,
) {
    if wants.wants_any_pointer_input() {
        return;
    }
    let notches = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / 100.0,
    };
    let dragged = buttons.pressed(MouseButton::Left) && motion.delta != Vec2::ZERO;
    if dragged || notches != 0.0 {
        choice.leave();
    }
    if choice.looking() {
        return;
    }
    for (mut orbit, mut transform) in &mut cameras {
        if buttons.pressed(MouseButton::Left) {
            orbit.yaw -= motion.delta.x * 0.005;
            orbit.pitch = (orbit.pitch - motion.delta.y * 0.005).clamp(-1.5, 1.5);
        }
        orbit.distance =
            (orbit.distance * (1.0 - notches * 0.1)).clamp(orbit.range.0, orbit.range.1);
        let rotation = Quat::from_euler(EulerRot::YXZ, orbit.yaw, orbit.pitch, 0.0);
        let eye = orbit.focus + rotation * Vec3::new(0.0, 0.0, orbit.distance);
        *transform = Transform::from_translation(eye).looking_at(orbit.focus, Vec3::Y);
    }
}

/// Reports the frame rate once, after the stage has been drawing for a while.
fn measure(time: Res<Time>, mut progress: ResMut<Progress>) {
    if !progress.framed || progress.reported {
        return;
    }
    progress.frames += 1;
    progress.measured += time.delta_secs();
    if progress.measured >= MEASURE_SECONDS {
        progress.reported = true;
        tell(
            "measured",
            &format!(
                r#","fps":{:.1}"#,
                progress.frames as f32 / progress.measured
            ),
        );
    }
}

/// Reports how long a new time took to show: from the panel's asking until it has been drawn.
fn report_seek(mut seek: ResMut<Seek>, roots: Query<&UsdInstanceTime>) {
    let Some(asked) = seek.asked else {
        return;
    };
    seek.frames += 1;
    if seek.frames < SEEK_FRAMES {
        return;
    }
    seek.asked = None;
    seek.frames = 0;
    let time = roots.iter().next().map_or(0.0, |time| time.current);
    tell(
        "seeked",
        &format!(r#","time":{time},"ms":{:.0}"#, now_ms() - asked),
    );
}
