//! The MCP surface through the SDK's in-process Hub against a migrated database:
//! discovery, permission refusals, `create_item` and the reads of its product. The test
//! user is `user-1` of `tenant-a`. Requires the PostgreSQL 18 server programs on `PATH`.
use std::time::Duration;

use serde_json::{Value, json};
use veoveo_fixtures::store::TestDb;
use veoveo_hub_sdk::hosting::{
    Hosted,
    testing::{self, TestHub},
};
use veoveo_mcp_usd::{
    contract::UsdPermission::{self, ItemsRead, ItemsWrite},
    mcp::UsdMcp,
    migrator, server,
    store::ItemStore,
};

/// A Hub that grants `granted` in front of a server on `db`.
async fn hub(db: &TestDb, granted: &[UsdPermission]) -> TestHub {
    let items = ItemStore::new(db.server_store(&server()).await);
    TestHub::new(
        testing::for_domain::<UsdMcp>()
            .handler(move || Hosted::new(UsdMcp::new(items.clone())))
            .build(),
    )
    .granting(granted.iter().map(|permission| permission.name()))
}

fn error_message(response: &Value) -> &str {
    response["error"]["message"].as_str().unwrap_or_default()
}

fn read_json(response: &Value) -> Value {
    let text = response["result"]["contents"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("no contents: {response}"));
    serde_json::from_str(text).unwrap()
}

#[tokio::test]
async fn items_are_created_read_and_listed_with_their_permissions() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let db = TestDb::new().await;
        db.migrate_server(&server(), migrator()).await;
        let full = hub(&db, &[ItemsRead, ItemsWrite]).await;

        let tools = full.rpc("tools/list", json!({})).await;
        let names: Vec<&str> = tools["result"]["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();
        assert_eq!(names, ["create_item"]);

        let created = full
            .rpc(
                "tools/call",
                json!({"name": "create_item", "arguments": {"name": "Quarterly report"}}),
            )
            .await;
        let product = &created["result"]["structuredContent"];
        let result_uri = product["resultUri"].as_str().expect("resultUri").to_owned();
        assert!(result_uri.starts_with("usd://item/"), "{created}");
        assert_eq!(product["metadata"]["item"]["name"], "Quarterly report");
        assert_eq!(created["result"]["content"][0]["uri"], result_uri);
        let text: Value =
            serde_json::from_str(created["result"]["content"][1]["text"].as_str().unwrap())
                .unwrap();
        assert_eq!(&text, product);

        let item = read_json(&full.rpc("resources/read", json!({"uri": result_uri})).await);
        assert_eq!(item["name"], "Quarterly report");
        assert_eq!(item["itemUri"], result_uri.as_str());

        let listing = read_json(
            &full
                .rpc("resources/read", json!({"uri": "usd://items"}))
                .await,
        );
        assert_eq!(listing["items"].as_array().unwrap().len(), 1);
        assert_eq!(listing["nextCursor"], Value::Null);

        // The resource adapter owns limit/cursor selection and follows its canonical template.
        for name in ["second", "third"] {
            let answer = full
                .rpc(
                    "tools/call",
                    json!({"name":"create_item","arguments":{"name":name}}),
                )
                .await;
            assert_eq!(answer["result"]["isError"], false, "{answer}");
        }
        let first = read_json(
            &full
                .rpc("resources/read", json!({"uri":"usd://items?limit=2"}))
                .await,
        );
        assert_eq!(first["items"].as_array().unwrap().len(), 2);
        let mut uri = url::Url::parse("usd://items").unwrap();
        uri.query_pairs_mut()
            .append_pair("limit", "2")
            .append_pair("cursor", first["nextCursor"].as_str().unwrap());
        let second = read_json(
            &full
                .rpc("resources/read", json!({"uri":uri.as_str()}))
                .await,
        );
        assert_eq!(second["items"].as_array().unwrap().len(), 1);
        assert!(second["nextCursor"].is_null());
        let ids: Vec<_> = first["items"]
            .as_array()
            .unwrap()
            .iter()
            .chain(second["items"].as_array().unwrap())
            .map(|item| item["itemId"].as_str().unwrap())
            .collect();
        assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
        let refused = full
            .rpc(
                "resources/read",
                json!({"uri":"usd://items?cursor=a&cursor=b"}),
            )
            .await;
        assert!(refused["error"].is_object(), "{refused}");

        // A name the schema refuses reaches no handler.
        let refused = full
            .rpc(
                "tools/call",
                json!({"name": "create_item", "arguments": {"name": " padded "}}),
            )
            .await;
        assert!(
            refused["result"]["isError"] == true || refused["error"].is_object(),
            "{refused}"
        );

        // Reading without the write permission cannot create, and the refusal names the
        // missing permission.
        let reader = hub(&db, &[ItemsRead]).await;
        let denied = reader
            .rpc(
                "tools/call",
                json!({"name": "create_item", "arguments": {"name": "denied"}}),
            )
            .await;
        assert!(
            error_message(&denied).contains("usd.items.write"),
            "{denied}"
        );
        let listing = read_json(
            &reader
                .rpc("resources/read", json!({"uri": "usd://items"}))
                .await,
        );
        assert_eq!(listing["items"].as_array().unwrap().len(), 3);

        // An unknown item reads as missing.
        let missing = reader
            .rpc(
                "resources/read",
                json!({"uri": "usd://item/0190f3a2-7b1c-7d2e-8f3a-4b5c6d7e8f90"}),
            )
            .await;
        assert!(
            error_message(&missing).contains("no such item"),
            "{missing}"
        );
    })
    .await
    .expect("the MCP test exceeded 60 seconds");
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

#[tokio::test]
async fn the_skills_manifest_names_every_file_with_its_digest() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let db = TestDb::new().await;
        db.migrate_server(&server(), migrator()).await;
        let hub = hub(&db, &[ItemsRead]).await;
        let listed = hub.rpc("skills/list", json!({})).await;
        let skills = listed["result"]["skills"]
            .as_array()
            .unwrap_or_else(|| panic!("{listed}"));
        assert_eq!(skills.len(), 1);
        let skill = &skills[0];
        assert_eq!(skill["uri"], "skill://usd/keep-items/SKILL.md");
        assert_eq!(skill["frontmatter"]["name"], "keep-items");

        // The manifest lists exactly the files under skills/keep-items/, each with the
        // size and SHA-256 of the bytes a read answers.
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("skills/keep-items");
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
                    .strip_prefix("skill://usd/keep-items/")
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
        // skills/get answers the listed entry.
        let got = hub
            .rpc(
                "skills/get",
                json!({"uri": "skill://usd/keep-items/SKILL.md"}),
            )
            .await;
        assert_eq!(got["result"]["skill"], *skill);
    })
    .await
    .expect("the skills test exceeded 60 seconds");
}
