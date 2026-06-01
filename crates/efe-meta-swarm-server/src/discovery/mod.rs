//! Participant discovery.
//!
//! A `trait Discovery` with two impls :
//!
//!   * [`StaticDiscovery`] — Phase-2 default ; reads from
//!     `EFE_META_SWARM_PARTICIPANTS`.
//!   * `K8sDnsDiscovery` — Phase 3 ; uses K8s DNS to resolve a
//!     headless Service to a list of pod IPs.
//!
//! PR #6 ships only `StaticDiscovery`.

mod static_impl;

pub use static_impl::StaticDiscovery;

use async_trait::async_trait;

use crate::config::controller::StaticParticipant;

/// A discovered participant the controller should connect to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredParticipant {
    pub id: String,
    pub endpoint: String, // gRPC `host:port`, no scheme
}

/// Pluggable participant discovery.
#[async_trait]
pub trait Discovery: Send + Sync + 'static {
    /// Return the current list of participants.  Called periodically
    /// (every 10 s) by the controller's refresh loop.
    async fn discover(&self) -> Result<Vec<DiscoveredParticipant>, crate::error::ServerError>;
}

impl From<&StaticParticipant> for DiscoveredParticipant {
    fn from(p: &StaticParticipant) -> Self {
        Self {
            id: p.id.clone(),
            endpoint: p.addr.clone(),
        }
    }
}
