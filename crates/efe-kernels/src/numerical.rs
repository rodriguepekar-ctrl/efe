// SPDX-License-Identifier: UNLICENSED
//! Sprint **Φ-D D29 (Bundle completion)** — numerical equivalence
//! harness.
//!
//! Cf. `docs/EFE_CUDA_KERNELS_COMPLETION_ARCHITECTURE.md` § 8.
//!
//! ## Mission
//!
//! Garantir que tout kernel CUDA produit un résultat
//! **numériquement équivalent** à l'implémentation référence CPU
//! (`CpuBackend`), avec une tolérance documentée par opération.
//! Sans cela, le moteur swarm peut produire des verdicts différents
//! selon l'hôte = catastrophique pour P3 audit chain.
//!
//! Ce module **ne dépend PAS de CUDA** — il fournit l'API
//! `assert_close_*` + les tolérances canoniques. Les vrais tests
//! (= run avec un GPU runner) vivent dans
//! `tests/numerical_equivalence.rs` derrière `#[cfg(feature = "cuda")]`.
//!
//! ## Tolérances canoniques (= doc Φ-D § 8.2)
//!
//! | Kernel | rtol fp32 | rtol fp16 |
//! |--------|----------:|----------:|
//! | weighted_mean | 1e-6 | 1e-3 |
//! | cone_project | 1e-5 | 1e-3 |
//! | fused_softmax | 1e-5 | 1e-3 |
//! | l2_normalize | 1e-6 | 1e-3 |
//! | cosine | 1e-5 | 1e-3 |
//! | l2_distance | 1e-5 | 1e-3 |
//! | argmax | 0 exact | 0 exact |
//! | topk | 0 ordering | dep on ties |
//! | mean_pooling | 1e-6 | 1e-3 |
//! | kl_divergence | 1e-5 | 1e-3 |
//! | shannon_entropy | 1e-5 | 1e-3 |
//! | moe_reconcile | 1e-5 | 1e-3 |

use crate::tune::KernelId;

/// Precision-aware tolerance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tolerance {
    /// Relative tolerance : `|a-b| / |b|` ≤ rtol.
    pub rtol: f64,
    /// Absolute tolerance : `|a-b|` ≤ atol (fallback quand `b ≈ 0`).
    pub atol: f64,
}

impl Tolerance {
    /// Tolerance exacte (= argmax / topk indices, jamais f-comparison).
    pub const EXACT: Self = Self { rtol: 0.0, atol: 0.0 };
    /// Tolerance fp32 typique.
    pub const FP32_STRICT: Self = Self { rtol: 1e-6, atol: 1e-7 };
    /// Tolerance fp32 relaxée (= reductions order non-déterministe).
    pub const FP32_LAX: Self = Self { rtol: 1e-5, atol: 1e-6 };
    /// Tolerance fp16 typique.
    pub const FP16_DEFAULT: Self = Self { rtol: 1e-3, atol: 1e-4 };
    /// Tolerance fp16 relaxée (= exp/log fonctions).
    pub const FP16_LAX: Self = Self { rtol: 5e-3, atol: 1e-3 };
}

/// Precision du kernel testé.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Precision {
    /// IEEE float32 (4 bytes).
    Fp32,
    /// IEEE float16 (2 bytes).
    Fp16,
    /// brain-float16 (Ampere+).
    Bf16,
}

/// Lookup la tolérance canonique pour (kernel × precision).
///
/// Pure function. Pin par le test
/// `canonical_tolerance_per_kernel_table`.
#[must_use]
pub fn canonical_tolerance(kernel: KernelId, precision: Precision) -> Tolerance {
    use KernelId::*;
    use Precision::*;
    match (kernel, precision) {
        // Argmax / topk = exact int indices (= pas de f-comparaison)
        (Argmax, _) => Tolerance::EXACT,
        (TopK, Fp32) => Tolerance::EXACT,
        (TopK, _) => Tolerance::FP16_DEFAULT, // ties handling tolerated

        // Reductions sur fp32 lax (= ordre summation diffère)
        (WeightedMean | MeanPooling | L2Normalize, Fp32) => Tolerance::FP32_STRICT,
        (WeightedMean | MeanPooling | L2Normalize, Fp16 | Bf16) => Tolerance::FP16_DEFAULT,

        // Cosine / distance / softmax / cone : fp32 lax + fp16 lax
        (CosineSimilarity | L2Distance | ConeProject | FusedSoftmaxEntropy, Fp32) => {
            Tolerance::FP32_LAX
        }
        (CosineSimilarity | L2Distance | ConeProject | FusedSoftmaxEntropy, Fp16 | Bf16) => {
            Tolerance::FP16_DEFAULT
        }

        // KL / entropy : utilisent log() = plus relaxé
        (KlDivergence | ShannonEntropy, Fp32) => Tolerance::FP32_LAX,
        (KlDivergence | ShannonEntropy, Fp16 | Bf16) => Tolerance::FP16_LAX,

        // Composite (= MoE) = combinaison de plusieurs ops
        (MoeReconcile, Fp32) => Tolerance::FP32_LAX,
        (MoeReconcile, Fp16 | Bf16) => Tolerance::FP16_LAX,
    }
}

/// Compare deux scalaires float avec la tolérance donnée.
///
/// Returns `Ok(())` si proche, `Err(NumericalMismatch)` sinon avec
/// le détail (= utile pour le panic message des tests).
pub fn assert_close_scalar(
    a: f64,
    b: f64,
    tol: Tolerance,
) -> Result<(), NumericalMismatch> {
    if a.is_nan() != b.is_nan() {
        return Err(NumericalMismatch {
            a,
            b,
            rtol_observed: f64::NAN,
            atol_observed: f64::NAN,
            kind: MismatchKind::NanDiscrepancy,
        });
    }
    if a.is_nan() && b.is_nan() {
        return Ok(());      // both NaN = OK (= deterministic NaN)
    }
    if a.is_infinite() || b.is_infinite() {
        return if a == b {
            Ok(())
        } else {
            Err(NumericalMismatch {
                a, b, rtol_observed: f64::INFINITY, atol_observed: f64::INFINITY,
                kind: MismatchKind::InfDiscrepancy,
            })
        };
    }

    let abs_diff = (a - b).abs();
    // Use atol fallback when |b| close to zero
    if b.abs() < 1e-10 {
        return if abs_diff <= tol.atol {
            Ok(())
        } else {
            Err(NumericalMismatch {
                a, b, rtol_observed: f64::NAN, atol_observed: abs_diff,
                kind: MismatchKind::AbsToleranceExceeded,
            })
        };
    }
    let rtol_observed = abs_diff / b.abs();
    if rtol_observed <= tol.rtol {
        Ok(())
    } else {
        Err(NumericalMismatch {
            a, b, rtol_observed, atol_observed: abs_diff,
            kind: MismatchKind::RelToleranceExceeded,
        })
    }
}

/// Compare deux slices scalaires avec une tolérance commune.
///
/// Retourne le **premier** mismatch trouvé (= short-circuit pour
/// les benches sur très grands arrays).
pub fn assert_close_slice(
    a: &[f64],
    b: &[f64],
    tol: Tolerance,
) -> Result<(), SliceNumericalMismatch> {
    if a.len() != b.len() {
        return Err(SliceNumericalMismatch {
            index: usize::MAX,
            element_mismatch: NumericalMismatch {
                a: 0.0, b: 0.0, rtol_observed: 0.0, atol_observed: 0.0,
                kind: MismatchKind::LengthDiscrepancy,
            },
        });
    }
    for (i, (av, bv)) in a.iter().zip(b.iter()).enumerate() {
        if let Err(m) = assert_close_scalar(*av, *bv, tol) {
            return Err(SliceNumericalMismatch {
                index: i,
                element_mismatch: m,
            });
        }
    }
    Ok(())
}

/// Type de divergence numérique détectée.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MismatchKind {
    /// `a` est NaN mais pas `b` (ou inversement).
    NanDiscrepancy,
    /// `a` est Inf mais pas `b` (ou inversement).
    InfDiscrepancy,
    /// `|a-b| > atol` et `|b| ≈ 0` (= fallback abs).
    AbsToleranceExceeded,
    /// `|a-b| / |b| > rtol`.
    RelToleranceExceeded,
    /// Slices de longueurs différentes.
    LengthDiscrepancy,
}

/// Détail du mismatch scalaire (= utile dans le panic message).
#[derive(Debug, Clone, Copy)]
pub struct NumericalMismatch {
    /// Valeur GPU (ou candidate sous test).
    pub a: f64,
    /// Valeur CPU (ou reference).
    pub b: f64,
    /// Relative tolerance observée.
    pub rtol_observed: f64,
    /// Absolute tolerance observée.
    pub atol_observed: f64,
    /// Type de divergence.
    pub kind: MismatchKind,
}

/// Mismatch dans une slice (= position + détail).
#[derive(Debug, Clone, Copy)]
pub struct SliceNumericalMismatch {
    /// Index où le mismatch a été trouvé.
    pub index: usize,
    /// Détail du mismatch scalaire.
    pub element_mismatch: NumericalMismatch,
}

impl std::fmt::Display for NumericalMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "mismatch: gpu={} cpu={} rtol={:.2e} atol={:.2e} kind={:?}",
            self.a, self.b, self.rtol_observed, self.atol_observed, self.kind,
        )
    }
}

impl std::fmt::Display for SliceNumericalMismatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "at index {}: {}", self.index, self.element_mismatch)
    }
}

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_tolerance_zero() {
        assert_eq!(Tolerance::EXACT.rtol, 0.0);
        assert_eq!(Tolerance::EXACT.atol, 0.0);
    }

    #[test]
    fn fp16_default_is_1e_3_rtol() {
        assert!((Tolerance::FP16_DEFAULT.rtol - 1e-3).abs() < 1e-9);
    }

    #[test]
    fn close_scalar_equal_ok() {
        assert!(assert_close_scalar(1.0, 1.0, Tolerance::FP32_STRICT).is_ok());
    }

    #[test]
    fn close_scalar_within_rtol_ok() {
        // 1.0 vs 1.00001 → rtol ≈ 1e-5
        assert!(assert_close_scalar(1.0, 1.00001, Tolerance::FP32_LAX).is_ok());
    }

    #[test]
    fn close_scalar_outside_rtol_err() {
        // 1.0 vs 1.1 → rtol ≈ 0.09
        let r = assert_close_scalar(1.0, 1.1, Tolerance::FP32_LAX);
        assert!(r.is_err());
        let m = r.unwrap_err();
        assert_eq!(m.kind, MismatchKind::RelToleranceExceeded);
    }

    #[test]
    fn close_scalar_both_nan_ok() {
        // NaN deterministe = OK
        assert!(
            assert_close_scalar(f64::NAN, f64::NAN, Tolerance::EXACT).is_ok()
        );
    }

    #[test]
    fn close_scalar_one_nan_err() {
        let r = assert_close_scalar(f64::NAN, 1.0, Tolerance::FP32_STRICT);
        assert!(r.is_err());
        assert_eq!(r.unwrap_err().kind, MismatchKind::NanDiscrepancy);
    }

    #[test]
    fn close_scalar_both_inf_same_sign_ok() {
        assert!(
            assert_close_scalar(f64::INFINITY, f64::INFINITY, Tolerance::EXACT)
                .is_ok()
        );
    }

    #[test]
    fn close_scalar_inf_diff_signs_err() {
        let r = assert_close_scalar(f64::INFINITY, f64::NEG_INFINITY, Tolerance::EXACT);
        assert!(r.is_err());
        assert_eq!(r.unwrap_err().kind, MismatchKind::InfDiscrepancy);
    }

    #[test]
    fn close_scalar_near_zero_uses_atol() {
        // b ≈ 0 → fallback atol
        let r = assert_close_scalar(1e-12, 1e-13, Tolerance::FP32_STRICT);
        assert!(r.is_ok(), "abs diff < atol");
        let r2 = assert_close_scalar(0.1, 0.0, Tolerance::FP32_STRICT);
        assert!(r2.is_err());
        assert_eq!(r2.unwrap_err().kind, MismatchKind::AbsToleranceExceeded);
    }

    #[test]
    fn close_slice_equal_ok() {
        let a = vec![1.0, 2.0, 3.0];
        let b = vec![1.0, 2.0, 3.0];
        assert!(assert_close_slice(&a, &b, Tolerance::FP32_STRICT).is_ok());
    }

    #[test]
    fn close_slice_length_mismatch_err() {
        let a = vec![1.0, 2.0];
        let b = vec![1.0, 2.0, 3.0];
        let r = assert_close_slice(&a, &b, Tolerance::FP32_STRICT);
        assert!(r.is_err());
        assert_eq!(
            r.unwrap_err().element_mismatch.kind,
            MismatchKind::LengthDiscrepancy
        );
    }

    #[test]
    fn close_slice_returns_first_mismatch() {
        let a = vec![1.0, 2.0, 9.9];
        let b = vec![1.0, 2.0, 3.0];
        let r = assert_close_slice(&a, &b, Tolerance::FP32_STRICT);
        assert!(r.is_err());
        let m = r.unwrap_err();
        assert_eq!(m.index, 2);
    }

    #[test]
    fn canonical_tolerance_per_kernel_table() {
        // Pin contractuel : table doc Φ-D § 8.2.
        // 12 kernels × 3 precisions = 36 lookups ; tout doit retourner
        // une Tolerance valide (= rtol ≥ 0, atol ≥ 0).
        for &k in KernelId::all() {
            for p in [Precision::Fp32, Precision::Fp16, Precision::Bf16] {
                let tol = canonical_tolerance(k, p);
                assert!(tol.rtol >= 0.0, "kernel {} prec {:?} rtol < 0", k.as_label(), p);
                assert!(tol.atol >= 0.0);
            }
        }
    }

    #[test]
    fn argmax_tolerance_always_exact() {
        for p in [Precision::Fp32, Precision::Fp16, Precision::Bf16] {
            assert_eq!(
                canonical_tolerance(KernelId::Argmax, p),
                Tolerance::EXACT,
                "argmax must always be exact regardless of precision"
            );
        }
    }

    #[test]
    fn fp16_tolerance_always_larger_than_fp32() {
        // Pin contractuel : fp16 a forcément une tolerance ≥ fp32
        // pour le même kernel (= sinon test fp16 fail sur des deltas
        // fp32-OK).
        for &k in KernelId::all() {
            let tol_fp32 = canonical_tolerance(k, Precision::Fp32);
            let tol_fp16 = canonical_tolerance(k, Precision::Fp16);
            if matches!(k, KernelId::Argmax) {
                continue; // exact tous deux
            }
            assert!(
                tol_fp16.rtol >= tol_fp32.rtol,
                "kernel {} : fp16 rtol {} should be >= fp32 rtol {}",
                k.as_label(), tol_fp16.rtol, tol_fp32.rtol,
            );
        }
    }

    #[test]
    fn mismatch_display_includes_values() {
        let m = NumericalMismatch {
            a: 1.0, b: 0.95, rtol_observed: 0.05, atol_observed: 0.05,
            kind: MismatchKind::RelToleranceExceeded,
        };
        let s = format!("{}", m);
        assert!(s.contains("gpu=1"));
        assert!(s.contains("cpu=0.95"));
        assert!(s.contains("rtol"));
    }

    #[test]
    fn slice_mismatch_display_includes_index() {
        let s = SliceNumericalMismatch {
            index: 42,
            element_mismatch: NumericalMismatch {
                a: 1.0, b: 2.0, rtol_observed: 0.5, atol_observed: 1.0,
                kind: MismatchKind::RelToleranceExceeded,
            },
        };
        let out = format!("{}", s);
        assert!(out.contains("index 42"));
    }
}
