//! `spawn_batch` kernel — CPU reference implementation.
//!
//! Populates the latent tensor of a fresh swarm with
//! `h[i] = prior[template[i]] + noise`. The MVP ships a CPU reference
//! (host-backend only). Sprint 3+ will add a real CUDA kernel behind
//! the same [`launch_spawn_batch`] signature via a pluggable trait.
//!
//! # Layout assumptions (SoA)
//!
//! - `latent`: `[N, D]` row-major, `bf16` (2 bytes/elem).
//! - `templates`: `[N]` `u8` — index into `0..k`.
//! - `priors`: `[K, D]` row-major, `bf16`.
//! - `mask`: `[N]` `u8` — set to `1` for every spawned agent.

use half::bf16;
use rand::Rng;
use rand_distr::StandardNormal;
use rand_pcg::Pcg64;

use crate::errors::{EfeError, Result};
use crate::memory::DevicePtr;
use crate::streams::StreamHandle;

/// Arguments for [`launch_spawn_batch`].
#[derive(Clone, Copy, Debug)]
pub struct SpawnBatchArgs {
    /// `[N, D]` bf16 tensor the kernel writes to.
    pub latent: DevicePtr,
    /// `[N]` u8 tensor: per-agent template index in `0..k`.
    pub templates: DevicePtr,
    /// `[K, D]` bf16 priors.
    pub priors: DevicePtr,
    /// `[N]` u8 active mask (written to all-ones).
    pub mask: DevicePtr,
    /// Number of agents (`N`).
    pub n: u32,
    /// Latent dimension (`D`).
    pub d: u32,
    /// Number of priors (`K`).
    pub k: u32,
    /// PRNG seed — deterministic per swarm.
    pub seed: u64,
    /// Noise standard deviation. Zero → pure prior.
    pub sigma: f32,
}

/// Launch the spawn kernel.
///
/// # Errors
///
/// - [`EfeError::InvalidConfig`] on inconsistent sizes or a
///   `templates[i] >= k` entry.
///
/// # Safety
///
/// In MVP the pointers are host-backend pointers produced by
/// [`crate::memory::Arena::alloc_zeroed_raw`]. The caller must:
///
/// 1. Ensure each pointer is valid for the declared length.
/// 2. Not concurrently mutate the pointed regions.
/// 3. Not call this function after the parent arena has been reset
///    (the spawner guarantees this by keeping the `ArenaHandle` alive).
pub fn launch_spawn_batch(args: SpawnBatchArgs, _stream: &StreamHandle) -> Result<()> {
    if args.n == 0 {
        return Ok(());
    }
    if args.d == 0 {
        return Err(EfeError::InvalidConfig("spawn_batch: d must be > 0".into()));
    }
    if args.k == 0 {
        return Err(EfeError::InvalidConfig("spawn_batch: k must be > 0".into()));
    }
    if !args.sigma.is_finite() || args.sigma < 0.0 {
        return Err(EfeError::InvalidConfig(format!(
            "spawn_batch: sigma {} must be finite and >= 0",
            args.sigma
        )));
    }

    // SAFETY: caller guarantees each pointer is valid for the declared
    // length. See function safety doc.
    unsafe {
        let n = args.n as usize;
        let d = args.d as usize;
        let k = args.k as usize;

        let latent_ptr: *mut bf16 = args.latent.as_raw().cast::<bf16>().as_ptr();
        let templates_ptr = args.templates.as_raw().as_ptr();
        let priors_ptr: *const bf16 = args.priors.as_raw().cast::<bf16>().as_ptr();
        let mask_ptr = args.mask.as_raw().as_ptr();

        let latent = std::slice::from_raw_parts_mut(latent_ptr, n * d);
        let templates = std::slice::from_raw_parts(templates_ptr, n);
        let priors = std::slice::from_raw_parts(priors_ptr, k * d);
        let mask = std::slice::from_raw_parts_mut(mask_ptr, n);

        spawn_batch_cpu(latent, templates, priors, mask, args.seed, args.sigma)
    }
}

/// CPU reference implementation.
///
/// # Errors
///
/// - [`EfeError::InvalidConfig`] on length mismatches or template
///   indices out of range.
pub fn spawn_batch_cpu(
    latent: &mut [bf16],
    templates: &[u8],
    priors: &[bf16],
    mask: &mut [u8],
    seed: u64,
    sigma: f32,
) -> Result<()> {
    let n = templates.len();
    if n == 0 {
        return Ok(());
    }
    if latent.len() % n != 0 {
        return Err(EfeError::InvalidConfig(format!(
            "spawn_batch: latent len {} not divisible by n {n}",
            latent.len()
        )));
    }
    let d = latent.len() / n;
    if d == 0 {
        return Err(EfeError::InvalidConfig(
            "spawn_batch: derived d == 0".into(),
        ));
    }
    if priors.len() % d != 0 {
        return Err(EfeError::InvalidConfig(format!(
            "spawn_batch: priors len {} not divisible by d {d}",
            priors.len()
        )));
    }
    let k = priors.len() / d;
    if k == 0 {
        return Err(EfeError::InvalidConfig(
            "spawn_batch: priors provide 0 templates".into(),
        ));
    }
    if mask.len() != n {
        return Err(EfeError::InvalidConfig(format!(
            "spawn_batch: mask len {} != n {n}",
            mask.len()
        )));
    }

    let mut rng = Pcg64::new(
        u128::from(seed),
        0xA02B_DBF7_BB3C_0A7D_1234_5678_9ABC_DEF0_u128,
    );
    for (agent_idx, t) in templates.iter().enumerate() {
        let t_idx = *t as usize;
        if t_idx >= k {
            return Err(EfeError::InvalidConfig(format!(
                "spawn_batch: template index {t_idx} out of range (k={k})"
            )));
        }
        let prior_offset = t_idx * d;
        let row_offset = agent_idx * d;
        for dim in 0..d {
            let prior_val = priors[prior_offset + dim].to_f32();
            let noise: f32 = if sigma > 0.0 {
                let raw: f32 = rng.sample(StandardNormal);
                raw * sigma
            } else {
                0.0
            };
            latent[row_offset + dim] = bf16::from_f32(prior_val + noise);
        }
        mask[agent_idx] = 1;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_priors(k: usize, d: usize) -> Vec<bf16> {
        let mut out = Vec::with_capacity(k * d);
        for class in 0..k {
            for dim in 0..d {
                out.push(bf16::from_f32((class as f32 + 1.0) + 0.01 * dim as f32));
            }
        }
        out
    }

    #[test]
    fn cpu_writes_all_rows_with_sigma_zero() {
        let n_agents = 8usize;
        let dim = 4usize;
        let n_templates = 3usize;
        let templates = vec![0u8, 1, 2, 0, 1, 2, 0, 1];
        let priors = mk_priors(n_templates, dim);
        let mut latent = vec![bf16::from_f32(0.0); n_agents * dim];
        let mut mask = vec![0u8; n_agents];

        spawn_batch_cpu(&mut latent, &templates, &priors, &mut mask, 42, 0.0).unwrap();
        for agent in 0..n_agents {
            let t = templates[agent] as usize;
            for d_i in 0..dim {
                let got = latent[agent * dim + d_i].to_f32();
                let want = priors[t * dim + d_i].to_f32();
                assert!(
                    (got - want).abs() < 0.05,
                    "row {agent} dim {d_i}: got {got}, want {want}"
                );
            }
            assert_eq!(mask[agent], 1);
        }
    }

    #[test]
    fn cpu_deterministic_same_seed() {
        let n = 16usize;
        let d = 8usize;
        let k = 2usize;
        let templates = vec![0u8; n];
        let priors = mk_priors(k, d);
        let (mut l1, mut m1) = (vec![bf16::from_f32(0.0); n * d], vec![0u8; n]);
        let (mut l2, mut m2) = (vec![bf16::from_f32(0.0); n * d], vec![0u8; n]);
        spawn_batch_cpu(&mut l1, &templates, &priors, &mut m1, 1234, 0.1).unwrap();
        spawn_batch_cpu(&mut l2, &templates, &priors, &mut m2, 1234, 0.1).unwrap();
        assert_eq!(l1, l2);
    }

    #[test]
    fn cpu_different_seeds_differ() {
        let n = 32usize;
        let d = 8usize;
        let k = 1usize;
        let templates = vec![0u8; n];
        let priors = mk_priors(k, d);
        let (mut la, mut ma) = (vec![bf16::from_f32(0.0); n * d], vec![0u8; n]);
        let (mut lb, mut mb) = (vec![bf16::from_f32(0.0); n * d], vec![0u8; n]);
        spawn_batch_cpu(&mut la, &templates, &priors, &mut ma, 1, 0.5).unwrap();
        spawn_batch_cpu(&mut lb, &templates, &priors, &mut mb, 2, 0.5).unwrap();
        assert_ne!(la, lb);
    }

    #[test]
    fn cpu_rejects_invalid_template_index() {
        let templates = vec![0u8, 1, 9, 0];
        let priors = mk_priors(2, 2);
        let mut latent = vec![bf16::from_f32(0.0); 4 * 2];
        let mut mask = vec![0u8; 4];
        let err = spawn_batch_cpu(&mut latent, &templates, &priors, &mut mask, 0, 0.0).unwrap_err();
        assert!(matches!(err, EfeError::InvalidConfig(_)));
    }

    #[test]
    fn cpu_zero_n_is_noop() {
        let templates: Vec<u8> = vec![];
        let priors = mk_priors(2, 4);
        let mut latent: Vec<bf16> = vec![];
        let mut mask: Vec<u8> = vec![];
        spawn_batch_cpu(&mut latent, &templates, &priors, &mut mask, 0, 0.0).unwrap();
    }

    #[test]
    fn cpu_rejects_mask_length_mismatch() {
        let templates = vec![0u8, 0];
        let priors = mk_priors(1, 2);
        let mut latent = vec![bf16::from_f32(0.0); 4];
        let mut mask = vec![0u8; 3];
        let err = spawn_batch_cpu(&mut latent, &templates, &priors, &mut mask, 0, 0.0).unwrap_err();
        assert!(matches!(err, EfeError::InvalidConfig(_)));
    }
}
