//! Loads the stage its page's `scene` query names, by its path on the page's own origin,
//! frames it and orbits it, on WebGPU. It tells its parent page how it is doing with
//! `{"usdViewer": ...}` messages.

use bevy::{
    asset::AssetMetaCheck,
    input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit},
    light::CascadeShadowConfigBuilder,
    prelude::*,
    render::renderer::RenderAdapterInfo,
    window::WindowResolution,
};
use usd_bevy::{
    DisplayPurposes, UsdAssetPlugin, UsdPlugin, UsdScene, UsdSceneRoot, UsdSceneState,
    instance::UsdInstanceTime,
    mesh::bounds::StageBounds,
    route::{
        dome_environment::UsdDomeEnvironmentPlugin,
        gpu_instancing::{UsdGpuInstancing, UsdGpuInstancingPlugin},
        gpu_skin::UsdGpuSkinningPlugin,
    },
};
use wasm_bindgen::{JsCast, JsValue};

/// Where the framed camera starts, as yaw and pitch around the stage's center.
const VIEW: (f32, f32) = (0.6, -0.45);
/// How long the viewer counts frames, once the stage is ready, before reporting a rate.
const MEASURE_SECONDS: f32 = 5.0;

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

/// Drag to orbit around `focus`, scroll to zoom.
#[derive(Component)]
struct Orbit {
    yaw: f32,
    pitch: f32,
    distance: f32,
    range: (f32, f32),
    focus: Vec3,
}

pub fn run() {
    let scene = query_scene();
    App::new()
        .add_plugins(
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
        ))
        .init_resource::<UsdGpuInstancing>()
        .init_resource::<Progress>()
        .insert_resource(DisplayPurposes {
            render: true,
            proxy: false,
            guide: false,
        })
        .insert_resource(Scene(scene))
        .add_systems(Startup, setup)
        .add_systems(Update, (report_state, frame_stage, orbit, measure))
        .run();
}

/// The `scene` query of the page's URL.
fn query_scene() -> String {
    web_sys::window()
        .and_then(|window| window.location().search().ok())
        .and_then(|search| web_sys::UrlSearchParams::new_with_str(&search).ok())
        .and_then(|params| params.get("scene"))
        .unwrap_or_default()
}

fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map_or(0.0, |performance| performance.now())
}

/// The bytes of the viewer's WASM memory.
fn memory_bytes() -> f64 {
    wasm_bindgen::memory()
        .dyn_into::<js_sys::WebAssembly::Memory>()
        .ok()
        .and_then(|memory| memory.buffer().dyn_into::<js_sys::ArrayBuffer>().ok())
        .map_or(0.0, |buffer| f64::from(buffer.byte_length()))
}

/// Tells the parent page, and the console, how the viewer is doing. `fields` is JSON.
fn tell(state: &str, fields: &str) {
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
            r#","backend":"{:?}","adapter":"{}","scene":"{}""#,
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
    commands.spawn((UsdSceneRoot(stage), UsdInstanceTime::default()));
}

fn report_state(roots: Query<&UsdSceneState, Changed<UsdSceneState>>) {
    for state in &roots {
        match state {
            UsdSceneState::Failed(error) => {
                error!("{error}");
                tell("failed", &format!(r#","error":{error:?}"#));
            }
            UsdSceneState::Loading => tell("loading", ""),
            UsdSceneState::Ready => (),
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
        orbit.distance = span * 1.1;
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
            r#","draws":{},"instances":{},"span":{span}"#,
            stage.draws(),
            stage.instances()
        ),
    );
}

fn orbit(
    mut cameras: Query<(&mut Orbit, &mut Transform)>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
) {
    let notches = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / 100.0,
    };
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
