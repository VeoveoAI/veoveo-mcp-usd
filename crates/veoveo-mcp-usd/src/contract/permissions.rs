//! The server's OAuth scope and permissions. Each permission names one action on one
//! resource type, with the access kind and risks the `ai.veoveo/authorization`
//! declaration carries; installation roles grant them and the Hub forwards the
//! granted ones.
use veoveo_hub_sdk::registration::{Access, PermissionDefinition, Risk};
use veoveo_types::{PermissionName, Vocabulary};

/// The OAuth scope that names the server's namespace. Scopes are coarse; the
/// permissions carry fine-grained access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Vocabulary)]
#[vocabulary(scope)]
pub enum UsdScope {
    #[vocabulary(rename = "usd")]
    Usd,
}

/// The server's permissions.
/// ```compile_fail
/// use veoveo_mcp_usd::contract::UsdPermission;
/// fn requires(_: UsdPermission) {}
/// requires("usd.items.read");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Vocabulary)]
pub enum UsdPermission {
    /// Read your tenant's items and the server's documents.
    #[vocabulary(rename = "usd.items.read")]
    ItemsRead,
    /// Create items in your tenant.
    #[vocabulary(rename = "usd.items.write")]
    ItemsWrite,
}

impl UsdPermission {
    /// The permission's wire name, as declared and as granted.
    pub fn name(self) -> PermissionName {
        PermissionName::parse(self.as_str()).expect("declared usd permission spelling")
    }
}

impl PermissionDefinition for UsdPermission {
    fn resource(self) -> &'static str {
        "usd.item"
    }

    fn access(self) -> Access {
        match self {
            Self::ItemsRead => Access::Read,
            Self::ItemsWrite => Access::Write,
        }
    }

    fn risks(self) -> &'static [Risk] {
        &[]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissions_follow_the_naming_form() {
        for permission in UsdPermission::ALL {
            let name = permission.as_str();
            let mut parts = name.split('.');
            assert_eq!(parts.next(), Some("usd"), "{name}");
            assert!(parts.next().is_some_and(|plural| !plural.is_empty()));
            assert!(parts.next().is_some_and(|verb| !verb.is_empty()));
            assert_eq!(parts.next(), None, "{name}");
            assert_eq!(permission.name().as_str(), name);
        }
        assert_eq!(UsdScope::Usd.as_str(), "usd");
    }
}
