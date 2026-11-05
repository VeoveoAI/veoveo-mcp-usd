//! The panels over the stage: scenes, prims, variants, payloads, cameras and time. They read the
//! live stage, and change it through the same components a program would.
use bevy::{camera::Projection, prelude::*};
use bevy_egui::{EguiContexts, egui};
use openusd::{sdf::Path, usd::Stage};
use usd_bevy::{
    UsdPrimRef, UsdScene, UsdSceneInstance, UsdSceneRoot, UsdSceneState,
    instance::{UsdInstanceOverrides, UsdInstanceTime, UsdInstances, UsdPlayback},
    read::variants::{variant_options, variant_selections},
    route::camera::UsdCamera,
};

use crate::{
    reads::VariantReads,
    scenes::{self, Scenes},
    viewer::{Seek, memory_bytes, now_ms},
};

/// The most rows a list shows; a tree shows this many children of a prim.
const MAX_ROWS: usize = 400;
/// A list longer than this starts closed: a closed list costs its frame nothing, and the kitchen's
/// 392 variant sets and 425 payloads open took it from 118 frames a second to 71.
const OPEN_ROWS: usize = 16;
/// How many stage time codes one step of the time buttons moves.
const STEP: f64 = 1.0;
/// The widest the slider's value and label are, beside it.
const TIME_FIELDS: f32 = 120.0;
/// The most decimals the time shows.
const TIME_DECIMALS: usize = 2;

/// One variant set on one prim.
pub struct VariantRow {
    prim: String,
    set: String,
    selection: String,
    options: Vec<String>,
}

/// What the panels read from the stage once, and again after it changes.
#[derive(Resource, Default)]
pub struct Facts {
    read: bool,
    /// Set when a panel changed which payloads the stage holds.
    stale: bool,
    start: f64,
    end: f64,
    rate: f64,
    prims: usize,
    variants: Vec<VariantRow>,
    /// Prims with a payload.
    payloads: Vec<String>,
    payloads_read: bool,
}

/// Whether the pointer is over a panel, so that dragging and scrolling leave the stage alone.
#[derive(Resource, Default)]
pub struct OverPanels(pub bool);

/// The prim the tree has selected.
#[derive(Resource, Default)]
pub struct Selection(Option<String>);

/// The stage camera the view looks through, if any.
#[derive(Resource, Default)]
pub struct CameraChoice {
    looking: Option<Entity>,
    /// The free camera's projection while it looks through another.
    free: Option<Projection>,
    /// Set when the free camera should take its projection back.
    leaving: bool,
}

impl CameraChoice {
    pub fn looking(&self) -> bool {
        self.looking.is_some()
    }

    /// Back to the free camera.
    pub fn leave(&mut self) {
        if self.looking.take().is_some() {
            self.leaving = true;
        }
    }
}

/// Reads the stage's facts when it is ready, and after its composition changes: a variant switch
/// marks the root's instance when it lands, whichever of the two runs first in a frame.
pub fn read_facts(
    mut facts: ResMut<Facts>,
    instances: NonSend<UsdInstances>,
    roots: Query<(Entity, &UsdSceneState, &UsdSceneRoot, Ref<UsdSceneInstance>)>,
    scenes: Res<Assets<UsdScene>>,
) {
    let Some((root, UsdSceneState::Ready, handle, instance)) = roots.iter().next() else {
        return;
    };
    if facts.read && !facts.stale && !instance.is_changed() {
        return;
    }
    let Some(stage) = instances.stage(root) else {
        return;
    };
    facts.read = true;
    facts.stale = false;
    facts.start = stage.start_time_code();
    facts.end = stage.end_time_code();
    facts.rate = stage.time_codes_per_second();
    facts.prims = 0;
    facts.variants.clear();
    let mut paths = Vec::new();
    let _ = stage.traverse(traversal(), |path| paths.push(path.clone()));
    facts.prims = paths.len();
    for path in &paths {
        if facts.variants.len() >= MAX_ROWS {
            break;
        }
        for (set, selection) in variant_selections(stage, path).unwrap_or_default() {
            let options = variant_options(stage, path, &set);
            facts.variants.push(VariantRow {
                prim: path.to_string(),
                set,
                selection,
                options,
            });
        }
    }
    // A stage opened with its payloads unloaded shows which prims have one, once.
    if !facts.payloads_read {
        facts.payloads_read = true;
        if let Some(scene) = scenes.get(&handle.0)
            && let Ok(unloaded) = scene.source.open_stage_unloaded()
        {
            let mut held = Vec::new();
            let _ = unloaded.traverse(traversal(), |path| held.push(path.clone()));
            facts.payloads = held
                .into_iter()
                .filter(|path| {
                    unloaded
                        .prim(path.clone())
                        .is_ok_and(|prim| !prim.is_loaded().unwrap_or(true))
                })
                .take(MAX_ROWS)
                .map(|path| path.to_string())
                .collect();
        }
    }
}

fn traversal() -> openusd::usd::PrimPredicate {
    use openusd::usd::{PrimPredicate, PrimStatus};
    PrimPredicate::new(
        PrimStatus::ACTIVE.union(PrimStatus::DEFINED),
        PrimStatus::ABSTRACT,
    )
}

/// The stage's cameras, apart from the camera that draws.
type StageCameras<'w, 's> = Query<
    'w,
    's,
    (&'static GlobalTransform, &'static Projection),
    (With<UsdCamera>, Without<Camera3d>),
>;

/// While a stage camera is chosen, the view stays on it, as it moves with time.
pub fn look_through(
    mut choice: ResMut<CameraChoice>,
    stage_cameras: StageCameras,
    mut view: Query<(&mut Transform, &mut Projection), With<Camera3d>>,
) {
    let Ok((mut transform, mut projection)) = view.single_mut() else {
        return;
    };
    if choice.leaving {
        choice.leaving = false;
        if let Some(free) = choice.free.take() {
            *projection = free;
        }
        return;
    }
    let Some(looking) = choice.looking else {
        return;
    };
    let Ok((placed, own)) = stage_cameras.get(looking) else {
        choice.leave();
        return;
    };
    if choice.free.is_none() {
        choice.free = Some(projection.clone());
    }
    *transform = placed.compute_transform();
    *projection = own.clone();
}

#[expect(
    clippy::too_many_arguments,
    reason = "a Bevy system names what it reads"
)]
pub fn show(
    mut contexts: EguiContexts,
    mut facts: ResMut<Facts>,
    mut selection: ResMut<Selection>,
    mut choice: ResMut<CameraChoice>,
    mut seek: ResMut<Seek>,
    mut over: ResMut<OverPanels>,
    mut reads: ResMut<VariantReads>,
    scene_list: Res<Scenes>,
    instances: NonSend<UsdInstances>,
    time: Res<Time>,
    mut roots: Query<(
        Entity,
        &UsdSceneState,
        &mut UsdInstanceTime,
        &mut UsdPlayback,
        &UsdInstanceOverrides,
    )>,
    cameras: Query<(Entity, &UsdPrimRef), With<UsdCamera>>,
) -> Result {
    let context = contexts.ctx_mut()?;
    let mut viewport = egui::Ui::new(
        context.clone(),
        "viewport".into(),
        egui::UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(context.viewport_rect()),
    );
    let root = roots.iter_mut().next();
    let stage = root
        .as_ref()
        .and_then(|(entity, ..)| instances.stage(*entity));
    let pointer = context.pointer_latest_pos();
    over.0 = false;
    let left = egui::Panel::left("stage")
        .default_size(280.0)
        .show(&mut viewport, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                section(ui, "Scenes", true, |ui| show_scenes(ui, &scene_list));
                if let Some(stage) = stage {
                    section(ui, format!("Prims ({})", facts.prims), true, |ui| {
                        let top = Path::abs_root();
                        tree(ui, stage, &top, &mut selection);
                        if let Some(selected) = &selection.0 {
                            ui.separator();
                            ui.small(selected.as_str());
                        }
                    });
                }
            });
        });
    over.0 |= pointer.is_some_and(|pointer| left.response.rect.contains(pointer));
    let Some((entity, state, mut current, mut playback, overrides)) = root else {
        return Ok(());
    };
    let right = egui::Panel::right("composition")
        .default_size(280.0)
        .show(&mut viewport, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                section(
                    ui,
                    format!("Variants ({})", facts.variants.len()),
                    facts.variants.len() <= OPEN_ROWS,
                    |ui| show_variants(ui, &facts, overrides, &mut reads),
                );
                section(
                    ui,
                    format!("Payloads ({})", facts.payloads.len()),
                    facts.payloads.len() <= OPEN_ROWS,
                    |ui| {
                        if show_payloads(ui, &facts, stage, &instances, entity) {
                            facts.stale = true;
                        }
                    },
                );
                section(ui, "Cameras", true, |ui| {
                    show_cameras(ui, &cameras, &mut choice)
                });
            });
        });
    over.0 |= pointer.is_some_and(|pointer| right.response.rect.contains(pointer));
    let bottom = egui::Panel::bottom("time").show(&mut viewport, |ui| {
        ui.horizontal(|ui| {
            let moving = facts.end > facts.start;
            let label = if playback.playing { "Pause" } else { "Play" };
            if ui.add_enabled(moving, egui::Button::new(label)).clicked() {
                playback.playing = !playback.playing;
            }
            let mut asked = current.current;
            if ui.add_enabled(moving, egui::Button::new("<")).clicked() {
                asked = (asked - STEP).max(facts.start);
            }
            if ui.add_enabled(moving, egui::Button::new(">")).clicked() {
                asked = (asked + STEP).min(facts.end);
            }
            let status = match &*state {
                UsdSceneState::Loading => "loading".to_owned(),
                UsdSceneState::Failed(error) => format!("failed: {error}"),
                UsdSceneState::Ready => format!(
                    "{:.0} fps, {:.0} MB",
                    1.0 / time.delta_secs().max(0.001),
                    memory_bytes() / 1_048_576.0
                ),
            };
            let status_width = ui
                .painter()
                .layout_no_wrap(
                    status.clone(),
                    egui::TextStyle::Body.resolve(ui.style()),
                    egui::Color32::PLACEHOLDER,
                )
                .size()
                .x;
            ui.spacing_mut().slider_width =
                (ui.available_width() - status_width - TIME_FIELDS).max(120.0);
            ui.add_enabled(
                moving,
                egui::Slider::new(&mut asked, facts.start..=facts.end)
                    .max_decimals(TIME_DECIMALS)
                    .text("time"),
            );
            if asked != current.current {
                playback.playing = false;
                current.current = asked;
                if seek.asked.is_none() {
                    seek.asked = Some(now_ms());
                    seek.frames = 0;
                }
            }
            ui.label(status);
        });
    });
    over.0 |= pointer.is_some_and(|pointer| bottom.response.rect.contains(pointer));
    Ok(())
}

fn section(
    ui: &mut egui::Ui,
    title: impl Into<egui::WidgetText>,
    open: bool,
    body: impl FnOnce(&mut egui::Ui),
) {
    egui::CollapsingHeader::new(title)
        .default_open(open)
        .show(ui, body);
}

fn show_scenes(ui: &mut egui::Ui, scenes: &Scenes) {
    let current = web_sys::window()
        .and_then(|window| window.location().search().ok())
        .and_then(|search| web_sys::UrlSearchParams::new_with_str(&search).ok())
        .and_then(|params| params.get("file"))
        .unwrap_or_default();
    let listed = scenes.0.lock().expect("the scene list");
    if listed.is_empty() {
        ui.small("No other scenes here.");
    }
    for scene in listed.iter() {
        if ui
            .selectable_label(scene.path == current, scene.name.as_str())
            .clicked()
            && scene.path != current
        {
            scenes::open(&scene.path);
        }
    }
}

/// One level of the stage's tree; a closed prim reads nothing below it.
fn tree(ui: &mut egui::Ui, stage: &Stage, path: &Path, selection: &mut Selection) {
    let Ok(prim) = stage.prim(path.clone()) else {
        return;
    };
    let names = prim.child_names().unwrap_or_default();
    for name in names.iter().take(MAX_ROWS) {
        let Ok(child) = path.append_path(name.as_str()) else {
            continue;
        };
        let Ok(entry) = stage.prim(child.clone()) else {
            continue;
        };
        let type_name = entry
            .type_name()
            .ok()
            .flatten()
            .map(|name| name.to_string())
            .unwrap_or_default();
        let selected = selection.0.as_deref() == Some(child.as_str());
        let label = if type_name.is_empty() {
            name.to_string()
        } else {
            format!("{name}  ({type_name})")
        };
        let has_children = entry.child_names().is_ok_and(|names| !names.is_empty());
        if has_children {
            let id = ui.make_persistent_id(child.as_str());
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
                .show_header(ui, |ui| {
                    if ui.selectable_label(selected, label).clicked() {
                        selection.0 = Some(child.to_string());
                    }
                })
                .body(|ui| tree(ui, stage, &child, selection));
        } else if ui.selectable_label(selected, label).clicked() {
            selection.0 = Some(child.to_string());
        }
    }
    if names.len() > MAX_ROWS {
        ui.small(format!("and {} more", names.len() - MAX_ROWS));
    }
}

/// Each variant set, whose choice goes to the reads ahead of the stage.
fn show_variants(
    ui: &mut egui::Ui,
    facts: &Facts,
    overrides: &UsdInstanceOverrides,
    reads: &mut VariantReads,
) {
    if facts.variants.is_empty() {
        ui.small("This stage has no variant sets.");
    }
    if reads.busy() {
        ui.small("Reading the selection's files.");
    }
    ui.add_enabled_ui(!reads.busy(), |ui| {
        show_variant_rows(ui, facts, overrides, reads)
    });
}

fn show_variant_rows(
    ui: &mut egui::Ui,
    facts: &Facts,
    overrides: &UsdInstanceOverrides,
    reads: &mut VariantReads,
) {
    for row in &facts.variants {
        ui.small(row.prim.as_str());
        let mut selected = row.selection.clone();
        egui::ComboBox::from_id_salt((row.prim.as_str(), row.set.as_str()))
            .selected_text(selected.as_str())
            .show_ui(ui, |ui| {
                for option in &row.options {
                    ui.selectable_value(&mut selected, option.clone(), option.as_str());
                }
            });
        ui.label(row.set.as_str());
        if selected != row.selection {
            let mut wanted = overrides.clone();
            wanted
                .variants
                .retain(|(prim, set, _)| *prim != row.prim || *set != row.set);
            wanted
                .variants
                .push((row.prim.clone(), row.set.clone(), selected));
            reads.ask(wanted);
        }
    }
}

/// Whether a payload was loaded or unloaded.
fn show_payloads(
    ui: &mut egui::Ui,
    facts: &Facts,
    stage: Option<&Stage>,
    instances: &UsdInstances,
    root: Entity,
) -> bool {
    let mut changed = false;
    if facts.payloads.is_empty() {
        ui.small("This stage has no payloads.");
    }
    let Some(stage) = stage else {
        return false;
    };
    for payload in &facts.payloads {
        let Ok(path) = openusd::sdf::path(payload) else {
            continue;
        };
        let mut loaded = stage
            .prim(path)
            .is_ok_and(|prim| prim.is_loaded().unwrap_or(true));
        if ui.checkbox(&mut loaded, payload.as_str()).changed() {
            if loaded {
                instances.load_payload(root, payload);
            } else {
                instances.unload_payload(root, payload);
            }
            changed = true;
        }
    }
    changed
}

fn show_cameras(
    ui: &mut egui::Ui,
    cameras: &Query<(Entity, &UsdPrimRef), With<UsdCamera>>,
    choice: &mut CameraChoice,
) {
    if ui
        .selectable_label(!choice.looking(), "Free orbit")
        .clicked()
    {
        choice.leave();
    }
    let mut listed: Vec<_> = cameras.iter().collect();
    listed.sort_by(|a, b| a.1.path.cmp(&b.1.path));
    for (entity, prim) in listed.into_iter().take(MAX_ROWS) {
        if ui
            .selectable_label(choice.looking == Some(entity), prim.path.as_str())
            .clicked()
        {
            choice.looking = Some(entity);
        }
    }
}
