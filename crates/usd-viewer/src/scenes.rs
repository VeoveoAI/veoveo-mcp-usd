//! The item's scenes, from the folder listings its origin answers: its top-level folders whose
//! root stage bears their name. An origin that lists nothing leaves the panel empty.
use std::sync::{Arc, Mutex};

use bevy::prelude::Resource;
use serde::Deserialize;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

/// The extensions of a root stage, in the order a folder is searched.
const ROOTS: [&str; 4] = ["usd", "usda", "usdc", "usdz"];

#[derive(Deserialize)]
struct Listing {
    entries: Vec<Entry>,
}

#[derive(Deserialize)]
struct Entry {
    name: String,
    #[serde(rename = "type")]
    kind: String,
}

/// A scene of the item.
#[derive(Clone)]
pub struct Scene {
    pub name: String,
    pub path: String,
}

/// The scenes found so far, filled while the stage loads.
#[derive(Resource, Clone, Default)]
pub struct Scenes(pub Arc<Mutex<Vec<Scene>>>);

/// Starts listing, and answers the resource that fills up.
pub fn start() -> Scenes {
    let scenes = Scenes::default();
    let filled = scenes.clone();
    wasm_bindgen_futures::spawn_local(async move {
        let Some(top) = listing("/").await else {
            return;
        };
        for entry in top.entries.iter().filter(|entry| entry.kind == "directory") {
            let encoded = String::from(js_sys::encode_uri_component(&entry.name));
            let Some(folder) = listing(&format!("/{encoded}/")).await else {
                continue;
            };
            let root = ROOTS.iter().find_map(|extension| {
                let name = format!("{}.{extension}", entry.name);
                folder
                    .entries
                    .iter()
                    .any(|file| file.kind == "file" && file.name == name)
                    .then_some(name)
            });
            if let Some(root) = root {
                filled.0.lock().expect("the scene list").push(Scene {
                    name: entry.name.clone(),
                    path: format!(
                        "/{encoded}/{}",
                        String::from(js_sys::encode_uri_component(&root))
                    ),
                });
            }
        }
    });
    scenes
}

async fn listing(path: &str) -> Option<Listing> {
    let window = web_sys::window()?;
    let response = JsFuture::from(window.fetch_with_str(path)).await.ok()?;
    let response: web_sys::Response = response.dyn_into().ok()?;
    if !response.ok() {
        return None;
    }
    let text = JsFuture::from(response.text().ok()?)
        .await
        .ok()?
        .as_string()?;
    serde_json::from_str(&text).ok()
}

/// Opens another scene: a new page, so that the memory of this one goes with it.
pub fn open(path: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let query = String::from(js_sys::encode_uri_component(path));
    let _ = window.location().set_search(&format!("?file={query}"));
}
