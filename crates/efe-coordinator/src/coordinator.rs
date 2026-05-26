// SPDX-License-Identifier: UNLICENSED
//! `Coordinator` — pièce centrale : orchestre nano-robots, LLM gros, blackboard, bus, vote, speculator.

use std::sync::Arc;
use std::time::Duration;

use crate::blackboard::PheromoneBlackboard;
use crate::dag::DagExecutor;
use crate::error::{CoordError, CoordResult};
use crate::events::{EventBus, InMemoryBus};
use crate::llm::{MultiLlmBridge, ProviderRegistry};
use crate::vote::VoteEngine;
use crate::DEFAULT_MAX_CONCURRENCY;

use efe_nano_app_coordinator::{
    AnalyzerClient, CompressorClient, FilterClient, ReformulatorClient, RouterClient,
    VerifierClient,
};

/// Bundle des 6 clients nano-robots Phase 1.
///
/// Tous optionnels au boot : un Coordinator sans Verifier marche encore
/// (mais sans fact-checking).  Un Coordinator sans Reformulator marche
/// aussi (compresseur reçoit le texte brut).
pub struct CoordinatorClients {
    /// Analyseur (NER + intent + sentiment + langue).
    pub analyzer: Option<Arc<AnalyzerClient>>,
    /// Reformulateur (dialecte → canonique).
    pub reformulator: Option<Arc<ReformulatorClient>>,
    /// Compresseur (texte long → court).
    pub compressor: Option<Arc<CompressorClient>>,
    /// Vérificateur (fact-check).
    pub verifier: Option<Arc<VerifierClient>>,
    /// Filtreur (routing pertinence).
    pub filter: Option<Arc<FilterClient>>,
    /// Routeur (sélection LLM Pareto).
    pub router: Option<Arc<RouterClient>>,
}

impl Default for CoordinatorClients {
    fn default() -> Self {
        Self {
            analyzer: None,
            reformulator: None,
            compressor: None,
            verifier: None,
            filter: None,
            router: None,
        }
    }
}

impl CoordinatorClients {
    /// Builder vide.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Helper : tous les clients renseignés ?
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.analyzer.is_some()
            && self.reformulator.is_some()
            && self.compressor.is_some()
            && self.verifier.is_some()
            && self.filter.is_some()
            && self.router.is_some()
    }
}

/// Configuration du Coordinator.
#[derive(Debug, Clone)]
pub struct CoordinatorConfig {
    /// Parallélisme max du DAG executor.
    pub max_concurrency: usize,
    /// Deadline globale par défaut.
    pub default_deadline: Duration,
    /// Multi-tenant ID par défaut (peut être override per-call).
    pub default_tenant: uuid::Uuid,
}

impl Default for CoordinatorConfig {
    fn default() -> Self {
        Self {
            max_concurrency: DEFAULT_MAX_CONCURRENCY,
            default_deadline: Duration::from_secs(30),
            default_tenant: uuid::Uuid::nil(),
        }
    }
}

/// Pièce centrale du Phase 2 — orchestre tout.
///
/// Construit via [`CoordinatorBuilder`].
#[derive(Clone)]
pub struct Coordinator {
    /// Configuration.
    pub config: CoordinatorConfig,
    /// Clients nano-robots Phase 1.
    pub clients: Arc<CoordinatorClients>,
    /// Bridge multi-LLM.
    pub bridge: MultiLlmBridge,
    /// Blackboard partagée (shared par toutes les missions concurrentes).
    pub blackboard: PheromoneBlackboard,
    /// Bus d'évènements (in-process par défaut).
    pub bus: Arc<dyn EventBus>,
    /// Executeur DAG (réutilisable).
    pub dag: DagExecutor,
    /// Moteur de vote (par défaut uniform).
    pub vote: VoteEngine,
}

/// Builder fluide pour `Coordinator`.
pub struct CoordinatorBuilder {
    config: CoordinatorConfig,
    clients: CoordinatorClients,
    registry: Option<ProviderRegistry>,
    bus: Option<Arc<dyn EventBus>>,
    vote: Option<VoteEngine>,
}

impl Default for CoordinatorBuilder {
    fn default() -> Self {
        Self {
            config: CoordinatorConfig::default(),
            clients: CoordinatorClients::default(),
            registry: None,
            bus: None,
            vote: None,
        }
    }
}

impl CoordinatorBuilder {
    /// Configure.
    #[must_use]
    pub fn config(mut self, c: CoordinatorConfig) -> Self {
        self.config = c;
        self
    }

    /// Set des clients.
    #[must_use]
    pub fn clients(mut self, c: CoordinatorClients) -> Self {
        self.clients = c;
        self
    }

    /// Set du registre LLM (sinon vide).
    #[must_use]
    pub fn registry(mut self, r: ProviderRegistry) -> Self {
        self.registry = Some(r);
        self
    }

    /// Set du bus (sinon `InMemoryBus`).
    #[must_use]
    pub fn bus(mut self, b: Arc<dyn EventBus>) -> Self {
        self.bus = Some(b);
        self
    }

    /// Set du VoteEngine.
    #[must_use]
    pub fn vote(mut self, v: VoteEngine) -> Self {
        self.vote = Some(v);
        self
    }

    /// Build final.
    pub fn build(self) -> CoordResult<Coordinator> {
        if self.config.max_concurrency == 0 {
            return Err(CoordError::Config(
                "max_concurrency must be >= 1".into(),
            ));
        }
        let registry = self.registry.unwrap_or_default();
        let bridge = MultiLlmBridge::new(registry);
        let bus: Arc<dyn EventBus> = self.bus.unwrap_or_else(|| Arc::new(InMemoryBus::new()));
        Ok(Coordinator {
            dag: DagExecutor::new(self.config.max_concurrency),
            config: self.config,
            clients: Arc::new(self.clients),
            bridge,
            blackboard: PheromoneBlackboard::new(),
            bus,
            vote: self.vote.unwrap_or_else(VoteEngine::uniform),
        })
    }
}

impl Coordinator {
    /// Construit un builder.
    #[must_use]
    pub fn builder() -> CoordinatorBuilder {
        CoordinatorBuilder::default()
    }

    /// Coordinator minimal : aucun nano-robot, aucun provider, bus in-mem.
    /// Utile pour les tests qui ciblent un sous-composant.
    #[must_use]
    pub fn for_tests() -> Self {
        Self {
            config: CoordinatorConfig::default(),
            clients: Arc::new(CoordinatorClients::default()),
            bridge: MultiLlmBridge::new(ProviderRegistry::new()),
            blackboard: PheromoneBlackboard::new(),
            bus: Arc::new(InMemoryBus::new()),
            dag: DagExecutor::default(),
            vote: VoteEngine::uniform(),
        }
    }

    /// Souscrit au bus.  Helper pour les tests.
    pub fn subscribe(
        &self,
        name: Option<&str>,
    ) -> (
        crate::events::SubscriberToken,
        tokio::sync::mpsc::Receiver<crate::events::StreamEvent>,
    ) {
        self.bus.subscribe(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_default_works() {
        let c = Coordinator::builder().build().unwrap();
        assert_eq!(c.config.max_concurrency, DEFAULT_MAX_CONCURRENCY);
        assert!(c.blackboard.is_empty());
        assert_eq!(c.bridge.registry().len(), 0);
    }

    #[test]
    fn builder_with_zero_concurrency_errors() {
        let cfg = CoordinatorConfig {
            max_concurrency: 0,
            ..Default::default()
        };
        let r = Coordinator::builder().config(cfg).build();
        assert!(matches!(r, Err(CoordError::Config(_))));
    }

    #[test]
    fn for_tests_returns_valid_coordinator() {
        let c = Coordinator::for_tests();
        assert!(c.clients.analyzer.is_none());
        assert!(c.blackboard.is_empty());
    }

    #[test]
    fn clients_empty_is_default() {
        let cs = CoordinatorClients::empty();
        assert!(!cs.is_complete());
        assert!(cs.analyzer.is_none());
    }

    #[tokio::test]
    async fn subscribe_then_publish_via_bus() {
        let c = Coordinator::for_tests();
        let (_t, mut rx) = c.subscribe(Some("test"));
        let swarm = uuid::Uuid::now_v7();
        let ev = crate::events::StreamEvent::Heartbeat {
            ts: chrono::Utc::now(),
        };
        c.bus.publish(ev).await.unwrap();
        let _ = swarm;
        let got = rx.recv().await.unwrap();
        assert_eq!(got.tag(), "HEARTBEAT");
    }
}
