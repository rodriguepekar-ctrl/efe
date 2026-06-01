// SPDX-License-Identifier: UNLICENSED
//! Sprint **Φ-D D32 (Bundle completion)** — auto-tune subcommand
//! + per-architecture tile sizes.
//!
//! Cf. `docs/EFE_CUDA_KERNELS_COMPLETION_ARCHITECTURE.md` § 6.3 +
//! § 15 artefact #21.
//!
//! ## Mission
//!
//! Au boot du serveur, choisir les **tile sizes optimaux** pour
//! chaque kernel selon la compute capability du GPU détecté.
//! Sans cela, on utilise des defaults sub-optimaux (= 32 threads/block
//! universel) ce qui laisse 20-40 % de perf sur la table.
//!
//! La logique est :
//!
//! 1. Détecter la SM (e.g. 7.5 = T4, 8.0 = A100, 9.0 = H100).
//! 2. Lookup la **table canonique** des tile sizes par (kernel × SM).
//! 3. Exporter via env vars `EFE_RUNTIME_GPU_*` qui sont lues par
//!    [`crate::kernels::CudaBackend::launch`].
//!
//! ## Architecture
//!
//! - **Pure function** : la table est codée en dur ; le lookup est
//!   `O(1)`. Aucun runtime probing.
//! - **No CUDA needed** : compile sans la feature ; les tests
//!   tournent sur Mac / Windows.
//! - **CLI binary** : `efe-server tune` invoque ce module et écrit
//!   les env vars à stdout (= consommable par un `eval $(...)`).
//!
//! ## API
//!
//! ```ignore
//! let tile = TileSize::for_kernel_and_gpu(KernelId::FusedSoftmax, ComputeCap(7, 5));
//! assert_eq!(tile.block_size, 32);
//! ```

use std::collections::HashMap;
use std::fmt;

/// Identifiant canonique des 12 kernels Φ-D.
///
/// Aligné sur les 12 kernels énumérés dans le doc Φ-D § 6.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KernelId {
    /// Kernel #1 — fused softmax + entropy (P0 priorité).
    FusedSoftmaxEntropy,
    /// Kernel #2 — weighted mean batched.
    WeightedMean,
    /// Kernel #3 — cone project batched.
    ConeProject,
    /// Kernel #4 — L2 normalize batched.
    L2Normalize,
    /// Kernel #5 — cosine similarity batched.
    CosineSimilarity,
    /// Kernel #6 — L2 distance batched.
    L2Distance,
    /// Kernel #7 — mean pooling over seq.
    MeanPooling,
    /// Kernel #8 — argmax reduction.
    Argmax,
    /// Kernel #9 — top-K.
    TopK,
    /// Kernel #10 — KL divergence.
    KlDivergence,
    /// Kernel #11 — Shannon entropy.
    ShannonEntropy,
    /// Kernel #12 — MoE reconcile (Mode-2 hook).
    MoeReconcile,
}

impl KernelId {
    /// Stable string (= label Prometheus + env var name).
    #[must_use]
    pub const fn as_label(self) -> &'static str {
        match self {
            Self::FusedSoftmaxEntropy => "fused_softmax_entropy",
            Self::WeightedMean => "weighted_mean",
            Self::ConeProject => "cone_project",
            Self::L2Normalize => "l2_normalize",
            Self::CosineSimilarity => "cosine_similarity",
            Self::L2Distance => "l2_distance",
            Self::MeanPooling => "mean_pooling",
            Self::Argmax => "argmax",
            Self::TopK => "topk",
            Self::KlDivergence => "kl_divergence",
            Self::ShannonEntropy => "shannon_entropy",
            Self::MoeReconcile => "moe_reconcile",
        }
    }

    /// Itérateur canonique. Pin contractuel : 12 kernels exactement.
    #[must_use]
    pub const fn all() -> &'static [KernelId; 12] {
        &[
            Self::FusedSoftmaxEntropy,
            Self::WeightedMean,
            Self::ConeProject,
            Self::L2Normalize,
            Self::CosineSimilarity,
            Self::L2Distance,
            Self::MeanPooling,
            Self::Argmax,
            Self::TopK,
            Self::KlDivergence,
            Self::ShannonEntropy,
            Self::MoeReconcile,
        ]
    }

    /// Priorité (P0/P1/P2/P3) selon doc Φ-D § 6.1.
    #[must_use]
    pub const fn priority(self) -> Priority {
        match self {
            Self::FusedSoftmaxEntropy
            | Self::WeightedMean
            | Self::ConeProject
            | Self::L2Normalize => Priority::P0,
            Self::CosineSimilarity
            | Self::L2Distance
            | Self::MeanPooling
            | Self::Argmax => Priority::P1,
            Self::TopK | Self::KlDivergence => Priority::P2,
            Self::ShannonEntropy | Self::MoeReconcile => Priority::P3,
        }
    }
}

/// Priorité d'implémentation. P0 = bloque GA ; P3 = peut slipper v1.1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    /// P0 — bloquant pour GA (4 kernels).
    P0,
    /// P1 — required pour GA (4 kernels).
    P1,
    /// P2 — souhaitable pour GA (2 kernels).
    P2,
    /// P3 — peut slipper v1.1 (2 kernels).
    P3,
}

/// Compute capability (= `(major, minor)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ComputeCap(pub u8, pub u8);

impl ComputeCap {
    /// NVIDIA T4 (= sm_75 = compute 7.5). Baseline GA contractuelle.
    pub const T4: Self = Self(7, 5);
    /// NVIDIA L4 (= sm_89).
    pub const L4: Self = Self(8, 9);
    /// NVIDIA A10 (= sm_86).
    pub const A10: Self = Self(8, 6);
    /// NVIDIA A100 (= sm_80).
    pub const A100: Self = Self(8, 0);
    /// NVIDIA H100 (= sm_90).
    pub const H100: Self = Self(9, 0);

    /// Itérateur canonique des 5 architectures supportées Φ-D.
    #[must_use]
    pub const fn supported_archs() -> &'static [ComputeCap; 5] {
        &[Self::T4, Self::L4, Self::A10, Self::A100, Self::H100]
    }

    /// SM label (e.g. "sm_75").
    #[must_use]
    pub fn as_sm_label(self) -> String {
        format!("sm_{}{}", self.0, self.1)
    }
}

impl fmt::Display for ComputeCap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.0, self.1)
    }
}

/// Tile size pour un kernel donné. Lance config CUDA (block × grid).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileSize {
    /// Threads par block.
    pub block_size: u32,
    /// Hint pour le grid size (= ceil(N / block_size) × block_per_sm).
    pub blocks_per_sm: u32,
    /// Shared memory bytes par block.
    pub shared_mem_bytes: u32,
}

impl TileSize {
    /// Lookup la tile size optimal pour `kernel × compute_cap`.
    ///
    /// Table dérivée des profils NSight Compute (= cf. doc Φ-D § 6.3).
    /// Fallback `default()` si combo non-tabulé.
    #[must_use]
    pub fn for_kernel_and_gpu(kernel: KernelId, cap: ComputeCap) -> Self {
        // Table canonique : (kernel, compute_cap) → (block_size, blocks_per_sm, smem)
        match (kernel, cap) {
            // === FusedSoftmaxEntropy ===
            (KernelId::FusedSoftmaxEntropy, ComputeCap::T4) => Self {
                block_size: 32, blocks_per_sm: 4, shared_mem_bytes: 256,
            },
            (KernelId::FusedSoftmaxEntropy, ComputeCap::L4) => Self {
                block_size: 64, blocks_per_sm: 4, shared_mem_bytes: 512,
            },
            (KernelId::FusedSoftmaxEntropy, ComputeCap::A10) => Self {
                block_size: 64, blocks_per_sm: 4, shared_mem_bytes: 512,
            },
            (KernelId::FusedSoftmaxEntropy, ComputeCap::A100) => Self {
                block_size: 128, blocks_per_sm: 8, shared_mem_bytes: 1024,
            },
            (KernelId::FusedSoftmaxEntropy, ComputeCap::H100) => Self {
                block_size: 256, blocks_per_sm: 8, shared_mem_bytes: 2048,
            },

            // === WeightedMean (= memory-bound) ===
            (KernelId::WeightedMean, ComputeCap::T4) => Self {
                block_size: 256, blocks_per_sm: 2, shared_mem_bytes: 0,
            },
            (KernelId::WeightedMean, ComputeCap::A100) => Self {
                block_size: 256, blocks_per_sm: 4, shared_mem_bytes: 0,
            },

            // === ConeProject ===
            (KernelId::ConeProject, ComputeCap::T4) => Self {
                block_size: 128, blocks_per_sm: 4, shared_mem_bytes: 1024,
            },
            (KernelId::ConeProject, ComputeCap::A100) => Self {
                block_size: 256, blocks_per_sm: 8, shared_mem_bytes: 2048,
            },

            // === L2Normalize (= simple, very memory-bound) ===
            (KernelId::L2Normalize, ComputeCap::T4) => Self {
                block_size: 128, blocks_per_sm: 4, shared_mem_bytes: 0,
            },
            (KernelId::L2Normalize, ComputeCap::A100) => Self {
                block_size: 256, blocks_per_sm: 8, shared_mem_bytes: 0,
            },

            // Default fallback (= conservative, marche sur tout)
            _ => Self::default(),
        }
    }
}

impl Default for TileSize {
    fn default() -> Self {
        Self {
            block_size: 128,
            blocks_per_sm: 4,
            shared_mem_bytes: 0,
        }
    }
}

/// Tune report : la collection de toutes les `TileSize` recommandées
/// pour la GPU détectée. Sérialisable en env vars.
#[derive(Debug, Clone)]
pub struct TuneReport {
    /// Compute capability détectée.
    pub cap: ComputeCap,
    /// Map kernel → tile size.
    pub tiles: HashMap<KernelId, TileSize>,
}

impl TuneReport {
    /// Construit un report pour la `cap` donnée en interrogeant la
    /// table canonique pour les 12 kernels.
    #[must_use]
    pub fn for_compute_cap(cap: ComputeCap) -> Self {
        let mut tiles = HashMap::with_capacity(12);
        for &k in KernelId::all() {
            tiles.insert(k, TileSize::for_kernel_and_gpu(k, cap));
        }
        Self { cap, tiles }
    }

    /// Format en lignes `export EFE_RUNTIME_GPU_<KERNEL>_BLOCK=X`
    /// (= consommable par `eval $(efe-server tune)`).
    #[must_use]
    pub fn as_env_exports(&self) -> Vec<String> {
        let mut out = Vec::with_capacity(36);
        out.push(format!("export EFE_RUNTIME_GPU_COMPUTE_CAP={}", self.cap));
        out.push(format!(
            "export EFE_RUNTIME_GPU_SM_LABEL={}",
            self.cap.as_sm_label()
        ));
        // Stable order (= déterministe) par énumération canonique.
        for &k in KernelId::all() {
            let t = &self.tiles[&k];
            let upper = k.as_label().to_uppercase();
            out.push(format!(
                "export EFE_RUNTIME_GPU_{}_BLOCK={}",
                upper, t.block_size
            ));
            out.push(format!(
                "export EFE_RUNTIME_GPU_{}_BLOCKS_PER_SM={}",
                upper, t.blocks_per_sm
            ));
            if t.shared_mem_bytes > 0 {
                out.push(format!(
                    "export EFE_RUNTIME_GPU_{}_SMEM={}",
                    upper, t.shared_mem_bytes
                ));
            }
        }
        out
    }
}

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kernel_count_canonical_12() {
        // Pin contractuel Φ-D § 6.1 : 12 kernels exactement.
        assert_eq!(KernelId::all().len(), 12);
    }

    #[test]
    fn kernel_labels_distinct() {
        let mut labels: Vec<&str> =
            KernelId::all().iter().map(|k| k.as_label()).collect();
        labels.sort();
        labels.dedup();
        assert_eq!(labels.len(), 12);
    }

    #[test]
    fn priority_partition_p0_4_p1_4_p2_2_p3_2() {
        let p0 = KernelId::all().iter().filter(|k| k.priority() == Priority::P0).count();
        let p1 = KernelId::all().iter().filter(|k| k.priority() == Priority::P1).count();
        let p2 = KernelId::all().iter().filter(|k| k.priority() == Priority::P2).count();
        let p3 = KernelId::all().iter().filter(|k| k.priority() == Priority::P3).count();
        assert_eq!(p0, 4, "Φ-D § 6.1 says 4 P0 kernels");
        assert_eq!(p1, 4, "Φ-D § 6.1 says 4 P1 kernels");
        assert_eq!(p2, 2, "Φ-D § 6.1 says 2 P2 kernels");
        assert_eq!(p3, 2, "Φ-D § 6.1 says 2 P3 kernels");
        assert_eq!(p0 + p1 + p2 + p3, 12);
    }

    #[test]
    fn supported_archs_count_5() {
        // Pin contractuel Φ-D : 5 archs (T4, L4, A10, A100, H100).
        assert_eq!(ComputeCap::supported_archs().len(), 5);
    }

    #[test]
    fn sm_label_format() {
        assert_eq!(ComputeCap::T4.as_sm_label(), "sm_75");
        assert_eq!(ComputeCap::A100.as_sm_label(), "sm_80");
        assert_eq!(ComputeCap::H100.as_sm_label(), "sm_90");
    }

    #[test]
    fn compute_cap_display() {
        assert_eq!(format!("{}", ComputeCap::T4), "7.5");
        assert_eq!(format!("{}", ComputeCap::A100), "8.0");
    }

    #[test]
    fn fused_softmax_t4_uses_warp_size_block() {
        // Doc Φ-D § 6.3 : T4 fused_softmax tile = 32 (= 1 warp).
        let t = TileSize::for_kernel_and_gpu(
            KernelId::FusedSoftmaxEntropy,
            ComputeCap::T4,
        );
        assert_eq!(t.block_size, 32);
    }

    #[test]
    fn fused_softmax_h100_uses_larger_block() {
        // H100 a plus de SMs + plus de shared memory.
        let h = TileSize::for_kernel_and_gpu(
            KernelId::FusedSoftmaxEntropy,
            ComputeCap::H100,
        );
        let t = TileSize::for_kernel_and_gpu(
            KernelId::FusedSoftmaxEntropy,
            ComputeCap::T4,
        );
        assert!(h.block_size > t.block_size);
    }

    #[test]
    fn unknown_combo_falls_back_to_default() {
        // Si on cherche un kernel sans entry dans la table, on
        // récupère le default conservative.
        let t = TileSize::for_kernel_and_gpu(KernelId::TopK, ComputeCap::H100);
        assert_eq!(t, TileSize::default());
    }

    #[test]
    fn tune_report_for_t4_has_all_12_kernels() {
        let r = TuneReport::for_compute_cap(ComputeCap::T4);
        assert_eq!(r.tiles.len(), 12);
    }

    #[test]
    fn env_exports_include_compute_cap() {
        let r = TuneReport::for_compute_cap(ComputeCap::T4);
        let exports = r.as_env_exports();
        assert!(exports
            .iter()
            .any(|e| e.contains("EFE_RUNTIME_GPU_COMPUTE_CAP=7.5")));
        assert!(exports
            .iter()
            .any(|e| e.contains("EFE_RUNTIME_GPU_SM_LABEL=sm_75")));
    }

    #[test]
    fn env_exports_include_all_kernel_blocks() {
        let r = TuneReport::for_compute_cap(ComputeCap::T4);
        let exports = r.as_env_exports();
        for k in KernelId::all() {
            let upper = k.as_label().to_uppercase();
            let needle = format!("EFE_RUNTIME_GPU_{}_BLOCK=", upper);
            assert!(
                exports.iter().any(|e| e.contains(&needle)),
                "missing {needle} in exports"
            );
        }
    }

    #[test]
    fn env_exports_are_eval_safe() {
        // Chaque export doit être un statement shell valide
        // (= `export NAME=value`). On vérifie au moins le prefix.
        let r = TuneReport::for_compute_cap(ComputeCap::A100);
        for line in r.as_env_exports() {
            assert!(line.starts_with("export EFE_RUNTIME_GPU_"));
            assert!(line.contains('='));
            assert!(!line.contains('\n'), "no newline in exports");
        }
    }

    #[test]
    fn tile_size_block_must_be_power_of_2_or_warp_multiple() {
        // CUDA convention : block_size doit être multiple de 32 (= warp)
        // ou bien 32 lui-même.
        for &cap in ComputeCap::supported_archs() {
            for &k in KernelId::all() {
                let t = TileSize::for_kernel_and_gpu(k, cap);
                assert!(
                    t.block_size % 32 == 0,
                    "kernel {} cap {} block {} not multiple of 32",
                    k.as_label(),
                    cap,
                    t.block_size
                );
                assert!(t.block_size <= 1024, "block > 1024 CUDA invalid");
            }
        }
    }
}
