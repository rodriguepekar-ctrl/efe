//! **Couche 3 · M1.c — the empirical Lipschitz canary.**
//!
//! Measures a lower bound `L̂` on the Lipschitz constant `L_Φ` of a participant's
//! internal operator `Φ` on its operational domain, by *adversarial* black-box
//! sampling. This is the live form of hypothesis **H1** (`L_Φ ≤ 1`, on which all
//! Krasnosel'skiĭ–Mann convergence of the meta-swarm rests): a real
//! [`crate::participants::NonExpansiveCertificate::Empirical`] replaces the dead
//! `Stub` that always passed.
//!
//! ## Honesty — this is a FALSIFIER, not a proof (never over-sell)
//!
//! `L̂ = max over sampled pairs of ‖Φ(x)−Φ(y)‖ / ‖x−y‖` is a **lower bound** on the
//! true `L_Φ`. It can only **falsify** non-expansiveness (`L̂ ≥ 1 ⇒ Φ is expansive,
//! H1 fails, fail-closed`); it can **never certify** it (`L̂ < 1` means "no expansion
//! found by this search", not "Φ is non-expansive"). Label everywhere:
//! **"non-expansiveness property-tested, NOT certified."** A closed-form guarantee
//! needs the fixed-topology construction (architecture doc, Palier-2) — a separate,
//! stronger milestone; this canary is delivered to *catch drift*, fail-closed.
//!
//! ## Why the sampling must be adversarial
//!
//! For this efe-core wiring, `Φ` is (architecture doc §1):
//!   * **intra-cell** analytically a `~β₁ ≈ 0.17` CONTRACTION — `x` enters only through
//!     `beta_mix` at weight `β₁`; everything else (attention, anchor projection, clip,
//!     centroid mean) is non-expansive by composition;
//!   * **across genesis-routing cells** DISCONTINUOUS — the meta-analyzer maps `x` to an
//!     *integer* agent count → a step-function topology → `Φ` jumps at cell boundaries,
//!     where the local Lipschitz constant is unbounded.
//!
//! So **uniform** pair sampling is optimistically biased: two uniform points almost
//! surely land in the *same* cell (where `L ≈ 0.17`) and essentially never straddle a
//! boundary (a measure-zero set) → `L̂ ≪ 1` while `Φ` is discontinuous. The canary
//! therefore actively **hunts discontinuities by bisection**: given a pair whose `Φ`
//! images are far apart, it repeatedly halves the interval while keeping the jump
//! enclosed; if a true discontinuity is trapped, `‖x−y‖ → 0` while `‖Φ(x)−Φ(y)‖` stays
//! bounded away from 0, so the ratio explodes and the expansion is exposed.
//!
//! ## Determinism
//!
//! The whole search is a deterministic function of `(dim, domain, probe, Φ)` — points
//! come from [`crate::Tensor::pseudo_random`] seeded by `probe.seed ⊕ index`, no
//! `rand`, no clock. Combined with M1.b's deterministic `Φ`, the resulting `L̂` (and
//! thus the certificate) is bit-reproducible. This is why M1.b gates M1.c.

use crate::Tensor;

/// Tuning knobs for the [`estimate_lipschitz`] canary.
///
/// All fields are public so callers can shrink the budget for fast tests
/// ([`LipschitzProbe::small`]) or widen it for a thorough pre-flight check
/// ([`LipschitzProbe::default`]).
#[derive(Debug, Clone)]
pub struct LipschitzProbe {
    /// Number of deterministic random seed points drawn in the domain. Points where
    /// `Φ` is out-of-domain (returns `None`) are discarded; the rest form the base set.
    pub n_seeds: usize,
    /// Number of random in-domain pairs scored for the baseline (intra-cell) ratio.
    /// Pairs reuse cached `Φ` evaluations of the seeds — they cost **no** extra `Φ` calls.
    pub n_pairs: usize,
    /// Finite-difference probe directions per seed (local operator-norm estimate).
    pub n_directions: usize,
    /// Number of widest-`Φ`-gap pairs refined by discontinuity bisection.
    pub n_bisect_pairs: usize,
    /// Maximum bisection depth per refined pair. Each level halves `‖x−y‖`, so a trapped
    /// discontinuity multiplies the ratio by ~2 per level: 30 ⇒ ratio ×10⁹.
    pub bisect_depth: usize,
    /// Finite-difference step (absolute, in latent units) for the directional probes.
    pub fd_eps: f64,
    /// Stop bisecting once `‖x−y‖` falls below this. It is the **f32 noise floor**: at
    /// separation `s` a continuous `Φ` has `‖Φ(x)−Φ(y)‖ ≈ L·s`, but its f32 rounding
    /// error `~ε·‖Φ‖` is separation-independent, so the *measured* ratio picks up a
    /// spurious `~ε·‖Φ‖ / s` term. Below ~`1e-3` that term would fabricate expansion out
    /// of pure rounding (false positive). It also sets the smallest jump the canary can
    /// resolve: a discontinuity of size `J` is exposed (`ratio ≥ 1`) once `s ≤ J`, so
    /// jumps `≥ min_sep` are caught; smaller ones are below the noise floor (honest lower
    /// bound).
    pub min_sep: f64,
    /// Acceptance margin: the empirical certificate is valid iff `L̂ < 1 − margin`.
    pub margin: f64,
    /// Hard cap on `Φ` evaluations. If the planned probes exceed it, the canary stops
    /// early and marks the report `budget_exhausted` (⇒ fail-closed: an incomplete search
    /// must not certify).
    pub max_evals: usize,
    /// RNG seed — makes the whole search reproducible.
    pub seed: u64,
}

impl Default for LipschitzProbe {
    fn default() -> Self {
        Self {
            n_seeds:        96,
            n_pairs:        256,
            n_directions:   6,
            n_bisect_pairs: 16,
            bisect_depth:   30,
            fd_eps:         1e-3,
            min_sep:        1e-3,
            margin:         0.05,
            max_evals:      50_000,
            seed:           0x00C0_FFEE_CAFE_1C3C,
        }
    }
}

impl LipschitzProbe {
    /// A cheap preset for tests / hot pre-flight checks over a *real* (slow) `Φ`:
    /// a few dozen seeds and short bisection, ~a few hundred `Φ` evaluations.
    #[must_use]
    pub fn small() -> Self {
        Self {
            n_seeds:        24,
            n_pairs:        64,
            n_directions:   4,
            n_bisect_pairs: 6,
            bisect_depth:   20,
            fd_eps:         1e-3,
            min_sep:        1e-3,
            margin:         0.05,
            max_evals:      5_000,
            seed:           0x00C0_FFEE_CAFE_1C3C,
        }
    }
}

/// Outcome of the [`estimate_lipschitz`] canary.
#[derive(Debug, Clone)]
pub struct LipschitzReport {
    /// The measured lower bound `L̂` on `L_Φ` (the max ratio over all probes).
    /// `0.0` if no valid pair could be scored (see `insufficient_samples`).
    pub l_hat: f64,
    /// `‖x−y‖` of the worst (max-ratio) pair — tiny worst_sep + large `l_hat`
    /// signals a discontinuity; moderate worst_sep signals smooth expansion.
    pub worst_sep: f64,
    /// Number of `Φ` evaluations performed.
    pub n_evals: usize,
    /// Number of pairs actually scored (both endpoints in-domain, distinct).
    pub n_pairs_scored: usize,
    /// Number of sampled points where `Φ` was out-of-domain (returned `None`).
    pub n_domain_errors: usize,
    /// Fewer than two in-domain seeds were found — the domain is (near) empty or `Φ`
    /// is unusable. A conclusive `L̂` could not be measured ⇒ fail-closed.
    pub insufficient_samples: bool,
    /// The `Φ`-evaluation budget was hit before the planned probes finished ⇒ the search
    /// is incomplete and must not certify (fail-closed).
    pub budget_exhausted: bool,
    /// `Φ` **failed to evaluate** ([`PhiOutcome::Failed`]) at ≥1 probe point — a numerical
    /// blow-up / divergence / timeout, i.e. a pathology of the operator itself (not a
    /// domain boundary). That is an H1-relevant signal ⇒ **fail-closed**: a `Φ` that
    /// blows up on part of its own domain is never certified non-expansive.
    pub saw_failure: bool,
}

/// The outcome of evaluating `Φ` at a probe point, as reported by the oracle to
/// [`estimate_lipschitz`]. The three-way distinction is a **soundness** requirement: a
/// numerical blow-up must NOT be silently confused with a legitimate domain boundary
/// (else the falsifier could accept a `Φ` that diverges on part of its domain).
#[derive(Debug, Clone)]
pub enum PhiOutcome {
    /// `Φ(x)` is defined and returned a value (still checked for finiteness + dim).
    InDomain(Vec<f32>),
    /// `x` is legitimately **outside** `Φ`'s domain (e.g. an alignment-cone breach): the
    /// operator is simply not defined here. The point is skipped (counted in
    /// `n_domain_errors`), which is sound because the meta-swarm never steps there either.
    OutOfDomain,
    /// `Φ` **failed** to evaluate at `x` (numerical explosion, divergence, timeout, …) —
    /// a pathology of the operator, NOT a domain boundary. The canary records it and
    /// **fails-closed** (`saw_failure`).
    Failed,
}

impl LipschitzReport {
    /// Is the measured bound acceptable for H1 at the given `margin`?
    ///
    /// Fail-closed: `false` if the search was inconclusive (`insufficient_samples`) or
    /// truncated (`budget_exhausted`) or produced a non-finite bound; otherwise
    /// `L̂ < 1 − margin`.
    #[must_use]
    pub fn accepts(&self, margin: f64) -> bool {
        !self.insufficient_samples
            && !self.budget_exhausted
            && !self.saw_failure
            && self.l_hat.is_finite()
            && self.l_hat < 1.0 - margin.max(0.0)
    }
}

/// One coordinate of a deterministic uniform sample in `[lo, hi]`, from a `u ∈ [-1, 1)`.
#[inline]
fn map_unit(u: f32, lo: f32, hi: f32) -> f32 {
    // (u+1)/2 ∈ [0, 1) → lo + t·(hi-lo)
    let t = 0.5 * (u + 1.0);
    lo + t * (hi - lo)
}

/// Draw the `k`-th deterministic uniform sample in the box `[lo, hi]`.
fn sample_in_box(dim: usize, lo: &[f32], hi: &[f32], seed: u64, k: u64) -> Tensor {
    // A distinct sub-seed per sample; `pseudo_random` yields dim coords in [-1, 1).
    let sub = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15).wrapping_add(k.wrapping_mul(0x2545_F491_4F6C_DD1D));
    let u = Tensor::pseudo_random(dim, sub);
    let mut out = Vec::with_capacity(dim);
    for i in 0..dim {
        out.push(map_unit(u.at(i), lo[i], hi[i]));
    }
    Tensor::from_vec(out)
}

/// L2 distance between two raw latent slices (assumed equal length).
#[inline]
fn dist(a: &[f32], b: &[f32]) -> f64 {
    let mut s = 0.0_f64;
    for (x, y) in a.iter().zip(b) {
        let d = f64::from(*x) - f64::from(*y);
        s += d * d;
    }
    s.sqrt()
}

/// Midpoint of two tensors (equal dim).
fn midpoint(a: &Tensor, b: &Tensor) -> Tensor {
    let (sa, sb) = (a.as_slice(), b.as_slice());
    let mut out = Vec::with_capacity(sa.len());
    for (x, y) in sa.iter().zip(sb) {
        out.push(0.5 * (x + y));
    }
    Tensor::from_vec(out)
}

/// **The empirical Lipschitz canary.** Returns a lower bound `L̂` on `L_Φ` over the
/// box domain `[lo, hi]`, hunting discontinuities adversarially.
///
/// `phi(x)` returns a [`PhiOutcome`]: `InDomain(Φ(x))` when defined, `OutOfDomain` when
/// `x` is legitimately outside `Φ`'s domain (skipped, sound), or `Failed` when `Φ` blew
/// up (numerical explosion / divergence / timeout) — a pathology that fails the search
/// closed (it must NOT be confused with a domain boundary). All arithmetic is
/// deterministic given `(dim, lo, hi, probe, phi)`.
///
/// # Panics
///
/// Debug-asserts `lo.len() == hi.len() == dim`. In release, a mismatch yields an
/// `insufficient_samples` report rather than a panic.
pub fn estimate_lipschitz<F>(
    dim: usize,
    lo: &[f32],
    hi: &[f32],
    probe: &LipschitzProbe,
    mut phi: F,
) -> LipschitzReport
where
    F: FnMut(&[f32]) -> PhiOutcome,
{
    debug_assert_eq!(lo.len(), dim);
    debug_assert_eq!(hi.len(), dim);

    let mut report = LipschitzReport {
        l_hat: 0.0,
        worst_sep: 0.0,
        n_evals: 0,
        n_pairs_scored: 0,
        n_domain_errors: 0,
        insufficient_samples: false,
        budget_exhausted: false,
        saw_failure: false,
    };
    if dim == 0 || lo.len() != dim || hi.len() != dim {
        report.insufficient_samples = true;
        return report;
    }

    // A small closure-local budget guard: every `Φ` call goes through `eval`.
    // Returns None when the budget is hit (and flags it) OR when `Φ` is out-of-domain.
    let max_evals = probe.max_evals;

    // --- Phase A: seed points -------------------------------------------------
    // Cache (point, Φ(point)) for every in-domain seed.
    let mut base: Vec<(Tensor, Vec<f32>)> = Vec::with_capacity(probe.n_seeds);
    for k in 0..probe.n_seeds as u64 {
        if report.n_evals >= max_evals {
            report.budget_exhausted = true;
            break;
        }
        let x = sample_in_box(dim, lo, hi, probe.seed, k);
        report.n_evals += 1;
        match phi(x.as_slice()) {
            PhiOutcome::InDomain(fx) if fx.len() == dim && fx.iter().all(|v| v.is_finite()) => {
                base.push((x, fx));
            }
            // A finite-but-wrong-dim vector or an explicit Failed is an operator pathology
            // (not a domain boundary) ⇒ fail-closed.
            PhiOutcome::InDomain(_) | PhiOutcome::Failed => report.saw_failure = true,
            PhiOutcome::OutOfDomain => report.n_domain_errors += 1,
        }
    }
    if base.len() < 2 {
        report.insufficient_samples = true;
        return report;
    }

    // Helper to fold a candidate ratio into the running maximum.
    let mut record = |num: f64, den: f64, l_hat: &mut f64, worst_sep: &mut f64, scored: &mut usize| {
        if den > 0.0 && num.is_finite() {
            *scored += 1;
            let r = num / den;
            if r > *l_hat {
                *l_hat = r;
                *worst_sep = den;
            }
        }
    };

    // --- Phase B: random in-domain pairs (baseline, no extra Φ evals) ---------
    // Also remember the widest-Φ-gap pairs to seed the discontinuity hunt.
    let n = base.len();
    let mut widest: Vec<(usize, usize, f64)> = Vec::new(); // (i, j, ‖Φ(i)−Φ(j)‖)
    let n64 = n as u64;
    for k in 0..probe.n_pairs as u64 {
        let i = (k % n64) as usize;
        // A deterministic, i-dependent partner index, coprime stride to spread pairs. The
        // arithmetic is done in explicit u64 (like the other phases) so the scored-pair set
        // — and thus L̂ and the certificate — is width-invariant (bit-reproducible on 32-
        // and 64-bit targets alike).
        let j = (k.wrapping_mul(2_654_435_761).wrapping_add(i as u64 + 1) % n64) as usize;
        if i == j {
            continue;
        }
        let (xa, fa) = &base[i];
        let (xb, fb) = &base[j];
        let den = dist(xa.as_slice(), xb.as_slice());
        let num = dist(fa, fb);
        record(num, den, &mut report.l_hat, &mut report.worst_sep, &mut report.n_pairs_scored);
        widest.push((i, j, num));
    }

    // --- Phase C: finite-difference directional probes (local operator norm) --
    // Perturb each seed along deterministic unit directions; a large ‖ΔΦ‖/fd_eps
    // reveals smooth local expansion the random pairs may miss.
    let fd_eps = probe.fd_eps as f32;
    'seeds: for (si, (x, fx)) in base.iter().enumerate() {
        for d in 0..probe.n_directions as u64 {
            if report.n_evals >= max_evals {
                report.budget_exhausted = true;
                break 'seeds;
            }
            // A unit-ish direction (pseudo_random ∈ [-1,1)^dim), normalized.
            let dir = Tensor::pseudo_random(dim, probe.seed ^ (0xD1B5_4A32_D192_ED03u64.wrapping_add((si as u64) << 8).wrapping_add(d)));
            let dnorm = dir.norm();
            if dnorm <= 0.0 {
                continue;
            }
            let scale = fd_eps / (dnorm as f32);
            let mut y = Vec::with_capacity(dim);
            for i in 0..dim {
                y.push(x.at(i) + scale * dir.at(i));
            }
            report.n_evals += 1;
            match phi(&y) {
                PhiOutcome::InDomain(fy) if fy.len() == dim && fy.iter().all(|v| v.is_finite()) => {
                    let num = dist(fx, &fy);
                    let den = dist(x.as_slice(), &y);
                    record(num, den, &mut report.l_hat, &mut report.worst_sep, &mut report.n_pairs_scored);
                }
                PhiOutcome::InDomain(_) | PhiOutcome::Failed => report.saw_failure = true,
                PhiOutcome::OutOfDomain => {}
            }
        }
    }

    // --- Phase D: adversarial discontinuity bisection -------------------------
    // Refine the widest-Φ-gap pairs: halve ‖x−y‖ while keeping the jump enclosed.
    // A trapped discontinuity makes the ratio explode.
    widest.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    for &(i, j, _) in widest.iter().take(probe.n_bisect_pairs) {
        let mut a = base[i].0.clone();
        let mut fa = base[i].1.clone();
        let mut b = base[j].0.clone();
        let mut fb = base[j].1.clone();
        // Below this `‖Φ(a)−Φ(b)‖`, the two images are equal up to f32 rounding: the
        // interval is "continuous here", there is no jump left to trap, so stop (further
        // bisection would only amplify rounding noise into a spurious ratio).
        const FP_FLOOR: f64 = 1e-6;
        for _ in 0..probe.bisect_depth {
            let den = dist(a.as_slice(), b.as_slice());
            if den < probe.min_sep {
                break;
            }
            // Score the current (shrinking) interval.
            let num = dist(&fa, &fb);
            record(num, den, &mut report.l_hat, &mut report.worst_sep, &mut report.n_pairs_scored);
            if num < FP_FLOOR {
                break;
            }
            if report.n_evals >= max_evals {
                report.budget_exhausted = true;
                break;
            }
            let m = midpoint(&a, &b);
            report.n_evals += 1;
            let fm = match phi(m.as_slice()) {
                PhiOutcome::InDomain(v) if v.len() == dim && v.iter().all(|t| t.is_finite()) => v,
                // Φ blew up between a and b — a pathology ⇒ fail-closed.
                PhiOutcome::InDomain(_) | PhiOutcome::Failed => {
                    report.saw_failure = true;
                    break;
                }
                // A domain hole between a and b — cannot bisect reliably through it.
                PhiOutcome::OutOfDomain => break,
            };
            // Keep the half that still straddles the jump: move the endpoint whose Φ is
            // CLOSER to Φ(m) up to m (that half is "continuous"; the jump is on the other).
            if dist(&fm, &fa) <= dist(&fm, &fb) {
                a = m;
                fa = fm;
            } else {
                b = m;
                fb = fm;
            }
        }
    }

    if report.n_pairs_scored == 0 {
        report.insufficient_samples = true;
    }
    report
}

// =============================================================================
// Tests — synthetic Φ (instant; no efe-core / efe-server needed)
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_box(dim: usize, r: f32) -> (Vec<f32>, Vec<f32>) {
        (vec![-r; dim], vec![r; dim])
    }

    /// Identity Φ(x)=x has L_Φ=1 exactly. The canary must measure L̂≈1 (never above by
    /// more than fp noise) and REJECT at any positive margin (non-strict inequality).
    #[test]
    fn identity_measures_one_and_is_rejected() {
        let dim = 8;
        let (lo, hi) = unit_box(dim, 1.0);
        let probe = LipschitzProbe::default();
        let rep = estimate_lipschitz(dim, &lo, &hi, &probe, |x| PhiOutcome::InDomain(x.to_vec()));
        assert!(rep.l_hat > 0.99 && rep.l_hat < 1.0001, "identity L̂≈1, got {}", rep.l_hat);
        assert!(!rep.accepts(0.05), "identity (L=1) must fail H1 at margin 0.05");
        assert!(!rep.insufficient_samples && !rep.budget_exhausted);
    }

    /// A κ-contraction Φ(x)=κx has L_Φ=κ. The canary must measure L̂≈κ and ACCEPT.
    #[test]
    fn contraction_measures_kappa_and_is_accepted() {
        let dim = 8;
        let (lo, hi) = unit_box(dim, 1.0);
        let kappa = 0.17_f32; // the architecture's intra-cell β₁ figure
        let probe = LipschitzProbe::default();
        let rep = estimate_lipschitz(dim, &lo, &hi, &probe, |x| {
            PhiOutcome::InDomain(x.iter().map(|v| kappa * v).collect())
        });
        assert!(
            (rep.l_hat - f64::from(kappa)).abs() < 1e-3,
            "contraction L̂≈{kappa}, got {}",
            rep.l_hat
        );
        assert!(rep.accepts(0.05), "0.17-contraction must satisfy H1");
    }

    /// **The adversarial payoff.** A Φ that is a 0.17-contraction *within* each cell but
    /// JUMPS across a routing boundary (a step at x[0]=0) is discontinuous ⇒ locally
    /// non-Lipschitz. Uniform pairs would read ≈0.17; the bisection MUST expose the jump
    /// (L̂ ≫ 1) and REJECT. This is the whole reason the sampling is adversarial.
    #[test]
    fn discontinuity_is_caught_by_bisection() {
        let dim = 4;
        let (lo, hi) = unit_box(dim, 1.0);
        let probe = LipschitzProbe::default();
        let rep = estimate_lipschitz(dim, &lo, &hi, &probe, |x| {
            // Intra-cell: 0.17-contraction. Cross-cell: a constant offset added when x[0]≥0.
            let jump = if x[0] >= 0.0 { 5.0 } else { 0.0 };
            PhiOutcome::InDomain(
                x.iter().enumerate().map(|(i, v)| 0.17 * v + if i == 0 { jump } else { 0.0 }).collect(),
            )
        });
        assert!(rep.l_hat > 1.0, "discontinuity must push L̂ above 1, got {}", rep.l_hat);
        assert!(!rep.accepts(0.05), "a discontinuous Φ must fail H1 (fail-closed)");
        assert!(rep.worst_sep < 1e-2, "the worst pair must be a near-boundary jump, sep={}", rep.worst_sep);
    }

    /// Determinism: same (dim, domain, probe, Φ) ⇒ bit-identical L̂ across runs.
    #[test]
    fn estimate_is_deterministic() {
        let dim = 6;
        let (lo, hi) = unit_box(dim, 2.0);
        let probe = LipschitzProbe::default();
        let phi = |x: &[f32]| PhiOutcome::InDomain(x.iter().map(|v| 0.5 * v).collect::<Vec<_>>());
        let a = estimate_lipschitz(dim, &lo, &hi, &probe, phi);
        let b = estimate_lipschitz(dim, &lo, &hi, &probe, phi);
        assert_eq!(a.l_hat.to_bits(), b.l_hat.to_bits(), "L̂ must be bit-reproducible");
    }

    /// A Φ that is out-of-domain everywhere ⇒ insufficient_samples ⇒ fail-closed.
    #[test]
    fn empty_domain_fails_closed() {
        let dim = 4;
        let (lo, hi) = unit_box(dim, 1.0);
        let probe = LipschitzProbe::small();
        let rep = estimate_lipschitz(dim, &lo, &hi, &probe, |_x| PhiOutcome::OutOfDomain);
        assert!(rep.insufficient_samples);
        assert!(!rep.accepts(0.05), "no in-domain samples ⇒ cannot certify");
        assert_eq!(rep.n_domain_errors, probe.n_seeds);
    }

    /// A partially-restricted domain: Φ defined only where x[0] ≥ 0. The canary must
    /// still measure a bound on the in-domain region and skip the rest as domain errors.
    #[test]
    fn partial_domain_is_handled() {
        let dim = 4;
        let (lo, hi) = unit_box(dim, 1.0);
        let probe = LipschitzProbe::default();
        let rep = estimate_lipschitz(dim, &lo, &hi, &probe, |x| {
            if x[0] >= 0.0 {
                PhiOutcome::InDomain(x.iter().map(|v| 0.3 * v).collect())
            } else {
                PhiOutcome::OutOfDomain
            }
        });
        assert!(rep.n_domain_errors > 0, "half-space domain must reject some samples");
        assert!(!rep.insufficient_samples, "the in-domain half must yield enough samples");
        assert!(rep.accepts(0.05), "the in-domain part is a 0.3-contraction");
    }

    /// The budget cap forces `budget_exhausted` ⇒ fail-closed even for a benign Φ.
    #[test]
    fn budget_exhaustion_fails_closed() {
        let dim = 4;
        let (lo, hi) = unit_box(dim, 1.0);
        let mut probe = LipschitzProbe::default();
        probe.max_evals = 5; // far below the planned probe count
        let rep =
            estimate_lipschitz(dim, &lo, &hi, &probe, |x| PhiOutcome::InDomain(x.iter().map(|v| 0.1 * v).collect()));
        assert!(rep.budget_exhausted, "tiny budget must trip budget_exhausted");
        assert!(!rep.accepts(0.05), "an incomplete search must not certify (fail-closed)");
    }

    /// **Soundness regression (adversarial review #1).** A Φ that is a benign
    /// 0.3-contraction on most of the box but BLOWS UP (`Failed`) on a sub-region must
    /// NOT be certified: the blow-up is an operator pathology, not a domain boundary, so
    /// `saw_failure` must fail the search closed even though the measured pairs look tame.
    #[test]
    fn partial_blowup_fails_closed_not_confused_with_out_of_domain() {
        let dim = 4;
        let (lo, hi) = unit_box(dim, 1.0);
        let probe = LipschitzProbe::default();
        let rep = estimate_lipschitz(dim, &lo, &hi, &probe, |x| {
            if x[0] > 0.6 {
                PhiOutcome::Failed // a numerical blow-up on part of the domain
            } else {
                PhiOutcome::InDomain(x.iter().map(|v| 0.3 * v).collect())
            }
        });
        assert!(rep.saw_failure, "a Φ blow-up must set saw_failure");
        assert!(
            !rep.accepts(0.05),
            "a Φ that blows up on part of its domain must NEVER be certified (fail-closed), \
             even though the benign region measures L̂={}",
            rep.l_hat
        );
        // A blow-up must NOT be silently laundered into the out-of-domain count.
        assert_eq!(rep.n_domain_errors, 0, "Failed must not inflate n_domain_errors (that would mislabel a pathology)");
    }

    /// A negative margin must not lift the acceptance threshold above 1.0 and admit a
    /// measured expansion (`accepts` clamps `margin` at 0).
    #[test]
    fn negative_margin_cannot_admit_expansion() {
        let rep = LipschitzReport {
            l_hat: 1.5,
            worst_sep: 1e-3,
            n_evals: 10,
            n_pairs_scored: 10,
            n_domain_errors: 0,
            insufficient_samples: false,
            budget_exhausted: false,
            saw_failure: false,
        };
        assert!(!rep.accepts(-1.0), "a negative margin must not lift the threshold above 1.0");
        assert!(!rep.accepts(0.0), "L̂=1.5 is expansion; must fail at margin 0 too");
    }
}
