//! [`TenantId`] — UUID-based per-tenant identifier.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

/// Tenant identifier (UUID v4).
///
/// All catalog reads, writes, and execution contexts are scoped by tenant.
///
/// # Examples
///
/// ```
/// use efe_nano_sdk::types::TenantId;
///
/// let t = TenantId::new();
/// assert_ne!(t, TenantId::new());  // each new() is unique
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TenantId(pub Uuid);

impl TenantId {
    /// Generate a new random tenant ID (UUID v4).
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Construct from a pre-existing UUID.
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Get the underlying UUID.
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }

    /// Special "system" tenant used for EFE-shipped nano agents.
    pub fn system() -> Self {
        // Stable system UUID (used for EFE-shipped agents).
        Self(Uuid::from_u128(0x00000000_0000_0000_0000_000000000001))
    }
}

impl Default for TenantId {
    fn default() -> Self {
        Self::new()
    }
}

impl FromStr for TenantId {
    type Err = uuid::Error;

    /// Parse a TenantId from a string representation of a UUID.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

impl fmt::Display for TenantId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_generates_unique() {
        let a = TenantId::new();
        let b = TenantId::new();
        assert_ne!(a, b);
    }

    #[test]
    fn from_uuid_round_trips() {
        let uuid = Uuid::new_v4();
        let t = TenantId::from_uuid(uuid);
        assert_eq!(t.as_uuid(), uuid);
    }

    #[test]
    fn from_str_valid_uuid() {
        let s = "550e8400-e29b-41d4-a716-446655440000";
        let t = TenantId::from_str(s).unwrap();
        assert_eq!(t.to_string(), s);
    }

    #[test]
    fn from_str_invalid() {
        assert!(TenantId::from_str("not-a-uuid").is_err());
    }

    #[test]
    fn system_tenant_is_stable() {
        let a = TenantId::system();
        let b = TenantId::system();
        assert_eq!(a, b);
        assert_eq!(
            a.to_string(),
            "00000000-0000-0000-0000-000000000001"
        );
    }

    #[test]
    fn serde_transparent() {
        let uuid = Uuid::from_u128(0xdeadbeef_cafe_babe_0000_000000000042);
        let t = TenantId::from_uuid(uuid);
        let s = serde_json::to_string(&t).unwrap();
        // serde(transparent) → serialized as the UUID string directly
        assert_eq!(s, format!(r#""{uuid}""#));
        let t2: TenantId = serde_json::from_str(&s).unwrap();
        assert_eq!(t, t2);
    }
}
