// SPDX-License-Identifier: UNLICENSED
//! M30 — Current-region awareness.
//!
//! Resolves the current EFE region from environment variables at
//! process start :
//!
//! - `EFE_REGION` (required) — e.g. `eu-west-1`, `eu-west-3`
//! - `EFE_REGION_ROLE` (required) — `active` or `passive`
//! - `EFE_REGION_PEER_BASE_URLS` (optional) — comma-separated list of
//!   peer region base URLs in the format `region=url`.

use std::collections::HashMap;
use std::env;
use std::sync::Arc;

use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};

use crate::error::{RegionError, Result};

/// Active or passive role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RegionRole {
    /// Active region — serves write traffic.
    Active,
    /// Passive region — serves read traffic only, replicates from
    /// active. Promoted to Active on failover.
    Passive,
}

/// Snapshot of the current region's identity + topology.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurrentRegion {
    /// Region name (e.g. `eu-west-1`).
    pub region: String,
    /// Active / passive role at this point in time.
    pub role: RegionRole,
    /// Map `peer_region → base URL`.
    pub peer_base_urls: HashMap<String, String>,
}

static CURRENT: OnceCell<Arc<CurrentRegion>> = OnceCell::new();

impl CurrentRegion {
    /// Initialize from environment variables exactly once.
    pub fn init_from_env() -> Result<Arc<Self>> {
        let region = env::var("EFE_REGION").map_err(|_| RegionError::Internal {
            reason: "EFE_REGION not set".into(),
        })?;
        let role_str = env::var("EFE_REGION_ROLE").map_err(|_| RegionError::Internal {
            reason: "EFE_REGION_ROLE not set".into(),
        })?;
        let role = match role_str.as_str() {
            "active" => RegionRole::Active,
            "passive" => RegionRole::Passive,
            other => {
                return Err(RegionError::Internal {
                    reason: format!("invalid EFE_REGION_ROLE: {other}"),
                })
            }
        };
        let peer_base_urls = parse_peer_urls(env::var("EFE_REGION_PEER_BASE_URLS").ok());

        let snap = Arc::new(Self {
            region,
            role,
            peer_base_urls,
        });
        let _ = CURRENT.set(snap.clone()); // ignore "already set" on test re-init
        Ok(snap)
    }

    /// Cheap accessor — returns the once-initialized snapshot.
    pub fn get() -> Option<Arc<Self>> {
        CURRENT.get().cloned()
    }

    /// Test-only override.
    #[doc(hidden)]
    pub fn install_for_test(this: Arc<Self>) {
        let _ = CURRENT.set(this);
    }

    /// Returns `true` if this region is the active one.
    pub fn is_active(&self) -> bool {
        matches!(self.role, RegionRole::Active)
    }

    /// Returns the peer base URL for `region`, if configured.
    pub fn peer_url(&self, region: &str) -> Option<&str> {
        self.peer_base_urls.get(region).map(String::as_str)
    }
}

fn parse_peer_urls(raw: Option<String>) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Some(raw) = raw else { return out };
    for entry in raw.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        if let Some((k, v)) = entry.split_once('=') {
            out.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_peer_urls_handles_empty() {
        assert!(parse_peer_urls(None).is_empty());
        assert!(parse_peer_urls(Some(String::new())).is_empty());
    }

    #[test]
    fn parse_peer_urls_parses_basic() {
        let raw = "eu-west-3=https://efe-eu-west-3.example.com,eu-west-1=https://efe-eu-west-1.example.com";
        let m = parse_peer_urls(Some(raw.into()));
        assert_eq!(
            m.get("eu-west-3").map(String::as_str),
            Some("https://efe-eu-west-3.example.com")
        );
        assert_eq!(
            m.get("eu-west-1").map(String::as_str),
            Some("https://efe-eu-west-1.example.com")
        );
    }

    #[test]
    fn parse_peer_urls_skips_invalid_entries() {
        let raw = "bad,eu-west-1=https://x,=https://y";
        let m = parse_peer_urls(Some(raw.into()));
        assert_eq!(m.len(), 2);
        assert!(m.contains_key("eu-west-1"));
    }

    #[test]
    fn role_is_active_predicate() {
        let r = CurrentRegion {
            region: "eu-west-1".into(),
            role: RegionRole::Active,
            peer_base_urls: HashMap::new(),
        };
        assert!(r.is_active());
        let p = CurrentRegion { role: RegionRole::Passive, ..r };
        assert!(!p.is_active());
    }
}
