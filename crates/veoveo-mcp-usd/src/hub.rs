//! The Hub's own routes, called for a tool's caller or a Task's owner with the bearer the
//! SDK exchanges for their profile. The Hub decides each call, so Lore's rules decide each
//! read; its refusals reach the agent with their status and RFC 9457 problem detail.
use std::{fmt, time::Duration};

use bytes::Bytes;
use reqwest::{
    StatusCode,
    header::{ACCEPT, ETAG, RANGE},
};
use serde::Deserialize;
use veoveo_hub_sdk::{
    HubProfileId,
    client::{CallError, HubClient},
};

use crate::contract::{BranchName, RepositoryId};

/// The header in which the files route names the branch's tip.
const REVISION: &str = "lore-revision";
/// How long a listing may take.
const LISTING_TIMEOUT: Duration = Duration::from_secs(30);
/// How long one file may take, to its last byte.
const FILE_TIMEOUT: Duration = Duration::from_secs(120);

/// The Hub at the origin of its issuer.
#[derive(Clone)]
pub struct Hub {
    base: url::Url,
    http: reqwest::Client,
}

/// Why a call to the Hub produced no answer.
#[derive(Debug, thiserror::Error)]
pub enum HubError {
    /// The SDK obtained no bearer for the caller.
    #[error("no token for the caller: {0}")]
    Bearer(#[from] CallError),
    #[error("the Hub could not be reached: {0}")]
    Transport(reqwest::Error),
    /// The Hub answered with an error status.
    #[error("the Hub answered {status}{problem}")]
    Answered {
        status: StatusCode,
        problem: Problem,
    },
    #[error("the Hub's answer is not what the server reads: {0}")]
    Decode(#[from] serde_json::Error),
}

impl HubError {
    /// Whether the Hub says there is no such file, branch or repository.
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::Answered { status, .. } if *status == StatusCode::NOT_FOUND)
    }
}

impl From<reqwest::Error> for HubError {
    fn from(error: reqwest::Error) -> Self {
        // A URL names a repository and a path, and holds no credential, but it has no place in
        // an agent's error text either.
        Self::Transport(error.without_url())
    }
}

/// The RFC 9457 problem detail the Hub answers an error with.
#[derive(Debug, Default, Deserialize)]
pub struct Problem {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for part in [&self.title, &self.detail].into_iter().flatten() {
            write!(f, ": {part}")?;
        }
        Ok(())
    }
}

/// A repository the caller holds a grant on.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Repository {
    pub id: RepositoryId,
    pub name: String,
}

#[derive(Debug, Deserialize)]
struct Repositories {
    repositories: Vec<Repository>,
}

/// What a folder holds.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Listing {
    pub entries: Vec<Entry>,
    /// The branch's tip it was read at.
    #[serde(skip)]
    pub revision: Option<String>,
}

/// Everything on a branch's tip.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Tree {
    pub entries: Vec<TreeEntry>,
    #[serde(skip)]
    pub revision: Option<String>,
}

/// One name in a tree, by its whole path.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TreeEntry {
    pub path: String,
    #[serde(rename = "type")]
    pub kind: EntryKind,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub address: Option<String>,
}

/// One name in a folder.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Entry {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: EntryKind,
    #[serde(default)]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    File,
    Directory,
    Link,
}

/// A file as the Hub answered it.
#[derive(Debug, Clone)]
pub struct File {
    pub bytes: Bytes,
    /// The file's content address.
    pub address: String,
    /// The branch's tip when it was read.
    pub revision: Option<String>,
}

/// What one byte of a file says about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    /// The file's content address.
    pub address: String,
    /// The branch's tip when it was read.
    pub revision: Option<String>,
}

fn revision_of(response: &reqwest::Response) -> Option<String> {
    response
        .headers()
        .get("lore-revision")?
        .to_str()
        .ok()
        .map(str::to_owned)
}

/// The Hub, for one caller at one profile.
#[derive(Clone)]
pub struct Reader {
    pub hub: Hub,
    pub client: HubClient,
    pub profile: HubProfileId,
}

impl Hub {
    pub fn new(issuer: &url::Url, http: reqwest::Client) -> anyhow::Result<Self> {
        anyhow::ensure!(
            issuer.has_host() && !issuer.cannot_be_a_base(),
            "the Hub's issuer is no HTTP origin"
        );
        Ok(Self {
            base: issuer.clone(),
            http,
        })
    }

    /// A route of `segments`, each percent-encoded as one path segment.
    fn route<'a>(&self, segments: impl IntoIterator<Item = &'a str>) -> url::Url {
        let mut url = self.base.clone();
        url.set_query(None);
        url.set_fragment(None);
        url.path_segments_mut()
            .expect("an HTTP origin has a path")
            .clear()
            .extend(segments);
        url
    }

    fn files_route<'a>(
        &self,
        profile: &'a HubProfileId,
        repository: &'a RepositoryId,
        branch: &'a BranchName,
        path: impl IntoIterator<Item = &'a str>,
    ) -> url::Url {
        self.route(
            [
                "lore",
                profile.as_str(),
                "repositories",
                repository.as_str(),
                "files",
                branch.as_str(),
            ]
            .into_iter()
            .chain(path),
        )
    }
}

impl Reader {
    async fn get(&self, url: url::Url, timeout: Duration) -> Result<reqwest::Response, HubError> {
        let bearer = self.client.hub_bearer().await?;
        let response = self
            .hub
            .http
            .get(url)
            .bearer_auth(bearer.expose())
            .timeout(timeout)
            .send()
            .await?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let body = response.bytes().await?;
        Err(HubError::Answered {
            status,
            problem: serde_json::from_slice(&body).unwrap_or_default(),
        })
    }

    /// `GET /lore/{profile}/repositories`: the repositories the caller holds a grant on.
    pub async fn repositories(&self) -> Result<Vec<Repository>, HubError> {
        let url = self
            .hub
            .route(["lore", self.profile.as_str(), "repositories"]);
        let response = self.get(url, LISTING_TIMEOUT).await?;
        let body = response.bytes().await?;
        Ok(serde_json::from_slice::<Repositories>(&body)?.repositories)
    }

    /// A folder's listing, with a trailing `/`.
    pub async fn listing(
        &self,
        repository: &RepositoryId,
        branch: &BranchName,
        folder: &[&str],
    ) -> Result<Listing, HubError> {
        let url = self.hub.files_route(
            &self.profile,
            repository,
            branch,
            folder.iter().copied().chain([""]),
        );
        let response = self.get(url, LISTING_TIMEOUT).await?;
        let revision = revision_of(&response);
        let body = response.bytes().await?;
        Ok(Listing {
            revision,
            ..serde_json::from_slice(&body)?
        })
    }

    /// `GET /lore/{profile}/repositories/{id}/tree/{branch}`: everything on the branch's tip, by
    /// one read.
    pub async fn tree(
        &self,
        repository: &RepositoryId,
        branch: &BranchName,
    ) -> Result<Tree, HubError> {
        let url = self.hub.route([
            "lore",
            self.profile.as_str(),
            "repositories",
            repository.as_str(),
            "tree",
            branch.as_str(),
        ]);
        let response = self.get(url, FILE_TIMEOUT).await?;
        let revision = revision_of(&response);
        let body = response.bytes().await?;
        Ok(Tree {
            revision,
            ..serde_json::from_slice(&body)?
        })
    }

    /// Reads one byte of a file as the caller, so Lore decides whether they may read it now, and
    /// what it answers names the file's content address and the branch's tip.
    pub async fn probe(
        &self,
        repository: &RepositoryId,
        branch: &BranchName,
        path: &str,
    ) -> Result<Probe, HubError> {
        let url = self
            .hub
            .files_route(&self.profile, repository, branch, path.split('/'));
        let bearer = self.client.hub_bearer().await?;
        let response = self
            .hub
            .http
            .get(url)
            .bearer_auth(bearer.expose())
            .header(ACCEPT, "application/octet-stream")
            .header(RANGE, "bytes=0-0")
            .timeout(LISTING_TIMEOUT)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.bytes().await?;
            return Err(HubError::Answered {
                status,
                problem: serde_json::from_slice(&body).unwrap_or_default(),
            });
        }
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(|value| value.trim_matches('"').to_owned())
        };
        Ok(Probe {
            address: header(ETAG.as_str()).unwrap_or_default(),
            revision: header(REVISION),
        })
    }

    /// A file of `repository`, by the segments of its path.
    pub async fn file(
        &self,
        repository: &RepositoryId,
        branch: &BranchName,
        path: &str,
    ) -> Result<File, HubError> {
        let url = self
            .hub
            .files_route(&self.profile, repository, branch, path.split('/'));
        let bearer = self.client.hub_bearer().await?;
        let response = self
            .hub
            .http
            .get(url)
            .bearer_auth(bearer.expose())
            .header(ACCEPT, "application/octet-stream")
            .timeout(FILE_TIMEOUT)
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = response.bytes().await?;
            return Err(HubError::Answered {
                status,
                problem: serde_json::from_slice(&body).unwrap_or_default(),
            });
        }
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(|value| value.trim_matches('"').to_owned())
        };
        let address = header(ETAG.as_str()).unwrap_or_default();
        let revision = header(REVISION);
        Ok(File {
            bytes: response.bytes().await?,
            address,
            revision,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hub() -> Hub {
        let _ = rustls::crypto::ring::default_provider().install_default();
        Hub::new(
            &url::Url::parse("https://hub.example/issuer?x=1").unwrap(),
            reqwest::Client::builder().build().unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn each_segment_is_encoded_once_at_the_issuers_origin() {
        let url = hub().files_route(
            &HubProfileId::parse("default").unwrap(),
            &RepositoryId::parse("r1").unwrap(),
            &BranchName::parse("feature/x").unwrap(),
            "a b/c#d.usd".split('/'),
        );
        assert_eq!(
            url.as_str(),
            "https://hub.example/lore/default/repositories/r1/files/feature%2Fx/a%20b/c%23d.usd"
        );
    }

    #[test]
    fn a_folder_listing_decodes() {
        let listing: Listing = serde_json::from_str(
            r#"{"repository":"r1","branch":"main","path":"","entries":[
                {"name":"a","type":"directory","size":12},
                {"name":"b.usd","type":"file","size":3,"address":"ab"},
                {"name":"c","type":"link"}]}"#,
        )
        .unwrap();
        assert_eq!(listing.entries[0].kind, EntryKind::Directory);
        assert_eq!(listing.entries[1].size, Some(3));
        assert_eq!(listing.entries[2].kind, EntryKind::Link);
    }
}
