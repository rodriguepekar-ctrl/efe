//! Genesis engine regression tests.
//!
//! Each test encodes a finding from the second audit pass.

use std::sync::Arc;

use efe_core::genesis::{
    BatchSpawner, Cardinality, DefaultSpawner, GenesisContext, HeuristicMetaAnalyzer, MetaAnalyzer,
    SpawnerConfig, StubTemplateRegistry, TaskClass, TaskDag, TaskMix, TemplateRegistry,
};
use efe_core::memory::{DeviceBackend, HostBackend, MemoryPool, PoolConfig};
use efe_core::streams::{StreamPool, StreamPoolConfig};
use efe_core::types::{AlignmentCone, LatentDim, PopulationMix, SwarmSpecBuilder, TemplateCode};
use efe_core::EfeError;

/// Workaround `#[non_exhaustive]`: build via `Default` and mutate.
fn cfg_with_sigma(sigma: f32) -> SpawnerConfig {
    let mut cfg = SpawnerConfig::default();
    cfg.init_sigma = sigma;
    cfg
}

fn cfg_with_margin(margin: f32) -> SpawnerConfig {
    let mut cfg = SpawnerConfig::default();
    cfg.hbm_margin_ratio = margin;
    cfg
}

// =====================================================================
// F1 - SpawnerConfig::init_sigma validation
// =====================================================================

#[test]
fn spawner_config_rejects_nan_init_sigma() {
    assert!(cfg_with_sigma(f32::NAN).validate().is_err());
}

#[test]
fn spawner_config_rejects_negative_init_sigma() {
    assert!(cfg_with_sigma(-0.01).validate().is_err());
}

#[test]
fn spawner_config_rejects_inf_init_sigma() {
    assert!(cfg_with_sigma(f32::INFINITY).validate().is_err());
}

#[test]
fn spawner_config_accepts_zero_init_sigma() {
    cfg_with_sigma(0.0).validate().expect("zero sigma is valid");
}

#[tokio::test]
async fn default_spawner_try_new_fails_on_invalid_config() {
    let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
    let pool =
        Arc::new(MemoryPool::new(PoolConfig::default().with_arena_size(1 << 14), backend).unwrap());
    let stream_pool = Arc::new(StreamPool::new(StreamPoolConfig::default().with_size(1)).unwrap());
    let registry: Arc<dyn TemplateRegistry> = Arc::new(StubTemplateRegistry::mvp_defaults(16));

    let cfg = cfg_with_sigma(f32::NAN);
    let res = DefaultSpawner::try_new(pool, registry, stream_pool, cfg);
    if let Err(err) = res {
        assert!(matches!(err, EfeError::InvalidConfig(_)));
    } else {
        panic!("try_new must fail on NaN init_sigma");
    }
}

// =====================================================================
// F2 - SpawnerConfig::hbm_margin_ratio validation
// =====================================================================

#[test]
fn spawner_config_rejects_nan_margin() {
    assert!(cfg_with_margin(f32::NAN).validate().is_err());
}

#[test]
fn spawner_config_rejects_negative_margin() {
    assert!(cfg_with_margin(-0.1).validate().is_err());
}

#[test]
fn spawner_config_rejects_absurd_margin() {
    assert!(cfg_with_margin(100.0).validate().is_err());
}

#[test]
fn spawner_config_accepts_reasonable_margins() {
    for margin in [0.0, 0.05, 0.10, 1.0, 2.0] {
        cfg_with_margin(margin)
            .validate()
            .expect("reasonable margin");
    }
}

// =====================================================================
// F3 - Cardinality must reject duplicate template codes
// =====================================================================

#[test]
fn cardinality_rejects_duplicate_template_codes() {
    let c = Cardinality {
        total: 5,
        per_template: vec![(TemplateCode::NaSem, 3), (TemplateCode::NaSem, 2)],
    };
    let err = c.validate().unwrap_err();
    assert!(matches!(err, EfeError::InvalidConfig(_)));
    assert!(format!("{err}").contains("duplicate"));
}

#[test]
fn cardinality_accepts_distinct_template_codes() {
    let c = Cardinality {
        total: 10,
        per_template: vec![
            (TemplateCode::NaSem, 5),
            (TemplateCode::NaLog, 3),
            (TemplateCode::NaCrit, 2),
        ],
    };
    c.validate().expect("distinct codes OK");
}

#[tokio::test]
async fn spawner_rejects_cardinality_with_duplicates() {
    let dim = 16;
    let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
    let pool = Arc::new(
        MemoryPool::new(
            PoolConfig::default()
                .with_max_arenas(1)
                .with_prealloc(1)
                .with_arena_size(1 << 16),
            backend,
        )
        .unwrap(),
    );
    let stream_pool = Arc::new(StreamPool::new(StreamPoolConfig::default().with_size(1)).unwrap());
    let registry: Arc<dyn TemplateRegistry> = Arc::new(StubTemplateRegistry::mvp_defaults(dim));
    let spawner = DefaultSpawner::new(pool, registry, stream_pool, SpawnerConfig::default());

    let bad = Cardinality {
        total: 4,
        per_template: vec![(TemplateCode::NaSem, 2), (TemplateCode::NaSem, 2)],
    };
    let spec = SwarmSpecBuilder::default()
        .problem_embedding(vec![0.0f32; dim as usize])
        .dim_latent(LatentDim::new(dim).unwrap())
        .population(PopulationMix::new(vec![(TemplateCode::NaSem, 4)]).unwrap())
        .alignment_cone(AlignmentCone::new(vec![vec![0.0f32; dim as usize]], 0.15).unwrap())
        .build()
        .unwrap();
    let dag = TaskDag::from_mix(
        TaskMix::one_hot(TaskClass::FactoidQa),
        Arc::from(vec![0.0f32; dim as usize]),
    );
    let ctx = GenesisContext::minimal(spec.id);

    let err = spawner
        .spawn(spec.id, &bad, &dag, &spec, &ctx)
        .await
        .unwrap_err();
    assert!(matches!(err, EfeError::InvalidConfig(_)));
}

// =====================================================================
// Additional invariants
// =====================================================================

#[tokio::test]
async fn spawner_default_config_is_valid() {
    // The default constructor uses `expect` — ensure it never panics.
    let dim = 16;
    let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
    let pool =
        Arc::new(MemoryPool::new(PoolConfig::default().with_arena_size(1 << 14), backend).unwrap());
    let stream_pool = Arc::new(StreamPool::new(StreamPoolConfig::default().with_size(1)).unwrap());
    let registry: Arc<dyn TemplateRegistry> = Arc::new(StubTemplateRegistry::mvp_defaults(dim));
    let _ = DefaultSpawner::new(pool, registry, stream_pool, SpawnerConfig::default());
}

#[test]
fn task_mix_uniform_has_argmax_first_class() {
    let mix = TaskMix::uniform();
    assert_eq!(mix.argmax(), TaskClass::FactoidQa);
}

#[tokio::test]
async fn heuristic_meta_returns_valid_mix_for_unusual_embeddings() {
    let ma = HeuristicMetaAnalyzer::load_default(32).unwrap();
    let emb: Arc<[f32]> = Arc::from(
        (0..32)
            .map(|i| (i as f32).cos() * 100.0)
            .collect::<Vec<_>>(),
    );
    let ctx = GenesisContext::minimal(efe_core::types::SwarmId::new());
    let dag = ma.decompose(emb, &ctx).await.unwrap();
    let sum: f32 = dag.task_mix.as_slice().iter().sum();
    assert!((sum - 1.0).abs() < 1e-4);
}
