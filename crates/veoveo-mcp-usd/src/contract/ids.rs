//! The server's checked values: what names a repository, a branch, a stage's path and a
//! prim. Each admits its input once, so internal code never handles a raw string where one
//! of these is meant. Lore decides what exists; these refuse what could not be a name.
use std::{borrow::Cow, fmt};

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

/// The most bytes any of these values holds.
const MAX_BYTES: usize = 1024;

/// A value that failed admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UsdIdError {
    #[error("a repository is 1 to 128 characters of letters, digits, `-` and `_`")]
    Repository,
    #[error("a branch is a name of 1 to 1024 bytes without control characters")]
    Branch,
    #[error(
        "a stage's path is relative and `/`-separated, with no empty, `.` or `..` segment, and ends in .usd, .usda, .usdc or .usdz"
    )]
    Stage,
    #[error("a prim's path is absolute, such as /World/Camera")]
    Prim,
}

fn no_controls(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_BYTES && !value.chars().any(char::is_control)
}

macro_rules! checked_string {
    ($(#[$meta:meta])* $name:ident, $describe:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl TryFrom<String> for $name {
            type Error = UsdIdError;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::parse(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                stringify!($name).into()
            }
            fn json_schema(_: &mut SchemaGenerator) -> Schema {
                json_schema!({"type": "string", "description": $describe})
            }
        }
    };
}

checked_string!(
    /// A Lore repository, as the Hub names it.
    RepositoryId,
    "A Lore repository's identifier, from list_scenes."
);
checked_string!(
    /// A branch of a repository.
    BranchName,
    "A branch of the repository."
);
checked_string!(
    /// A root stage in a repository, relative to its top.
    StagePath,
    "The stage's path in the repository, such as franka/franka.usd."
);
checked_string!(
    /// A prim's absolute path in a stage.
    PrimPath,
    "A prim's absolute path, such as /World/Camera."
);

impl RepositoryId {
    pub fn parse(value: impl Into<String>) -> Result<Self, UsdIdError> {
        let value = value.into();
        let admitted = !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
        admitted
            .then_some(Self(value))
            .ok_or(UsdIdError::Repository)
    }
}

impl BranchName {
    pub fn parse(value: impl Into<String>) -> Result<Self, UsdIdError> {
        let value = value.into();
        no_controls(&value)
            .then_some(Self(value))
            .ok_or(UsdIdError::Branch)
    }
}

impl StagePath {
    pub fn parse(value: impl Into<String>) -> Result<Self, UsdIdError> {
        let value = value.into();
        let segments = value
            .split('/')
            .all(|segment| !matches!(segment, "" | "." | "..") && !segment.contains('\\'));
        let extension = value
            .rsplit_once('.')
            .map(|(_, extension)| extension.to_ascii_lowercase());
        let usd = matches!(extension.as_deref(), Some("usd" | "usda" | "usdc" | "usdz"));
        (no_controls(&value) && segments && usd)
            .then_some(Self(value))
            .ok_or(UsdIdError::Stage)
    }

    /// The path's segments.
    pub fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.split('/')
    }

    /// The file's name.
    pub fn file_name(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or(&self.0)
    }
}

impl PrimPath {
    pub fn parse(value: impl Into<String>) -> Result<Self, UsdIdError> {
        let value = value.into();
        let admitted = no_controls(&value)
            && value.starts_with('/')
            && value.len() > 1
            && !value.chars().any(char::is_whitespace);
        admitted.then_some(Self(value)).ok_or(UsdIdError::Prim)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repositories_are_plain_identifiers() {
        assert!(RepositoryId::parse("0190f3a27b1c7d2e8f3a4b5c6d7e8f90").is_ok());
        assert!(RepositoryId::parse("usd-demo").is_ok());
        for value in ["", "a/b", "a b", "a?b", "../x"] {
            assert_eq!(RepositoryId::parse(value), Err(UsdIdError::Repository));
        }
    }

    #[test]
    fn a_branch_may_hold_a_slash_but_no_control() {
        assert!(BranchName::parse("feature/x").is_ok());
        assert_eq!(BranchName::parse(""), Err(UsdIdError::Branch));
        assert_eq!(BranchName::parse("a\nb"), Err(UsdIdError::Branch));
    }

    #[test]
    fn a_stage_is_a_relative_usd_file() {
        for value in [
            "franka/franka.usd",
            "a.USDZ",
            "kitchen_set/kitchen_set.usdz",
            "b/c.usda",
            "d/e f.usdc",
        ] {
            assert!(StagePath::parse(value).is_ok(), "{value}");
        }
        for value in [
            "",
            "/a.usd",
            "a/../b.usd",
            "./a.usd",
            "a//b.usd",
            "a.png",
            "usd",
            "a\\b.usd",
            "a\nb.usd",
        ] {
            assert_eq!(StagePath::parse(value), Err(UsdIdError::Stage), "{value}");
        }
        assert_eq!(
            StagePath::parse("franka/franka.usd").unwrap().file_name(),
            "franka.usd"
        );
    }

    #[test]
    fn a_prim_is_absolute() {
        assert!(PrimPath::parse("/World/Camera").is_ok());
        for value in ["", "/", "World", "/a b"] {
            assert_eq!(PrimPath::parse(value), Err(UsdIdError::Prim), "{value}");
        }
    }

    #[test]
    fn serde_admits_through_parse() {
        let parsed: StagePath = serde_json::from_str("\"a/b.usda\"").unwrap();
        assert_eq!(parsed.as_str(), "a/b.usda");
        assert!(serde_json::from_str::<StagePath>("\"a/../b.usda\"").is_err());
    }
}
