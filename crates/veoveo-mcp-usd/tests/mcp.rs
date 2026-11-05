//! The MCP surface through the SDK's in-process Hub: discovery, permission refusals, the App, the
//! viewer's declaration and the skill's manifest. The test user is `user-1` of `tenant-a`. Reads of
//! a stage need a Hub with Lore behind it, which the deployed probes cover.
use std::time::Duration;

use serde_json::{Value, json};
use veoveo_hub_sdk::{
    apps::AppHtml,
    hosting::{
        Hosted,
        testing::{self, TestHub},
    },
};
use veoveo_mcp_usd::{
    contract::UsdPermission::{self, StagesRead, StagesRender},
    hub::Hub,
    mcp::UsdMcp,
    thumbnail::Thumbnails,
};

fn app_html() -> AppHtml {
    AppHtml::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/scenes.html"))
        .expect("the App's document")
}

/// A Hub that grants `granted` in front of the server. Nothing behind it answers a Hub route.
fn hub(granted: &[UsdPermission]) -> TestHub {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let hub = Hub::new(
        &url::Url::parse("http://127.0.0.1:9/").unwrap(),
        reqwest::Client::builder().build().unwrap(),
    )
    .unwrap();
    let app = app_html();
    let thumbnails = Thumbnails::new(std::env::temp_dir().join("usd-mcp-test-thumbnails"))
        .expect("the thumbnails' folder");
    TestHub::new(
        testing::for_domain::<UsdMcp>()
            .handler(move || Hosted::new(UsdMcp::new(hub.clone(), app.clone(), thumbnails.clone())))
            .build(),
    )
    .granting(granted.iter().map(|permission| permission.name()))
}

#[tokio::test]
async fn tools_are_listed_with_their_permissions() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let full = hub(&[StagesRead, StagesRender]);
        let tools = full.rpc("tools/list", json!({})).await;
        let mut names: Vec<&str> = tools["result"]["tools"]
            .as_array()
            .unwrap_or_else(|| panic!("{tools}"))
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        names.sort_unstable();
        assert_eq!(
            names,
            ["inspect_stage", "list_scenes", "render_stage", "thumbnail"]
        );

        // Reading does not let you render.
        let reader = hub(&[StagesRead]);
        let listed = reader.rpc("tools/list", json!({})).await;
        let mut names: Vec<&str> = listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        names.sort_unstable();
        assert_eq!(names, ["inspect_stage", "list_scenes", "thumbnail"]);

        // With nothing granted, nothing is admitted.
        let nothing = hub(&[]);
        let refused = nothing
            .rpc(
                "tools/call",
                json!({"name": "list_scenes", "arguments": {}}),
            )
            .await;
        assert!(refused["error"].is_object(), "{refused}");
    })
    .await
    .expect("the test finished in time");
}

#[tokio::test]
async fn a_bad_stage_path_is_refused_before_any_read() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let reader = hub(&[StagesRead]);
        for path in ["../a.usd", "/a.usd", "a.png"] {
            let refused = reader
                .rpc(
                    "tools/call",
                    json!({"name": "inspect_stage", "arguments": {
                        "repository": "r", "branch": "main", "path": path,
                    }}),
                )
                .await;
            assert_eq!(refused["result"]["isError"], true, "{path}: {refused}");
        }
    })
    .await
    .expect("the test finished in time");
}

#[tokio::test]
async fn the_app_lists_scenes_through_the_bridge_and_opens_them_with_a_link() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let reader = hub(&[StagesRead]);
        let resources = reader.rpc("resources/list", json!({})).await;
        let app = resources["result"]["resources"]
            .as_array()
            .unwrap_or_else(|| panic!("{resources}"))
            .iter()
            .find(|resource| resource["uri"] == "ui://usd/scenes")
            .expect("the App is listed");
        assert_eq!(app["mimeType"], "text/html;profile=mcp-app");
        let document = reader
            .rpc("resources/read", json!({"uri": "ui://usd/scenes"}))
            .await;
        let html = document["result"]["contents"][0]["text"]
            .as_str()
            .unwrap_or_else(|| panic!("{document}"));
        for needle in [
            "ui/initialize",
            "list_scenes",
            "ui/open-link",
            "\"view\", \"lore\"",
        ] {
            assert!(html.contains(needle), "{needle}");
        }
        let tools = reader.rpc("tools/list", json!({})).await;
        let list = tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == "list_scenes")
            .unwrap();
        assert_eq!(list["_meta"]["ui"]["resourceUri"], "ui://usd/scenes");
    })
    .await
    .expect("the test finished in time");
}

/// Every file under `dir`, as its path relative to `root`, `/`-separated.
fn files(root: &std::path::Path, dir: &std::path::Path, found: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files(root, &path, found);
        } else {
            let relative = path.strip_prefix(root).unwrap();
            found.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
}

fn as_json(response: &Value) -> Value {
    response["result"].clone()
}

#[tokio::test]
async fn the_skills_manifest_names_every_file_with_its_digest() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let hub = hub(&[StagesRead]);
        let listed = as_json(&hub.rpc("skills/list", json!({})).await);
        let skills = listed["skills"]
            .as_array()
            .unwrap_or_else(|| panic!("{listed}"));
        assert_eq!(skills.len(), 1);
        let skill = &skills[0];
        assert_eq!(skill["uri"], "skill://usd/view-stages/SKILL.md");
        assert_eq!(skill["frontmatter"]["name"], "view-stages");

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("skills/view-stages");
        let mut on_disk = Vec::new();
        files(&root, &root, &mut on_disk);
        on_disk.sort();
        let manifest = skill["resources"].as_array().expect("a full manifest");
        let mut listed: Vec<String> = manifest
            .iter()
            .map(|file| {
                file["uri"]
                    .as_str()
                    .unwrap()
                    .strip_prefix("skill://usd/view-stages/")
                    .unwrap()
                    .to_owned()
            })
            .collect();
        listed.sort();
        assert_eq!(listed, on_disk);
        for file in manifest {
            let read = hub.rpc("resources/read", json!({"uri": file["uri"]})).await;
            let text = read["result"]["contents"][0]["text"]
                .as_str()
                .unwrap_or_else(|| panic!("{read}"));
            assert_eq!(text.len() as u64, file["size"].as_u64().unwrap());
            assert_eq!(
                rmcp::model::sha256_digest(text.as_bytes()),
                file["digest"].as_str().unwrap()
            );
        }
    })
    .await
    .expect("the test finished in time");
}
