//! Static discovery — Phase-2 default.

use async_trait::async_trait;

use super::{DiscoveredParticipant, Discovery};
use crate::config::controller::StaticParticipant;
use crate::error::ServerError;

/// Discovery from an in-memory list populated at startup from
/// `EFE_META_SWARM_PARTICIPANTS`.
#[derive(Debug, Clone)]
pub struct StaticDiscovery {
    participants: Vec<DiscoveredParticipant>,
}

impl StaticDiscovery {
    pub fn from_static(list: &[StaticParticipant]) -> Self {
        Self {
            participants: list.iter().map(DiscoveredParticipant::from).collect(),
        }
    }
}

#[async_trait]
impl Discovery for StaticDiscovery {
    async fn discover(&self) -> Result<Vec<DiscoveredParticipant>, ServerError> {
        Ok(self.participants.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn returns_static_list() {
        let list = vec![
            StaticParticipant {
                id: "supplier".into(),
                addr: "localhost:50091".into(),
            },
            StaticParticipant {
                id: "contract".into(),
                addr: "localhost:50092".into(),
            },
        ];
        let d = StaticDiscovery::from_static(&list);
        let out = d.discover().await.unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].id, "supplier");
        assert_eq!(out[1].endpoint, "localhost:50092");
    }

    #[tokio::test]
    async fn empty_list_returns_empty() {
        let d = StaticDiscovery::from_static(&[]);
        let out = d.discover().await.unwrap();
        assert!(out.is_empty());
    }
}
