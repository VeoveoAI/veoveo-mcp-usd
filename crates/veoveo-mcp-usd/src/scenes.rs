//! The scenes of a repository at one revision, by one read of its whole tree. A scene is a folder
//! with exactly one stage in it at any depth, or with a stage named for it beside other layers,
//! known by the folder's name; or else a stage on its own, known by its file's. A revision fixes
//! all of this, so it is held in memory, and each person's read of the repository's top is what
//! says they may read it now.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use crate::{
    contract::{BranchName, RepositoryId},
    hub::{EntryKind, HubError, Reader, TreeEntry},
};

const STAGES: [&str; 4] = ["usd", "usda", "usdc", "usdz"];
/// How often a repository's tip is asked for when its last scenes can be shown meanwhile.
const LOOK_AGAIN: Duration = Duration::from_secs(60);

fn stage_stem(name: &str) -> Option<&str> {
    let (stem, extension) = name.rsplit_once('.')?;
    STAGES
        .contains(&extension.to_ascii_lowercase().as_str())
        .then_some(stem)
}

/// One scene.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub name: String,
    pub path: String,
    pub size: Option<u64>,
    pub address: String,
}

/// A repository's scenes at one revision.
#[derive(Debug, Default)]
pub struct RepositoryScenes {
    pub revision: Option<String>,
    pub scenes: Vec<Found>,
    /// Every file's content address, by its path.
    pub addresses: HashMap<String, String>,
}

#[derive(Default)]
struct Folder<'a> {
    folders: Vec<&'a TreeEntry>,
    stages: Vec<&'a TreeEntry>,
    /// The stage that stands for everything in this folder, and how many it holds.
    only: Option<&'a TreeEntry>,
    count: usize,
}

impl RepositoryScenes {
    pub fn find(entries: &[TreeEntry], revision: Option<String>) -> Self {
        let mut folders: HashMap<&str, Folder> = HashMap::new();
        folders.entry("").or_default();
        for entry in entries {
            let parent = entry.path.rsplit_once('/').map_or("", |(parent, _)| parent);
            let name = entry.path.rsplit('/').next().unwrap_or_default();
            match entry.kind {
                EntryKind::Directory => {
                    folders.entry(&entry.path).or_default();
                    folders.entry(parent).or_default().folders.push(entry);
                }
                EntryKind::File if stage_stem(name).is_some() => {
                    folders.entry(parent).or_default().stages.push(entry);
                }
                _ => {}
            }
        }
        let mut paths: Vec<&str> = folders.keys().copied().collect();
        paths.sort_by_key(|path| {
            std::cmp::Reverse(path.matches('/').count() + usize::from(!path.is_empty()))
        });
        for path in paths {
            let name = path.rsplit('/').next().unwrap_or_default();
            let inside: Vec<_> = folders[path]
                .folders
                .iter()
                .filter_map(|entry| folders.get(entry.path.as_str()))
                .map(|folder| (folder.count, folder.only))
                .collect();
            let Some(folder) = folders.get_mut(path) else {
                continue;
            };
            let root = folder.stages.iter().copied().find(|stage| {
                let file = stage.path.rsplit('/').next().unwrap_or_default();
                !name.is_empty() && stage_stem(file) == Some(name)
            });
            folder.count =
                folder.stages.len() + inside.iter().map(|(count, _)| count).sum::<usize>();
            folder.only = root.or_else(|| {
                (folder.count == 1)
                    .then(|| folder.stages.first().copied())
                    .flatten()
                    .or_else(|| {
                        (folder.count == 1)
                            .then(|| inside.iter().find_map(|(_, only)| *only))
                            .flatten()
                    })
            });
            if root.is_some() {
                folder.count = 1;
            }
        }
        let mut scenes = Vec::new();
        let mut stack = vec![""];
        while let Some(path) = stack.pop() {
            let folder = &folders[path];
            for entry in &folder.folders {
                let inside = &folders[entry.path.as_str()];
                match inside.only {
                    Some(stage) => {
                        let name = entry.path.rsplit('/').next().unwrap_or_default();
                        scenes.extend(found(name, stage, entry.size));
                    }
                    None => stack.push(entry.path.as_str()),
                }
            }
            if !path.is_empty() && folder.only.is_some() {
                continue;
            }
            for stage in &folder.stages {
                let file = stage.path.rsplit('/').next().unwrap_or_default();
                scenes.extend(found(stage_stem(file).unwrap_or(file), stage, stage.size));
            }
        }
        scenes.sort_by_key(|scene| scene.path.to_lowercase());
        let addresses = entries
            .iter()
            .filter_map(|entry| Some((entry.path.clone(), entry.address.clone()?)))
            .collect();
        Self {
            revision,
            scenes,
            addresses,
        }
    }
}

fn found(name: &str, stage: &TreeEntry, size: Option<u64>) -> Option<Found> {
    Some(Found {
        name: name.to_owned(),
        path: stage.path.clone(),
        size,
        address: stage.address.clone()?,
    })
}

type Place = (RepositoryId, BranchName);

/// A branch's scenes and when its tip was last asked for.
type Held = HashMap<Place, (Arc<RepositoryScenes>, Instant)>;

/// The latest scenes of each repository's branch.
#[derive(Clone, Default)]
pub struct Scenes {
    held: Arc<Mutex<Held>>,
    turns: Arc<Mutex<HashMap<Place, Arc<tokio::sync::Mutex<()>>>>>,
}

impl Scenes {
    /// The last scenes known, at whatever revision, and whether to ask for the tip again. A person
    /// with a grant on the repository may see an earlier revision of it.
    pub fn latest(
        &self,
        repository: &RepositoryId,
        branch: &BranchName,
    ) -> Option<(Arc<RepositoryScenes>, bool)> {
        let held = self.held.lock().unwrap_or_else(|held| held.into_inner());
        held.get(&(repository.clone(), branch.clone()))
            .map(|(scenes, checked)| (scenes.clone(), checked.elapsed() > LOOK_AGAIN))
    }

    /// The scenes of the branch's tip, for the reader's person. Only a new tip reads the whole tree.
    pub async fn at_tip(
        &self,
        reader: &Reader,
        repository: &RepositoryId,
        branch: &BranchName,
    ) -> Result<Arc<RepositoryScenes>, HubError> {
        let key = (repository.clone(), branch.clone());
        let tip = reader.listing(repository, branch, &[]).await?.revision;
        let current = || {
            let mut held = self.held.lock().unwrap_or_else(|held| held.into_inner());
            let (scenes, checked) = held.get_mut(&key)?;
            (scenes.revision == tip).then(|| {
                *checked = Instant::now();
                scenes.clone()
            })
        };
        if let Some(scenes) = current() {
            return Ok(scenes);
        }
        let turn = {
            let mut turns = self.turns.lock().unwrap_or_else(|held| held.into_inner());
            turns.entry(key.clone()).or_default().clone()
        };
        let _turn = turn.lock().await;
        if let Some(scenes) = current() {
            return Ok(scenes);
        }
        let tree = reader.tree(repository, branch).await?;
        let scenes = Arc::new(RepositoryScenes::find(&tree.entries, tree.revision));
        let mut held = self.held.lock().unwrap_or_else(|held| held.into_inner());
        held.insert(key, (scenes.clone(), Instant::now()));
        Ok(scenes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, kind: EntryKind) -> TreeEntry {
        TreeEntry {
            path: path.to_owned(),
            kind,
            size: Some(3),
            address: (kind == EntryKind::File).then(|| "ad".to_owned()),
        }
    }

    #[test]
    fn scenes_are_found_at_any_depth_by_their_folders_names() {
        use EntryKind::{Directory as D, File as F};
        let entries = [
            ("chameleon", D),
            ("chameleon/chameleon.usdz", F),
            ("franka", D),
            ("franka/arm.usd", F),
            ("franka/franka.usd", F),
            ("rigid", D),
            ("rigid/R001", D),
            ("rigid/R001/01_compact", D),
            ("rigid/R001/01_compact/meta.json", F),
            ("rigid/R001/01_compact/physx", D),
            ("rigid/R001/01_compact/physx/model.usdc", F),
            ("rigid/R001/02_steel", D),
            ("rigid/R001/02_steel/physx", D),
            ("rigid/R001/02_steel/physx/model.usdc", F),
            ("parts", D),
            ("parts/a.usda", F),
            ("parts/b.usda", F),
            ("src", D),
            ("src/main.rs", F),
            ("top.usdz", F),
        ]
        .map(|(path, kind)| entry(path, kind));
        let found = RepositoryScenes::find(&entries, None);
        let names: Vec<(&str, &str)> = found
            .scenes
            .iter()
            .map(|scene| (scene.name.as_str(), scene.path.as_str()))
            .collect();
        assert_eq!(
            names,
            [
                ("chameleon", "chameleon/chameleon.usdz"),
                ("franka", "franka/franka.usd"),
                ("a", "parts/a.usda"),
                ("b", "parts/b.usda"),
                ("01_compact", "rigid/R001/01_compact/physx/model.usdc"),
                ("02_steel", "rigid/R001/02_steel/physx/model.usdc"),
                ("top", "top.usdz"),
            ]
        );
    }
}
