// SPDX-License-Identifier: UNLICENSED
//! Registre thread-safe de providers — recherche par `ProviderId`.

use std::sync::Arc;

use dashmap::DashMap;

use super::provider::{LlmProvider, ProviderId};

/// Registre des providers (handles partagés Arc).
///
/// Une instance est partagée par tous les workflows.  Les providers
/// sont enregistrés une fois au boot du Coordinator.
#[derive(Clone, Default)]
pub struct ProviderRegistry {
    inner: Arc<DashMap<ProviderId, Arc<dyn LlmProvider>>>,
}

impl ProviderRegistry {
    /// Registre vide.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Ajoute (ou écrase) un provider.
    pub fn register(&self, provider: Arc<dyn LlmProvider>) {
        let id = provider.id();
        self.inner.insert(id, provider);
    }

    /// Récupère un provider par ID.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<Arc<dyn LlmProvider>> {
        self.inner.get(id).map(|p| p.value().clone())
    }

    /// Liste des IDs enregistrés.
    #[must_use]
    pub fn ids(&self) -> Vec<ProviderId> {
        self.inner.iter().map(|e| e.key().clone()).collect()
    }

    /// Nombre de providers.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Aucun provider enregistré ?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::mock::MockLlmProvider;

    #[test]
    fn register_then_get() {
        let r = ProviderRegistry::new();
        r.register(Arc::new(MockLlmProvider::new("claude")));
        r.register(Arc::new(MockLlmProvider::new("gpt4")));
        assert_eq!(r.len(), 2);
        assert!(r.get("claude").is_some());
        assert!(r.get("nope").is_none());
        let mut ids = r.ids();
        ids.sort();
        assert_eq!(ids, vec!["claude".to_string(), "gpt4".to_string()]);
    }

    #[test]
    fn register_overwrites_same_id() {
        let r = ProviderRegistry::new();
        r.register(Arc::new(MockLlmProvider::new("x").with_cost_cents(1)));
        r.register(Arc::new(MockLlmProvider::new("x").with_cost_cents(99)));
        assert_eq!(r.len(), 1);
    }
}
