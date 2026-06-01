//! Batch spawner: instantiates a fresh swarm in HBM.
//!
//! The spawner is the only stateful stage of the genesis pipeline. It
//! composes three external services — an [`MemoryPool`] (M2), a
//! [`TemplateRegistry`] (M8), and a [`StreamPool`] (M2) — to:
//!
//! 1. Acquire an arena sized for the target population.
//! 2. Allocate the four SoA tensors (latent, mask, anchors, problem
//!    embedding).
//! 3. Upload the per-template priors, the per-agent template indices,
//!    the alignment anchors, and the problem embedding.
//! 4. Launch the `spawn_batch` kernel to materialize
//!    `h[i] = prior[template[i]] + sigma * N(0, 1)`.
//! 5. Package everything into a [`NanoAgentBatch`] + [`EpistemicLatentSpace`].

use std::sync::Arc;

use async_trait::async_trait;
use half::bf16;

use crate::els::EpistemicLatentSpace;
use crate::errors::{EfeError, Result};
use crate::genesis::cardinality::Cardinality;
use crate::genesis::context::GenesisContext;
use crate::genesis::dag::TaskDag;
use crate::genesis::registry::TemplateRegistry;
use crate::genesis::spawn_kernel::{launch_spawn_batch, SpawnBatchArgs};
use crate::memory::{DevicePtr, MemoryPool, NanoAgentBatch, RawAllocation};
use crate::streams::StreamPool;
use crate::types::{NanoAgentId, SwarmId, SwarmSpec, TemplateCode};

/// Spawner configuration knobs.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct SpawnerConfig {
    /// Noise std-dev in `h_i = prior + sigma * N(0,1)`.
    pub init_sigma: f32,
    /// Mixed into the PRNG seed to avoid cross-deployment determinism leaks.
    pub seed_salt: u64,
    /// Margin added on top of the computed HBM requirement (fraction).
    pub hbm_margin_ratio: f32,
}

impl Default for SpawnerConfig {
    fn default() -> Self {
        Self {
            init_sigma: 0.02,
            seed_salt: 0xEFE5_EEDD_1234_ABCD,
            hbm_margin_ratio: 0.10,
        }
    }
}

impl SpawnerConfig {
    /// Validate configuration values.
    ///
    /// # Errors
    ///
    /// - [`EfeError::InvalidConfig`] if `init_sigma` is not finite or
    ///   negative, or if `hbm_margin_ratio` is not finite, negative,
    ///   or unreasonably large (> 10.0, which would triple-count HBM).
    pub fn validate(&self) -> Result<()> {
        if !self.init_sigma.is_finite() || self.init_sigma < 0.0 {
            return Err(EfeError::InvalidConfig(format!(
                "SpawnerConfig: init_sigma {} must be finite and >= 0",
                self.init_sigma
            )));
        }
        if !self.hbm_margin_ratio.is_finite()
            || self.hbm_margin_ratio < 0.0
            || self.hbm_margin_ratio > 10.0
        {
            return Err(EfeError::InvalidConfig(format!(
                "SpawnerConfig: hbm_margin_ratio {} must be in [0.0, 10.0]",
                self.hbm_margin_ratio
            )));
        }
        Ok(())
    }
}

/// Size of the bf16 (used for latent + priors).
const BF16_BYTES: u64 = 2;
/// Size of f32 (used for anchors + problem embedding).
const F32_BYTES: u64 = 4;

/// Allocates HBM and populates a fresh swarm.
#[async_trait]
pub trait BatchSpawner: Send + Sync {
    /// Spawn a new population.
    ///
    /// # Errors
    ///
    /// Propagates [`EfeError::HbmAllocFailed`] on pool saturation,
    /// [`EfeError::TemplateMissing`] when a required template is absent
    /// from the registry, and any kernel failure.
    async fn spawn(
        &self,
        swarm_id: SwarmId,
        cardinality: &Cardinality,
        dag: &TaskDag,
        spec: &SwarmSpec,
        ctx: &GenesisContext,
    ) -> Result<(NanoAgentBatch, EpistemicLatentSpace)>;

    /// Respawn a subset of agents in place (for mode-collapse recovery).
    ///
    /// # Errors
    ///
    /// Propagates kernel failures.
    async fn respawn_diversify(&self, batch: &mut NanoAgentBatch, count: u32) -> Result<()>;
}

/// Default MVP spawner backed by the `HostBackend` + CPU kernel.
pub struct DefaultSpawner {
    pool: Arc<MemoryPool>,
    registry: Arc<dyn TemplateRegistry>,
    stream_pool: Arc<StreamPool>,
    config: SpawnerConfig,
}

impl DefaultSpawner {
    /// Build with explicit dependencies and config.
    ///
    /// # Panics
    ///
    /// Panics if `config` fails [`SpawnerConfig::validate`]. Use
    /// [`Self::try_new`] if you need to handle configuration errors.
    #[must_use]
    pub fn new(
        pool: Arc<MemoryPool>,
        registry: Arc<dyn TemplateRegistry>,
        stream_pool: Arc<StreamPool>,
        config: SpawnerConfig,
    ) -> Self {
        Self::try_new(pool, registry, stream_pool, config)
            .expect("DefaultSpawner: invalid SpawnerConfig")
    }

    /// Fallible constructor that validates [`SpawnerConfig`].
    ///
    /// # Errors
    ///
    /// Propagates [`SpawnerConfig::validate`] failures.
    pub fn try_new(
        pool: Arc<MemoryPool>,
        registry: Arc<dyn TemplateRegistry>,
        stream_pool: Arc<StreamPool>,
        config: SpawnerConfig,
    ) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            pool,
            registry,
            stream_pool,
            config,
        })
    }

    /// Compute the HBM requirement for a population (with margin).
    fn hbm_bytes_needed(&self, n: u32, d: u32, n_anchors: u32) -> u64 {
        let latent = u64::from(n) * u64::from(d) * BF16_BYTES;
        let mask = u64::from(n);
        let anchors = u64::from(n_anchors) * u64::from(d) * F32_BYTES;
        let priors =
            u64::from(self.registry.available_templates().len() as u32) * u64::from(d) * BF16_BYTES;
        let templates_buf = u64::from(n);
        let problem_emb = u64::from(d) * F32_BYTES;
        // Reserve alignment slack via the configured margin.
        let subtotal = latent + mask + anchors + priors + templates_buf + problem_emb;
        #[allow(clippy::cast_precision_loss)]
        let subtotal_f = subtotal as f64;
        let margin_f = subtotal_f * f64::from(self.config.hbm_margin_ratio);
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let margin = margin_f.round() as u64;
        subtotal + margin
    }

    /// Build the per-agent template buffer (one byte per agent).
    ///
    /// Returns the vector of *local* template indices (into the
    /// registry priors vector the caller will produce) plus the
    /// TemplateCode slice for the batch.
    fn assemble_template_indices(
        cardinality: &Cardinality,
    ) -> Result<(Vec<u8>, Vec<TemplateCode>, Vec<TemplateCode>)> {
        // 1. Distinct templates in the order the registry exposes them.
        //    We compact them into a registry-local index space for the
        //    kernel.
        let mut unique: Vec<TemplateCode> = Vec::new();
        for (code, _) in &cardinality.per_template {
            if !unique.contains(code) {
                unique.push(*code);
            }
        }
        // 2. Build the per-agent index vector.
        let mut indices = Vec::with_capacity(cardinality.total as usize);
        let mut codes = Vec::with_capacity(cardinality.total as usize);
        for (code, count) in &cardinality.per_template {
            if *count == 0 {
                continue;
            }
            let local = unique
                .iter()
                .position(|c| c == code)
                .ok_or(EfeError::Internal(
                    "assemble_template_indices: code missing",
                ))?;
            let local_u8 = u8::try_from(local)
                .map_err(|_| EfeError::Internal("assemble_template_indices: too many templates"))?;
            for _ in 0..*count {
                indices.push(local_u8);
                codes.push(*code);
            }
        }
        Ok((indices, codes, unique))
    }

    /// Deterministic seed per swarm (ties into `config.seed_salt`).
    fn compute_seed(&self, swarm_id: SwarmId) -> u64 {
        // Mix the swarm UUID bytes with the salt.
        let uuid_bytes = swarm_id.as_uuid().as_u128();
        let lo = uuid_bytes as u64;
        let hi = (uuid_bytes >> 64) as u64;
        lo ^ hi.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ self.config.seed_salt
    }
}

#[async_trait]
impl BatchSpawner for DefaultSpawner {
    async fn spawn(
        &self,
        swarm_id: SwarmId,
        cardinality: &Cardinality,
        dag: &TaskDag,
        spec: &SwarmSpec,
        ctx: &GenesisContext,
    ) -> Result<(NanoAgentBatch, EpistemicLatentSpace)> {
        cardinality.validate()?;
        let n = cardinality.total;
        let d = spec.dim_latent.get();
        if n == 0 {
            return Err(EfeError::InvalidConfig(
                "DefaultSpawner: cardinality total is 0".into(),
            ));
        }

        let anchors = &spec.alignment_cone.anchors;
        let n_anchors = u32::try_from(anchors.len())
            .map_err(|_| EfeError::InvalidConfig("too many anchors".into()))?;

        // Validate that every anchor has the right dim.
        for (i, anchor) in anchors.iter().enumerate() {
            if u32::try_from(anchor.len()).ok() != Some(d) {
                return Err(EfeError::InvalidConfig(format!(
                    "anchor {i} has dim {} != {d}",
                    anchor.len()
                )));
            }
        }

        let required = self.hbm_bytes_needed(n, d, n_anchors);
        tracing::debug!(
            swarm_id = %swarm_id,
            n, d, n_anchors, required,
            "genesis::spawner: acquiring arena"
        );

        // --- 1. Acquire arena and derive primary allocations. ---
        let handle = self.pool.acquire(required, ctx.deadline).await?;
        let arena = handle.arena();
        let latent_raw = arena.alloc_zeroed_raw(u64::from(n) * u64::from(d) * BF16_BYTES)?;
        let mask_raw = arena.alloc_zeroed_raw(u64::from(n))?;
        let anchors_raw =
            arena.alloc_zeroed_raw(u64::from(n_anchors) * u64::from(d) * F32_BYTES)?;
        let problem_raw = arena.alloc_zeroed_raw(u64::from(d) * F32_BYTES)?;

        // --- 2. Assemble per-agent template indices. ---
        let (template_indices, agent_codes, unique_codes) =
            Self::assemble_template_indices(cardinality)?;
        let templates_raw = arena.alloc_zeroed_raw(u64::from(n))?;
        // SAFETY: the host backend exposes a writable host pointer; we
        // own the arena slice and template_indices has exactly `n` bytes.
        unsafe {
            std::ptr::copy_nonoverlapping(
                template_indices.as_ptr(),
                templates_raw.ptr.as_raw().as_ptr(),
                n as usize,
            );
        }

        // --- 3. Build per-unique-template prior buffer (K, D) bf16. ---
        let k = u32::try_from(unique_codes.len())
            .map_err(|_| EfeError::Internal("too many unique templates"))?;
        let priors_raw = arena.alloc_zeroed_raw(u64::from(k) * u64::from(d) * BF16_BYTES)?;
        self.write_priors(&unique_codes, d, priors_raw)?;

        // --- 4. Upload anchors (host → device). ---
        Self::write_anchors(anchors, d, anchors_raw)?;

        // --- 5. Upload problem embedding. ---
        Self::write_problem_embedding(&dag.problem_embedding, d, problem_raw)?;

        // --- 6. Launch spawn_batch kernel. ---
        let stream = self.stream_pool.acquire().await?;
        let seed = self.compute_seed(swarm_id);
        let args = SpawnBatchArgs {
            latent: latent_raw.ptr,
            templates: templates_raw.ptr,
            priors: priors_raw.ptr,
            mask: mask_raw.ptr,
            n,
            d,
            k,
            seed,
            sigma: self.config.init_sigma,
        };
        launch_spawn_batch(args, &stream)?;
        stream.synchronize().await?;

        // --- 7. Package the result. ---
        let ids = assemble_ids(n);
        let batch = NanoAgentBatch::new(swarm_id, handle, agent_codes, ids);
        let els = EpistemicLatentSpace::new_populated_with_cone(
            swarm_id,
            spec.dim_latent,
            n,
            latent_raw,
            mask_raw,
            anchors_raw,
            problem_raw,
            n_anchors,
            spec.alignment_cone.max_projection_distance,
        );

        tracing::debug!(
            swarm_id = %swarm_id,
            hbm_bytes = required,
            "genesis::spawner: spawn complete"
        );
        Ok((batch, els))
    }

    async fn respawn_diversify(&self, batch: &mut NanoAgentBatch, _count: u32) -> Result<()> {
        // MVP: stub implementation. We log and return Ok so the
        // controller's recovery path can proceed. Real respawn
        // re-initializes the last `count` rows; plumbed in Sprint 3.
        tracing::warn!(
            swarm_id = %batch.swarm_id(),
            "respawn_diversify: MVP stub (no-op)"
        );
        Ok(())
    }
}

impl DefaultSpawner {
    fn write_priors(
        &self,
        unique_codes: &[TemplateCode],
        d: u32,
        dst: RawAllocation,
    ) -> Result<()> {
        let d_usize = d as usize;
        let mut bf16_buf: Vec<bf16> = Vec::with_capacity(unique_codes.len() * d_usize);
        for code in unique_codes {
            let prior = self.registry.prior_embedding(*code)?;
            if prior.len() != d_usize {
                return Err(EfeError::InvalidConfig(format!(
                    "prior for {} has dim {} != {d}",
                    code.as_str(),
                    prior.len()
                )));
            }
            bf16_buf.extend(prior.iter().map(|f| bf16::from_f32(*f)));
        }
        // SAFETY: `dst` is an arena-backed allocation of size
        // `k * d * BF16_BYTES`. We copy exactly that many bytes.
        unsafe {
            let bytes_ptr = bf16_buf.as_ptr().cast::<u8>();
            let total_bytes = bf16_buf.len() * std::mem::size_of::<bf16>();
            std::ptr::copy_nonoverlapping(bytes_ptr, dst.ptr.as_raw().as_ptr(), total_bytes);
        }
        Ok(())
    }

    fn write_anchors(anchors: &[Vec<f32>], d: u32, dst: RawAllocation) -> Result<()> {
        let d_usize = d as usize;
        // SAFETY: `dst` is an arena-backed allocation sized for
        // `n_anchors * d * F32_BYTES` bytes. We write row-major.
        unsafe {
            let base: *mut f32 = dst.ptr.as_raw().cast::<f32>().as_ptr();
            for (i, anchor) in anchors.iter().enumerate() {
                if anchor.len() != d_usize {
                    return Err(EfeError::InvalidConfig(format!(
                        "anchor {i}: len {} != {d}",
                        anchor.len()
                    )));
                }
                std::ptr::copy_nonoverlapping(anchor.as_ptr(), base.add(i * d_usize), d_usize);
            }
        }
        Ok(())
    }

    fn write_problem_embedding(emb: &[f32], d: u32, dst: RawAllocation) -> Result<()> {
        let d_usize = d as usize;
        if emb.len() != d_usize {
            return Err(EfeError::InvalidConfig(format!(
                "problem embedding len {} != {d}",
                emb.len()
            )));
        }
        // SAFETY: `dst` sized exactly `d * F32_BYTES` bytes.
        unsafe {
            let base: *mut f32 = dst.ptr.as_raw().cast::<f32>().as_ptr();
            std::ptr::copy_nonoverlapping(emb.as_ptr(), base, d_usize);
        }
        Ok(())
    }
}

// Silence unused-import warning when DevicePtr is only referenced via
// the kernel args struct.
#[allow(dead_code)]
fn _device_ptr_keep(_: DevicePtr) {}

fn assemble_ids(n: u32) -> Vec<NanoAgentId> {
    (0..n).map(NanoAgentId).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genesis::dag::{TaskClass, TaskMix};
    use crate::genesis::registry::StubTemplateRegistry;
    use crate::memory::{DeviceBackend, HostBackend, PoolConfig};
    use crate::streams::StreamPoolConfig;
    use crate::types::{AlignmentCone, LatentDim, PopulationMix, SwarmSpecBuilder};

    fn mk_pools(dim: u32) -> (Arc<MemoryPool>, Arc<StreamPool>, Arc<dyn TemplateRegistry>) {
        let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
        let pool = MemoryPool::new(
            PoolConfig::default()
                .with_max_arenas(2)
                .with_prealloc(1)
                .with_arena_size(1 << 20),
            backend,
        )
        .expect("pool");
        let stream_pool =
            StreamPool::new(StreamPoolConfig::default().with_size(2)).expect("streams");
        let registry: Arc<dyn TemplateRegistry> = Arc::new(StubTemplateRegistry::mvp_defaults(dim));
        (Arc::new(pool), Arc::new(stream_pool), registry)
    }

    fn mk_spec(dim: u32, total: u32) -> SwarmSpec {
        SwarmSpecBuilder::default()
            .problem_embedding(vec![0.1f32; dim as usize])
            .dim_latent(LatentDim::new(dim).unwrap())
            .population(PopulationMix::new(vec![(TemplateCode::NaSem, total)]).unwrap())
            .alignment_cone(AlignmentCone::new(vec![vec![0.0f32; dim as usize]], 0.15).unwrap())
            .build()
            .unwrap()
    }

    fn mk_cardinality(total: u32) -> Cardinality {
        Cardinality {
            total,
            per_template: vec![(TemplateCode::NaSem, total)],
        }
    }

    fn mk_dag(dim: u32) -> TaskDag {
        TaskDag::from_mix(
            TaskMix::one_hot(TaskClass::FactoidQa),
            Arc::from(vec![0.1f32; dim as usize]),
        )
    }

    fn mk_ctx(id: SwarmId) -> GenesisContext {
        GenesisContext::minimal(id)
    }

    #[tokio::test]
    async fn spawner_produces_batch_of_correct_size() {
        let dim = 32;
        let total = 8;
        let (pool, streams, registry) = mk_pools(dim);
        let spawner = DefaultSpawner::new(pool, registry, streams, SpawnerConfig::default());

        let spec = mk_spec(dim, total);
        let (batch, els) = spawner
            .spawn(
                spec.id,
                &mk_cardinality(total),
                &mk_dag(dim),
                &spec,
                &mk_ctx(spec.id),
            )
            .await
            .unwrap();

        assert_eq!(batch.count(), total);
        assert_eq!(batch.templates().len(), total as usize);
        assert!(batch.has_arena());
        assert!(els.is_populated());
        assert_eq!(els.agent_count(), total);
        assert_eq!(els.anchor_count(), 1);
    }

    #[tokio::test]
    async fn spawner_is_deterministic_for_same_swarm_id() {
        let dim = 16;
        let total = 4;
        let (pool, streams, registry) = mk_pools(dim);
        let spawner = Arc::new(DefaultSpawner::new(
            pool,
            registry,
            streams,
            SpawnerConfig {
                init_sigma: 0.1,
                ..Default::default()
            },
        ));

        let spec1 = mk_spec(dim, total);
        let mut spec2 = mk_spec(dim, total);
        spec2.id = spec1.id;

        let (batch1, els1) = spawner
            .spawn(
                spec1.id,
                &mk_cardinality(total),
                &mk_dag(dim),
                &spec1,
                &mk_ctx(spec1.id),
            )
            .await
            .unwrap();
        let (batch2, els2) = spawner
            .spawn(
                spec2.id,
                &mk_cardinality(total),
                &mk_dag(dim),
                &spec2,
                &mk_ctx(spec2.id),
            )
            .await
            .unwrap();

        assert_eq!(batch1.count(), batch2.count());
        // Compare the latent contents via raw pointers (HostBackend).
        let ptr1 = els1.latent_raw().unwrap().ptr;
        let ptr2 = els2.latent_raw().unwrap().ptr;
        // SAFETY: both pointers are valid host-backed memory of size n*d*2.
        unsafe {
            let bytes = (total as usize) * (dim as usize) * 2;
            let a = std::slice::from_raw_parts(ptr1.as_raw().as_ptr(), bytes);
            let b = std::slice::from_raw_parts(ptr2.as_raw().as_ptr(), bytes);
            assert_eq!(a, b, "determinism violated for same swarm_id");
        }
    }

    #[tokio::test]
    async fn spawner_rejects_mismatched_anchor_dim() {
        let dim = 16;
        let total = 4;
        let (pool, streams, registry) = mk_pools(dim);
        let spawner = DefaultSpawner::new(pool, registry, streams, SpawnerConfig::default());

        let mut spec = mk_spec(dim, total);
        spec.alignment_cone = AlignmentCone::new(vec![vec![0.0f32; 5]], 0.15).unwrap();
        let res = spawner
            .spawn(
                spec.id,
                &mk_cardinality(total),
                &mk_dag(dim),
                &spec,
                &mk_ctx(spec.id),
            )
            .await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn spawner_rejects_zero_total_cardinality() {
        let dim = 16;
        let (pool, streams, registry) = mk_pools(dim);
        let spawner = DefaultSpawner::new(pool, registry, streams, SpawnerConfig::default());

        let spec = mk_spec(dim, 8);
        let bad = Cardinality {
            total: 0,
            per_template: vec![(TemplateCode::NaSem, 0)],
        };
        let err = spawner
            .spawn(spec.id, &bad, &mk_dag(dim), &spec, &mk_ctx(spec.id))
            .await
            .unwrap_err();
        assert!(matches!(err, EfeError::InvalidConfig(_)));
    }

    #[tokio::test]
    async fn spawner_releases_arena_on_drop() {
        let dim = 16;
        let total = 4;
        let (pool, streams, registry) = mk_pools(dim);
        let spawner =
            DefaultSpawner::new(pool.clone(), registry, streams, SpawnerConfig::default());

        let spec = mk_spec(dim, total);
        {
            let _ = spawner
                .spawn(
                    spec.id,
                    &mk_cardinality(total),
                    &mk_dag(dim),
                    &spec,
                    &mk_ctx(spec.id),
                )
                .await
                .unwrap();
        }
        let m = pool.metrics();
        assert_eq!(m.arenas_active, 0);
    }

    #[tokio::test]
    async fn respawn_diversify_is_noop_in_mvp() {
        let dim = 16;
        let total = 8;
        let (pool, streams, registry) = mk_pools(dim);
        let spawner = DefaultSpawner::new(pool, registry, streams, SpawnerConfig::default());

        let spec = mk_spec(dim, total);
        let (mut batch, _els) = spawner
            .spawn(
                spec.id,
                &mk_cardinality(total),
                &mk_dag(dim),
                &spec,
                &mk_ctx(spec.id),
            )
            .await
            .unwrap();
        spawner.respawn_diversify(&mut batch, 2).await.unwrap();
    }
}
