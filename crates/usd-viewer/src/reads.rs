//! A variant selection on its way to the stage. A selection can name files the stage has not read,
//! and in a browser composing cannot wait for a read, so the viewer reads them first, by their
//! relative paths on this origin, and then hands the stage the selection and the files together.
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use bevy::{
    asset::io::{AssetSourceId, Reader as _},
    prelude::*,
};
use usd_bevy::{UsdScene, UsdSceneRoot, UsdSource, instance::UsdInstanceOverrides};

use crate::viewer::{now_ms, tell};

/// The most rounds of reading a selection takes: each round reads what the last one's files named.
const ROUNDS: usize = 128;

/// The selection a panel asked for, and what its reads ended with.
#[derive(Resource, Default)]
pub struct VariantReads {
    /// Asked for and not on the stage yet.
    wanted: Option<UsdInstanceOverrides>,
    /// When, in the page's milliseconds.
    asked: f64,
    /// Whether the reads for `wanted` are under way.
    reading: bool,
    /// Filled when they end: the source with the files read, if any were.
    finished: Arc<Mutex<Option<Option<UsdSource>>>>,
}

impl VariantReads {
    /// Whether a selection is still on its way, so that the panel leaves it be.
    pub fn busy(&self) -> bool {
        self.wanted.is_some()
    }

    /// Asks for `overrides` on the stage.
    pub fn ask(&mut self, overrides: UsdInstanceOverrides) {
        self.wanted = Some(overrides);
        self.asked = now_ms();
    }

    /// Says how long the selection took to reach the stage.
    fn land(&mut self) {
        self.wanted = None;
        self.reading = false;
        tell(
            "selected",
            &format!(r#","ms":{:.0}"#, now_ms() - self.asked),
        );
    }
}

/// Starts the reads of a selection a panel asked for, and applies it when they end.
pub fn apply(
    mut reads: ResMut<VariantReads>,
    server: Res<AssetServer>,
    mut scenes: ResMut<Assets<UsdScene>>,
    mut roots: Query<(&UsdSceneRoot, &mut UsdInstanceOverrides)>,
) {
    let Some(wanted) = reads.wanted.clone() else {
        return;
    };
    let Some((root, mut overrides)) = roots.iter_mut().next() else {
        return;
    };
    if !reads.reading {
        let Some(scene) = scenes.get(&root.0) else {
            return;
        };
        // A package holds all it may name, and asking would compose the whole stage again.
        if scene.source.identifier().ends_with(".usdz") {
            *overrides = wanted;
            reads.land();
            return;
        }
        reads.reading = true;
        let finished = reads.finished.clone();
        let source = scene.source.clone();
        let server = server.clone();
        wasm_bindgen_futures::spawn_local(async move {
            let source = gather(source, &wanted, &server).await;
            *finished.lock().expect("a selection's reads") = Some(source);
        });
        return;
    }
    let Some(source) = reads.finished.lock().expect("a selection's reads").take() else {
        return;
    };
    if let Some(source) = source
        && let Some(mut scene) = scenes.get_mut(&root.0)
    {
        scene.source = source;
    }
    *overrides = wanted;
    reads.land();
}

/// `source` with what `wanted` names and it lacks, or `None` if it lacks nothing. A file that cannot
/// be read stays out, and the stage shows without it, as a fresh load does.
async fn gather(
    mut source: UsdSource,
    wanted: &UsdInstanceOverrides,
    server: &AssetServer,
) -> Option<UsdSource> {
    let mut read_any = false;
    for _ in 0..ROUNDS {
        let missing = source.missing_dependencies_under(wanted);
        if missing.is_empty() {
            break;
        }
        for file in missing {
            let bytes = read(server, &file).await;
            if bytes.is_none() {
                warn!("USD dependency unavailable, continuing without it: {file}");
            }
            source.add_file(&file, bytes);
            read_any = true;
        }
    }
    read_any.then_some(source)
}

async fn read(server: &AssetServer, file: &str) -> Option<Arc<[u8]>> {
    if openusd::ar::is_package_relative_path(file) {
        return None;
    }
    let source = server.get_source(AssetSourceId::Default).ok()?;
    let mut reader = source.reader().read(Path::new(file)).await.ok()?;
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).await.ok()?;
    Some(bytes.into())
}
