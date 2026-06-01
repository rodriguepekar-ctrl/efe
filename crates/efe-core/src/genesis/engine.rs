//! Genesis engine: orchestrates meta / router / cardinality / spawner.
//!
//! The engine implements [`crate::swarm::GenesisStrategy`] so [`crate::SwarmController`]
//! can drive it without knowing the internal pipeline. Each stage is
//! mockable via its own trait.

use std::sync::Arc;

use async_trait::async_trait;

use crate::els::EpistemicLatentSpace;
use crate::errors::{EfeError, Result};
use crate::genesis::cardinality::{CardinalityEstimator, HeuristicEstimator};
use crate::genesis::context::GenesisContext;
use crate::genesis::meta_analyzer::{HeuristicMetaAnalyzer, MetaAnalyzer};
use crate::genesis::registry::TemplateRegistry;
use crate::genesis::router::{StaticRouter, TemplateRouter};
use crate::genesis::spawner::{BatchSpawner, DefaultSpawner, SpawnerConfig};
use crate::memory::{MemoryPool, NanoAgentBatch};
use crate::streams::StreamPool;
use crate::swarm::GenesisStrategy;
use crate::types::SwarmSpec;

/// Full genesis pipeline assembled from four pluggable stages.
pub struct GenesisEngine {
    meta: Arc<dyn MetaAnalyzer>,
    router: Arc<dyn TemplateRouter>,
    cardinality: Arc<dyn CardinalityEstimator>,
    spawner: Arc<dyn BatchSpawner>,
}

impl GenesisEngine {
    /// Start a builder.
    #[must_use]
    pub fn builder() -> GenesisEngineBuilder {
        GenesisEngineBuilder::default()
    }
}

#[async_trait]
impl GenesisStrategy for GenesisEngine {
    async fn spawn(&self, spec: &SwarmSpec) -> Result<(NanoAgentBatch, EpistemicLatentSpace)> {
        let ctx = GenesisContext::minimal(spec.id);
        let embedding: Arc<[f32]> = Arc::from(spec.problem_embedding.clone());

        // 1. Meta.
        let dag = self.meta.decompose(embedding, &ctx).await?;
        tracing::debug!(
            primary = dag.primary_class.as_str(),
            entropy = dag.entropy_estimate,
            "genesis::meta complete"
        );

        // 2. Router.
        let distribution = self.router.select(&dag, &ctx)?;
        tracing::debug!(
            template_count = distribution.len(),
            "genesis::router complete"
        );

        // 3. Cardinality.
        let cardinality = self.cardinality.estimate(&dag, &distribution, &ctx)?;
        cardinality.validate()?;
        tracing::debug!(total = cardinality.total, "genesis::cardinality complete");

        // 4. Spawn.
        self.spawner
            .spawn(spec.id, &cardinality, &dag, spec, &ctx)
            .await
    }

    async fn respawn_diversify(&self, batch: &mut NanoAgentBatch, count: u32) -> Result<()> {
        self.spawner.respawn_diversify(batch, count).await
    }
}

/// Builder for [`GenesisEngine`].
#[derive(Default)]
pub struct GenesisEngineBuilder {
    meta: Option<Arc<dyn MetaAnalyzer>>,
    router: Option<Arc<dyn TemplateRouter>>,
    cardinality: Option<Arc<dyn CardinalityEstimator>>,
    spawner: Option<Arc<dyn BatchSpawner>>,
}

impl GenesisEngineBuilder {
    /// Override the meta-analyzer.
    #[must_use]
    pub fn meta(mut self, m: Arc<dyn MetaAnalyzer>) -> Self {
        self.meta = Some(m);
        self
    }

    /// Override the router.
    #[must_use]
    pub fn router(mut self, r: Arc<dyn TemplateRouter>) -> Self {
        self.router = Some(r);
        self
    }

    /// Override the cardinality estimator.
    #[must_use]
    pub fn cardinality(mut self, c: Arc<dyn CardinalityEstimator>) -> Self {
        self.cardinality = Some(c);
        self
    }

    /// Override the spawner.
    #[must_use]
    pub fn spawner(mut self, s: Arc<dyn BatchSpawner>) -> Self {
        self.spawner = Some(s);
        self
    }

    /// Build using MVP defaults for any unset stage.
    ///
    /// # Errors
    ///
    /// - [`EfeError::InvalidConfig`] if default meta-analyzer construction
    ///   fails (e.g. `dim == 0`).
    pub fn build_mvp(
        self,
        pool: Arc<MemoryPool>,
        registry: Arc<dyn TemplateRegistry>,
        stream_pool: Arc<StreamPool>,
    ) -> Result<GenesisEngine> {
        let dim = registry.dim_latent();
        let meta = self.meta.unwrap_or_else(|| {
            Arc::new(
                HeuristicMetaAnalyzer::load_default(dim)
                    .expect("heuristic meta-analyzer with valid dim"),
            )
        });
        if meta.expected_dim() != dim {
            return Err(EfeError::InvalidConfig(format!(
                "meta-analyzer dim {} != registry dim {dim}",
                meta.expected_dim()
            )));
        }
        let router = self.router.unwrap_or_else(|| {
            Arc::new(StaticRouter::mvp_defaults().expect("static router defaults"))
        });
        let cardinality = self
            .cardinality
            .unwrap_or_else(|| Arc::new(HeuristicEstimator::default()));
        let spawner = self.spawner.unwrap_or_else(|| {
            Arc::new(DefaultSpawner::new(
                pool,
                registry,
                stream_pool,
                SpawnerConfig::default(),
            ))
        });
        Ok(GenesisEngine {
            meta,
            router,
            cardinality,
            spawner,
        })
    }

    /// Build with all stages set manually.
    ///
    /// # Errors
    ///
    /// - [`EfeError::InvalidConfig`] if any required stage is missing.
    pub fn build(self) -> Result<GenesisEngine> {
        let meta = self
            .meta
            .ok_or_else(|| EfeError::InvalidConfig("GenesisEngine: meta required".into()))?;
        let router = self
            .router
            .ok_or_else(|| EfeError::InvalidConfig("GenesisEngine: router required".into()))?;
        let cardinality = self
            .cardinality
            .ok_or_else(|| EfeError::InvalidConfig("GenesisEngine: cardinality required".into()))?;
        let spawner = self
            .spawner
            .ok_or_else(|| EfeError::InvalidConfig("GenesisEngine: spawner required".into()))?;
        Ok(GenesisEngine {
            meta,
            router,
            cardinality,
            spawner,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genesis::registry::StubTemplateRegistry;
    use crate::memory::{DeviceBackend, HostBackend, PoolConfig};
    use crate::streams::StreamPoolConfig;
    use crate::types::{AlignmentCone, LatentDim, PopulationMix, SwarmSpecBuilder, TemplateCode};

    fn mk_engine(dim: u32) -> (GenesisEngine, SwarmSpec) {
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        let pool = Arc::new(
            MemoryPool::new(
                PoolConfig::default()
                    .with_max_arenas(2)
                    .with_prealloc(1)
                    .with_arena_size(1 << 20),
                backend,
            )
            .unwrap(),
        );
        let stream_pool =
            Arc::new(StreamPool::new(StreamPoolConfig::default().with_size(2)).unwrap());
        let registry: Arc<dyn TemplateRegistry> = Arc::new(StubTemplateRegistry::mvp_defaults(dim));
        let engine = GenesisEngine::builder()
            .build_mvp(pool, registry, stream_pool)
            .unwrap();
        let spec = SwarmSpecBuilder::default()
            .problem_embedding(vec![0.1f32; dim as usize])
            .dim_latent(LatentDim::new(dim).unwrap())
            .population(PopulationMix::new(vec![(TemplateCode::NaSem, 8)]).unwrap())
            .alignment_cone(AlignmentCone::new(vec![vec![0.0f32; dim as usize]], 0.15).unwrap())
            .build()
            .unwrap();
        (engine, spec)
    }

    #[tokio::test]
    async fn engine_full_pipeline_roundtrip() {
        let (engine, spec) = mk_engine(32);
        let (batch, els) = engine.spawn(&spec).await.unwrap();
        assert!(batch.count() >= 128); // heuristic estimator min
        assert!(els.is_populated());
    }

    #[tokio::test]
    async fn engine_deterministic_same_spec() {
        let (engine, mut spec1) = mk_engine(16);
        let spec2 = SwarmSpec {
            id: spec1.id, // same
            ..spec1.clone()
        };
        spec1 = spec2.clone();
        let (batch1, _) = engine.spawn(&spec1).await.unwrap();
        let (batch2, _) = engine.spawn(&spec2).await.unwrap();
        assert_eq!(batch1.count(), batch2.count());
        assert_eq!(batch1.templates().len(), batch2.templates().len());
    }

    #[tokio::test]
    async fn engine_build_mvp_rejects_dim_mismatch() {
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        let pool = Arc::new(
            MemoryPool::new(PoolConfig::default().with_arena_size(1 << 16), backend).unwrap(),
        );
        let stream_pool =
            Arc::new(StreamPool::new(StreamPoolConfig::default().with_size(1)).unwrap());
        let registry: Arc<dyn TemplateRegistry> = Arc::new(StubTemplateRegistry::mvp_defaults(64));
        // Override meta with a different dim → should fail validation.
        let meta: Arc<dyn MetaAnalyzer> =
            Arc::new(HeuristicMetaAnalyzer::load_default(128).unwrap());
        let res = GenesisEngine::builder()
            .meta(meta)
            .build_mvp(pool, registry, stream_pool);
        assert!(res.is_err());
    }
}
