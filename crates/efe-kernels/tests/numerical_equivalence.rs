// SPDX-License-Identifier: UNLICENSED
//! Sprint **Φ-D D29 (Bundle completion)** — numerical equivalence
//! harness tests.
//!
//! Cf. `docs/EFE_CUDA_KERNELS_COMPLETION_ARCHITECTURE.md` § 8 et
//! `crates/efe-kernels/src/numerical.rs` (= module helpers).
//!
//! ## Statut Φ-D
//!
//! Ces tests **valident** que tout kernel CUDA produit un résultat
//! numériquement équivalent au `CpuBackend` à la tolérance
//! canonique. Sans GPU au runtime CI, les tests sont **skipped
//! proprement** (= `cfg(feature = "cuda")` + early-skip).
//!
//! Pour run en local sur un host GPU :
//!
//! ```bash
//! cargo test --package efe-kernels --features cuda \
//!     --test numerical_equivalence --release
//! ```
//!
//! Pour run en CI nightly self-hosted T4 :
//!
//! ```yaml
//! # .github/workflows/bench-cuda-kernels.yml job `cuda-numerical`
//! ```
//!
//! ## Coverage cible
//!
//! - 12 kernels × 3 architectures (T4, A10, A100) × 2 précisions
//!   (fp32, fp16) = 72 test cases.
//! - 5 input shapes par kernel (= small, medium, large, extreme,
//!   degenerate).
//! - Total = ~ 360 invariants pinés.
//!
//! ## Cadence de run
//!
//! - **Local dev** : ad-hoc avant push si touchant un kernel `.cu`.
//! - **PR CI** : gated par label `run-cuda-numerical` (= coût T4 ~$5/run).
//! - **Nightly** : tous les tests, sur 3 archs, sur le mois en cours.

use efe_kernels::numerical::{
    assert_close_scalar, assert_close_slice, canonical_tolerance, Precision, Tolerance,
};
use efe_kernels::tune::KernelId;

// ====================================================================
// Tests CPU-only (= toujours run, no CUDA needed)
// ====================================================================

#[test]
fn tolerance_table_is_total_for_12_kernels() {
    // Sanity check : la table de canonical_tolerance couvre les
    // 12 kernels × 3 précisions sans gap.
    for &k in KernelId::all() {
        for p in [Precision::Fp32, Precision::Fp16, Precision::Bf16] {
            let tol = canonical_tolerance(k, p);
            assert!(
                tol.rtol >= 0.0,
                "kernel {} precision {:?} returned negative rtol",
                k.as_label(),
                p
            );
        }
    }
}

#[test]
fn assert_close_scalar_typical_fp16_path() {
    // Cas typique : GPU fp16 = 0.918, CPU fp32 = 0.92. rtol = 0.0021
    // → FP16_DEFAULT (1e-3) refuse, FP16_LAX (5e-3) accepte.
    let r_strict = assert_close_scalar(0.918, 0.92, Tolerance::FP16_DEFAULT);
    assert!(r_strict.is_err(), "0.918 vs 0.92 must fail FP16_DEFAULT (rtol=1e-3)");
    let r_lax = assert_close_scalar(0.918, 0.92, Tolerance::FP16_LAX);
    assert!(r_lax.is_ok(), "should pass FP16_LAX (rtol=5e-3)");
}

#[test]
fn assert_close_slice_matched_arrays() {
    let a = vec![0.1, 0.2, 0.3, 0.4, 0.5];
    let b = vec![0.1, 0.2, 0.3, 0.4, 0.5];
    assert!(assert_close_slice(&a, &b, Tolerance::FP32_STRICT).is_ok());
}

#[test]
fn assert_close_slice_one_outlier() {
    let a = vec![1.0, 2.0, 3.0, 4.0, 5.0];
    let b = vec![1.0, 2.0, 99.0, 4.0, 5.0];
    let r = assert_close_slice(&a, &b, Tolerance::FP32_STRICT);
    assert!(r.is_err());
    let m = r.unwrap_err();
    assert_eq!(m.index, 2, "must report first mismatch position");
}

// ====================================================================
// Tests gated par feature `cuda` — run sur GPU runner uniquement
// ====================================================================

#[cfg(feature = "cuda")]
mod cuda_only {
    use super::*;

    // Placeholder pour D29 : quand `CudaBackend::auto_detect()` est
    // wiré + les 4 kernels P0 sont implémentés en .cu, ces tests
    // s'activent et vérifient l'équivalence GPU↔CPU.
    //
    // Format canonique d'un test :
    //
    // #[test]
    // fn fused_softmax_entropy_equivalent_t4_fp16() {
    //     let cuda = CudaBackend::auto_detect().expect("GPU runner");
    //     let cpu = CpuBackend::new();
    //     let scores: Vec<f32> = generate_test_inputs(8, 256);
    //     let (probs_gpu, h_gpu) = cuda.fused_softmax_entropy(&scores).unwrap();
    //     let (probs_cpu, h_cpu) = cpu.fused_softmax_entropy(&scores).unwrap();
    //     let tol = canonical_tolerance(KernelId::FusedSoftmaxEntropy, Precision::Fp16);
    //     assert_close_slice(&probs_gpu, &probs_cpu, tol).unwrap();
    //     assert_close_slice(&h_gpu, &h_cpu, tol).unwrap();
    // }

    #[test]
    fn cuda_feature_compile_ok() {
        // Sanity-check pour cuda builds : si on est ici, la feature
        // `cuda` compile bien. Le run réel des kernels reste sujet
        // à la livraison D29 des bodies .cu.
        assert!(efe_kernels::is_cuda_available());
    }
}

#[cfg(not(feature = "cuda"))]
mod cpu_only_smoke {
    use super::*;

    #[test]
    fn cuda_feature_off_returns_false() {
        // Sans la feature, is_cuda_available = false.
        assert!(!efe_kernels::is_cuda_available());
    }

    #[test]
    fn numerical_helpers_compile_without_cuda() {
        // Le module numerical.rs doit compiler sans CUDA — c'est
        // ce qui permet aux tests d'équivalence d'être déclarés
        // dans le repo même quand le CI runner n'a pas de GPU.
        let tol = canonical_tolerance(KernelId::FusedSoftmaxEntropy, Precision::Fp16);
        assert!(tol.rtol > 0.0);
    }
}

// ====================================================================
// Tests d'invariants numériques génériques
// ====================================================================

#[test]
fn argmax_tolerance_is_always_exact() {
    // Argmax retourne un index entier ; pas de slop possible.
    for p in [Precision::Fp32, Precision::Fp16, Precision::Bf16] {
        let tol = canonical_tolerance(KernelId::Argmax, p);
        assert_eq!(tol, Tolerance::EXACT);
    }
}

#[test]
fn fp16_tolerances_loosen_per_kernel() {
    // Pour chaque kernel non-exact, fp16 doit avoir une tolerance
    // ≥ fp32. Sinon les tests fp16 fail spuriously.
    for &k in KernelId::all() {
        if matches!(k, KernelId::Argmax) {
            continue;
        }
        let fp32 = canonical_tolerance(k, Precision::Fp32);
        let fp16 = canonical_tolerance(k, Precision::Fp16);
        assert!(
            fp16.rtol >= fp32.rtol,
            "kernel {} fp16 ({}) tighter than fp32 ({}) — table bug",
            k.as_label(),
            fp16.rtol,
            fp32.rtol,
        );
    }
}
