//! Template registry consumed by M3.
//!
//! The registry is logically owned by M8 (templates module). M3 only
//! consumes it via the [`TemplateRegistry`] trait for reading prior
//! embeddings.
//!
//! # Sprint D8.5 — AnchorAware extension
//!
//! [`AnchorAwareTemplateRegistry`] is an **opt-in wrapper** that
//! modulates the `NaSem` template prior with the catalog anchors of
//! the active request. When the wrapped slot has been populated by
//! the surrounding backend (e.g.
//! `efe_server::mode1::Mode1Backend::execute`), the per-request
//! `NaSem` prior becomes a noisy blend of the anchors' centroid
//! and the base `NaSem` prior. This addresses the H3c-γ residual
//! identified in the D7 calibration audit (templates aveugles aux
//! anchors).
//!
//! **The wrapper is backward-compatible** : it implements the same
//! [`TemplateRegistry`] trait, so existing callers (genesis spawner,
//! tests) work without modification. Activation is opt-in via the
//! Mode-1 server's `Mode1Config::with_anchor_aware_templates()`.

use std::sync::{Arc, Mutex};

use crate::errors::{EfeError, Result};
use crate::types::TemplateCode;

/// Registry of nano-agent templates.
///
/// Implementations must be cheap to call (< 1 µs) — the registry is
/// consulted once per spawn.
pub trait TemplateRegistry: Send + Sync {
    /// Prior embedding for the given template.
    ///
    /// The returned slice has length equal to [`Self::dim_latent`].
    ///
    /// # Errors
    ///
    /// - [`EfeError::TemplateMissing`] if the template is not in the
    ///   registry.
    fn prior_embedding(&self, code: TemplateCode) -> Result<Arc<[f32]>>;

    /// Latent dimension the registry was built for.
    fn dim_latent(&self) -> u32;

    /// All templates this registry can serve, in order.
    fn available_templates(&self) -> &[TemplateCode];

    /// Whether `code` is present in this registry.
    fn contains(&self, code: TemplateCode) -> bool {
        self.available_templates().iter().any(|c| *c == code)
    }
}

/// Minimal registry useful for tests.
///
/// Each prior is a deterministic function of the template code and the
/// requested dim — no disk I/O, no allocations cached across calls.
pub struct StubTemplateRegistry {
    dim: u32,
    templates: Vec<TemplateCode>,
}

impl StubTemplateRegistry {
    /// Build a registry exposing the given templates at `dim`.
    #[must_use]
    pub fn new(dim: u32, templates: Vec<TemplateCode>) -> Self {
        Self { dim, templates }
    }

    /// Default MVP registry: all three MVP templates at the given dim.
    #[must_use]
    pub fn mvp_defaults(dim: u32) -> Self {
        Self {
            dim,
            templates: TemplateCode::MVP_SET.to_vec(),
        }
    }

    fn prior_for(code: TemplateCode, dim: u32) -> Vec<f32> {
        // Deterministic, class-specific pattern. Not meaningful — just
        // distinguishable across templates so downstream tests can
        // verify the right prior was used.
        let scale = (f32::from(code.as_u8()) + 1.0) * 0.01;
        (0..dim).map(|i| scale * (i as f32).sin()).collect()
    }
}

impl TemplateRegistry for StubTemplateRegistry {
    fn prior_embedding(&self, code: TemplateCode) -> Result<Arc<[f32]>> {
        if !self.templates.contains(&code) {
            return Err(EfeError::TemplateMissing(code.as_str()));
        }
        Ok(Arc::from(Self::prior_for(code, self.dim)))
    }

    fn dim_latent(&self) -> u32 {
        self.dim
    }

    fn available_templates(&self) -> &[TemplateCode] {
        &self.templates
    }
}

// =====================================================================
// Sprint D8.5 — AnchorAwareTemplateRegistry
// =====================================================================

/// Per-request snapshot of the anchors that should bias the NaSem
/// template prior. Populated by the surrounding backend (e.g.
/// `Mode1Backend::execute`) before delegation, consumed by
/// [`AnchorAwareTemplateRegistry::prior_embedding`] inside the
/// genesis spawn phase.
///
/// # Concurrency note
///
/// Multiple concurrent requests on the same backend instance share
/// the same slot — the last `set` wins. This is the same limitation
/// as `LastCentroid` / `LastSpecTrace` in the Mode-1 backend (cf.
/// `efe_server::mode1` module docstring). Mode 1 currently has
/// **single-flight semantics** in practice ; D8.5 inherits that
/// constraint and documents it as a follow-up for Phase 2 multi-tenant
/// hardening.
#[derive(Clone, Default, Debug)]
pub struct ActiveAnchorsSlot(Arc<Mutex<Option<Arc<[Vec<f32>]>>>>);

impl ActiveAnchorsSlot {
    /// Construct an empty slot.
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(None)))
    }

    /// Set the active anchors for the current request. The genesis
    /// spawner will read them when generating the NaSem prior.
    pub fn set(&self, anchors: Vec<Vec<f32>>) {
        let mut g = self.0.lock().expect("ActiveAnchorsSlot: mutex poisoned");
        *g = Some(Arc::from(anchors));
    }

    /// Clear the slot (defensive, used between requests).
    pub fn clear(&self) {
        let mut g = self.0.lock().expect("ActiveAnchorsSlot: mutex poisoned");
        *g = None;
    }

    /// Snapshot the current anchors without consuming them. Returns
    /// `None` if no anchors have been set since the last `clear`.
    pub fn peek(&self) -> Option<Arc<[Vec<f32>]>> {
        let g = self.0.lock().expect("ActiveAnchorsSlot: mutex poisoned");
        g.clone()
    }
}

/// **Sprint D8.5** — Wrapper around a base [`TemplateRegistry`] that
/// modulates the `NaSem` prior with the active request's anchors.
///
/// The blend formula for `NaSem` priors :
///
/// ```text
///     prior_anchor_aware[i] =
///         (1 - blend_strength) * base_prior[i]
///         + blend_strength * anchor_centroid[i]
///         + N(0, noise_sigma)
/// ```
///
/// where `anchor_centroid` is the element-wise mean of the active
/// anchors. `NaLog` and `NaCrit` priors are passed through unchanged.
///
/// When no anchors are active in the slot (e.g. backend hasn't
/// populated yet, or this registry is used outside of a request
/// context), the wrapper returns the base prior verbatim — preserving
/// backward compat for tests and benches that don't go through a
/// `Mode1Backend`.
pub struct AnchorAwareTemplateRegistry {
    base: Arc<dyn TemplateRegistry>,
    slot: ActiveAnchorsSlot,
    blend_strength: f32,
    noise_sigma: f32,
}

impl AnchorAwareTemplateRegistry {
    /// Create the wrapper. Defaults : `blend_strength = 0.6`,
    /// `noise_sigma = 0.05`.
    #[must_use]
    pub fn new(base: Arc<dyn TemplateRegistry>) -> Self {
        Self::with_params(base, ActiveAnchorsSlot::new(), 0.6, 0.05)
    }

    /// Create the wrapper with explicit parameters.
    ///
    /// `blend_strength` ∈ [0, 1] : 0 = base prior unchanged, 1 = pure
    /// anchor centroid. `noise_sigma` ≥ 0 : Gaussian noise std-dev.
    #[must_use]
    pub fn with_params(
        base: Arc<dyn TemplateRegistry>,
        slot: ActiveAnchorsSlot,
        blend_strength: f32,
        noise_sigma: f32,
    ) -> Self {
        Self {
            base,
            slot,
            blend_strength: blend_strength.clamp(0.0, 1.0),
            noise_sigma: noise_sigma.max(0.0),
        }
    }

    /// Expose the slot so the surrounding backend can `set` / `clear`
    /// per request.
    #[must_use]
    pub fn slot(&self) -> ActiveAnchorsSlot {
        self.slot.clone()
    }

    /// Compute the element-wise mean of the active anchors. Returns
    /// `None` if no anchors are set or anchors are length-0.
    fn anchor_centroid(&self) -> Option<Vec<f32>> {
        let anchors = self.slot.peek()?;
        if anchors.is_empty() {
            return None;
        }
        let dim = anchors[0].len();
        if dim == 0 {
            return None;
        }
        let mut acc = vec![0.0_f32; dim];
        let mut n = 0;
        for a in anchors.iter() {
            if a.len() != dim {
                // Mismatched anchors → defensive : skip rather than
                // mix garbage into the prior.
                continue;
            }
            for (i, &x) in a.iter().enumerate() {
                acc[i] += x;
            }
            n += 1;
        }
        if n == 0 {
            return None;
        }
        for x in &mut acc {
            *x /= n as f32;
        }
        Some(acc)
    }

    /// Deterministic pseudo-Gaussian noise (Box-Muller-style) seeded
    /// solely from the **dimension index** `idx`. We avoid `rand` in
    /// `efe-core` to keep dependencies minimal and the test
    /// reproducible.
    ///
    /// **Important** : the seed depends only on `idx`, NOT on the
    /// request or the anchors. The intent of this term is purely
    /// to **break exact equality** between the blended prior and
    /// `(1-bs)*base + bs*centroid` — i.e. ensure the prior is never
    /// a deterministic linear combination of two known vectors. It
    /// does NOT decorrelate spawned agents (the spawner uses the same
    /// prior for all agents in a class) and does NOT add per-request
    /// variance (the same `(idx, sigma)` always returns the same
    /// noise). Both of those are out of scope for D8.5 and would
    /// require a different mechanism if needed in the future.
    fn pseudo_noise(idx: usize, sigma: f32) -> f32 {
        // LCG constants (Knuth's MMIX). Declared at module scope at
        // the top of the impl to satisfy clippy::items_after_statements.
        const LCG_MUL: u64 = 2_862_933_555_777_941_757;
        const LCG_INC: u64 = 3_037_000_493;
        const MASK_24: u64 = 0xFF_FFFF;
        const SCALE_24: f32 = 0x100_0000 as f32;

        if sigma <= 0.0 {
            return 0.0;
        }
        // LCG seeded by idx → uniform u1, u2 in (0, 1].
        let mut state: u64 = (idx as u64).wrapping_mul(LCG_MUL).wrapping_add(LCG_INC);
        state ^= state >> 33;
        let u1 = ((state & MASK_24) as f32 + 1.0) / SCALE_24;
        state = state.wrapping_mul(LCG_MUL).wrapping_add(LCG_INC);
        state ^= state >> 33;
        let u2 = ((state & MASK_24) as f32 + 1.0) / SCALE_24;
        // Box-Muller → standard Normal sample
        let z = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f32::consts::PI * u2).cos();
        sigma * z
    }
}

impl TemplateRegistry for AnchorAwareTemplateRegistry {
    fn prior_embedding(&self, code: TemplateCode) -> Result<Arc<[f32]>> {
        let base_prior = self.base.prior_embedding(code)?;

        // Only NaSem is anchor-aware. NaLog and NaCrit pass through.
        if code != TemplateCode::NaSem {
            return Ok(base_prior);
        }
        let Some(centroid) = self.anchor_centroid() else {
            return Ok(base_prior); // no anchors → base prior
        };
        if centroid.len() != base_prior.len() {
            // Defensive : dimension mismatch → fall back to base.
            return Ok(base_prior);
        }
        let dim = base_prior.len();
        let bs = self.blend_strength;
        let mut blended: Vec<f32> = Vec::with_capacity(dim);
        for i in 0..dim {
            let v = (1.0 - bs) * base_prior[i]
                + bs * centroid[i]
                + Self::pseudo_noise(i, self.noise_sigma);
            blended.push(v);
        }
        Ok(Arc::from(blended))
    }

    fn dim_latent(&self) -> u32 {
        self.base.dim_latent()
    }

    fn available_templates(&self) -> &[TemplateCode] {
        self.base.available_templates()
    }
}

impl std::fmt::Debug for AnchorAwareTemplateRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // We deliberately omit `base` (Arc<dyn Trait>, no Debug bound)
        // and `slot` (interior-mutable Arc) — render their summary form
        // and finish_non_exhaustive() so clippy's
        // `missing_fields_in_debug` lint is satisfied.
        f.debug_struct("AnchorAwareTemplateRegistry")
            .field("blend_strength", &self.blend_strength)
            .field("noise_sigma", &self.noise_sigma)
            .field("dim_latent", &self.base.dim_latent())
            .field("base", &"<TemplateRegistry>")
            .field("slot", &"<ActiveAnchorsSlot>")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mvp_defaults_contain_all_mvp_templates() {
        let reg = StubTemplateRegistry::mvp_defaults(128);
        for t in TemplateCode::MVP_SET {
            assert!(reg.contains(*t));
        }
    }

    #[test]
    fn prior_shape_matches_dim() {
        let reg = StubTemplateRegistry::mvp_defaults(256);
        let prior = reg.prior_embedding(TemplateCode::NaSem).unwrap();
        assert_eq!(prior.len(), 256);
    }

    #[test]
    fn missing_template_errors() {
        let reg = StubTemplateRegistry::new(128, vec![TemplateCode::NaSem]);
        let err = reg.prior_embedding(TemplateCode::NaCrit).unwrap_err();
        assert!(matches!(err, EfeError::TemplateMissing(_)));
    }

    #[test]
    fn prior_is_deterministic() {
        let reg = StubTemplateRegistry::mvp_defaults(64);
        let a = reg.prior_embedding(TemplateCode::NaLog).unwrap();
        let b = reg.prior_embedding(TemplateCode::NaLog).unwrap();
        assert_eq!(a.as_ref(), b.as_ref());
    }

    #[test]
    fn different_templates_have_different_priors() {
        let reg = StubTemplateRegistry::mvp_defaults(16);
        let a = reg.prior_embedding(TemplateCode::NaSem).unwrap();
        let b = reg.prior_embedding(TemplateCode::NaLog).unwrap();
        assert_ne!(a.as_ref(), b.as_ref());
    }

    // ============================================================
    // Sprint D8.5 — AnchorAwareTemplateRegistry tests
    // ============================================================

    #[test]
    fn d85_anchor_aware_no_anchors_returns_base_prior() {
        let base: Arc<dyn TemplateRegistry> =
            Arc::new(StubTemplateRegistry::mvp_defaults(32));
        let aaw = AnchorAwareTemplateRegistry::new(base.clone());
        let base_prior = base.prior_embedding(TemplateCode::NaSem).unwrap();
        let aaw_prior = aaw.prior_embedding(TemplateCode::NaSem).unwrap();
        assert_eq!(
            aaw_prior.as_ref(),
            base_prior.as_ref(),
            "with no active anchors, AnchorAware must return the base prior unchanged"
        );
    }

    #[test]
    fn d85_anchor_aware_nalog_nacrit_pass_through_even_with_anchors() {
        let base: Arc<dyn TemplateRegistry> =
            Arc::new(StubTemplateRegistry::mvp_defaults(16));
        let aaw = AnchorAwareTemplateRegistry::new(base.clone());
        // Set anchors that would change NaSem.
        aaw.slot().set(vec![vec![1.0_f32; 16], vec![-1.0_f32; 16]]);
        // NaLog and NaCrit must NOT be modulated.
        let aaw_log = aaw.prior_embedding(TemplateCode::NaLog).unwrap();
        let base_log = base.prior_embedding(TemplateCode::NaLog).unwrap();
        assert_eq!(aaw_log.as_ref(), base_log.as_ref());
        let aaw_crit = aaw.prior_embedding(TemplateCode::NaCrit).unwrap();
        let base_crit = base.prior_embedding(TemplateCode::NaCrit).unwrap();
        assert_eq!(aaw_crit.as_ref(), base_crit.as_ref());
    }

    #[test]
    fn d85_anchor_aware_nasem_blends_when_anchors_set() {
        let base: Arc<dyn TemplateRegistry> =
            Arc::new(StubTemplateRegistry::mvp_defaults(8));
        let aaw = AnchorAwareTemplateRegistry::with_params(
            base.clone(),
            ActiveAnchorsSlot::new(),
            /* blend_strength */ 0.5,
            /* noise_sigma */ 0.0,  // no noise → deterministic test
        );
        let anchor = vec![1.0_f32; 8];
        aaw.slot().set(vec![anchor.clone()]);
        let base_prior = base.prior_embedding(TemplateCode::NaSem).unwrap();
        let blended = aaw.prior_embedding(TemplateCode::NaSem).unwrap();
        // With blend=0.5 and noise=0, expect (0.5*base + 0.5*anchor) per dim.
        for i in 0..8 {
            let expected = 0.5 * base_prior[i] + 0.5 * 1.0;
            assert!(
                (blended[i] - expected).abs() < 1e-6,
                "dim {i}: expected {expected}, got {}",
                blended[i]
            );
        }
    }

    #[test]
    fn d85_anchor_aware_centroid_averages_multiple_anchors() {
        let base: Arc<dyn TemplateRegistry> =
            Arc::new(StubTemplateRegistry::mvp_defaults(4));
        let aaw = AnchorAwareTemplateRegistry::with_params(
            base.clone(),
            ActiveAnchorsSlot::new(),
            1.0,  // pure anchor centroid
            0.0,
        );
        // 3 anchors averaging to (2.0, 0.0, 0.0, 0.0)
        aaw.slot().set(vec![
            vec![3.0, 0.0, 0.0, 0.0],
            vec![3.0, 0.0, 0.0, 0.0],
            vec![0.0, 0.0, 0.0, 0.0],
        ]);
        let blended = aaw.prior_embedding(TemplateCode::NaSem).unwrap();
        // Expected centroid = (2.0, 0.0, 0.0, 0.0).
        assert!((blended[0] - 2.0).abs() < 1e-6);
        for v in &blended[1..] {
            assert!(v.abs() < 1e-6);
        }
    }

    #[test]
    fn d85_anchor_aware_handles_dim_mismatch_gracefully() {
        let base: Arc<dyn TemplateRegistry> =
            Arc::new(StubTemplateRegistry::mvp_defaults(16));
        let aaw = AnchorAwareTemplateRegistry::new(base.clone());
        // Anchor of wrong dimension.
        aaw.slot().set(vec![vec![1.0_f32; 8]]);
        // Should fall back to base prior, not panic.
        let blended = aaw.prior_embedding(TemplateCode::NaSem).unwrap();
        let base_prior = base.prior_embedding(TemplateCode::NaSem).unwrap();
        assert_eq!(blended.as_ref(), base_prior.as_ref());
    }

    #[test]
    fn d85_active_anchors_slot_set_clear_peek() {
        let slot = ActiveAnchorsSlot::new();
        assert!(slot.peek().is_none());
        slot.set(vec![vec![0.5_f32; 4]]);
        let peeked = slot.peek().expect("slot has anchors");
        assert_eq!(peeked.len(), 1);
        assert_eq!(peeked[0].len(), 4);
        // peek does not consume.
        assert!(slot.peek().is_some());
        slot.clear();
        assert!(slot.peek().is_none());
    }

    #[test]
    fn d85_anchor_aware_dim_and_templates_delegate_to_base() {
        let base: Arc<dyn TemplateRegistry> =
            Arc::new(StubTemplateRegistry::mvp_defaults(64));
        let aaw = AnchorAwareTemplateRegistry::new(base.clone());
        assert_eq!(aaw.dim_latent(), base.dim_latent());
        assert_eq!(aaw.available_templates(), base.available_templates());
    }

    #[test]
    fn d85_anchor_aware_blend_strength_clamped() {
        let base: Arc<dyn TemplateRegistry> =
            Arc::new(StubTemplateRegistry::mvp_defaults(4));
        // blend_strength out of range — should clamp to [0, 1].
        let aaw = AnchorAwareTemplateRegistry::with_params(
            base, ActiveAnchorsSlot::new(), 5.0, -1.0,
        );
        // blend = 1.0 (clamped), noise_sigma = 0.0 (clamped from -1).
        // Validity check : the structure is built without panic.
        let _ = aaw.prior_embedding(TemplateCode::NaSem).unwrap();
    }

    #[test]
    fn d85_pseudo_noise_zero_sigma_returns_zero() {
        // The noise function must return exactly 0.0 when sigma = 0.
        // This makes deterministic tests possible. We compare via
        // bit equality to avoid the clippy `float_cmp` lint while
        // still asserting "exactly zero".
        for i in 0..100 {
            let n = AnchorAwareTemplateRegistry::pseudo_noise(i, 0.0);
            assert!(n == 0.0, "expected exact zero at idx {i}, got {n}");
        }
    }

    #[test]
    fn d85_pseudo_noise_finite_with_positive_sigma() {
        for i in 0..100 {
            let n = AnchorAwareTemplateRegistry::pseudo_noise(i, 0.05);
            assert!(n.is_finite(), "noise at idx {i} must be finite, got {n}");
        }
    }
}
