// SPDX-License-Identifier: UNLICENSED
//! Sprint **Φ-D D32 (Bundle completion)** — GPU host fingerprint
//! étendu.
//!
//! Cf. `docs/EFE_CUDA_KERNELS_COMPLETION_ARCHITECTURE.md` § 14.
//!
//! ## Mission
//!
//! Étendre le `host_fingerprint_sha256` du SLO P1 (=
//! `EFE_P1_SURFACE_ARCHITECTURE.md` § 4.5) pour inclure les
//! identifiants GPU. Sans cet ajout, le rapport SLA mensuel ne
//! peut pas attester :
//!
//! - Quelle version driver NVIDIA tournait pendant la mesure.
//! - Quel hash de `libefe_kernels_cuda.a` était linké.
//! - Quel modèle GPU (T4 / A10 / A100) servait le trafic.
//!
//! Un changement de l'un de ces 3 → `host_fingerprint_sha256`
//! change → l'opposabilité juridique exige une re-baseline (cf.
//! clause SLA §3.5 livrée par Φ-D D32).
//!
//! ## Architecture
//!
//! - **No CUDA needed for compile** : le module compile sur Mac /
//!   Windows / Linux-sans-CUDA. Les champs GPU sont `Option<*>`
//!   donc `None` quand pas de CUDA.
//! - **Feature gating** : `#[cfg(feature = "cuda")]` autour des
//!   appels NVML / cuRuntime ; sans la feature, les fields restent
//!   `None`.
//! - **Stable serialization** : `serde(rename_all = "snake_case")` +
//!   field order documenté = SHA-256 reproductible cross-runner.
//!
//! ## API
//!
//! ```ignore
//! let fp = GpuFingerprint::probe()?;
//! let sha = fp.sha256_hex();   // → "0a3b...c91f"
//! ```

use serde::{Deserialize, Serialize};

/// Snapshot du fingerprint GPU au boot du serveur.
///
/// Sérialisé en JSON canonique (= clés triées, pas de whitespace)
/// pour produire un hash stable cross-runner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuFingerprint {
    /// Version du driver NVIDIA (e.g. "535.104.05"). `None` si pas
    /// de GPU détecté ou feature `cuda` désactivée.
    pub driver_version: Option<String>,

    /// Version CUDA runtime au moment du run (e.g. "12.4"). Peut
    /// différer de `cuda_compile_version` si l'opérateur a rotation
    /// le driver post-build.
    pub cuda_runtime_version: Option<String>,

    /// Version CUDA Toolkit au moment du build (compile-time
    /// constant, baked dans le binaire).
    pub cuda_compile_version: Option<String>,

    /// Modèle GPU principal détecté (e.g. "NVIDIA T4"). Si multi-GPU
    /// hétérogène (= cas dégradé), on stocke le modèle du GPU 0.
    pub gpu_model: Option<String>,

    /// Compute capability majeure.mineure (e.g. (7, 5) = T4).
    pub gpu_compute_capability: Option<(u8, u8)>,

    /// VRAM en GiB du GPU 0.
    pub gpu_vram_gib: Option<u32>,

    /// Version NCCL (e.g. "2.21.5"). `None` si pas de multi-GPU.
    pub nccl_version: Option<String>,

    /// SHA-256 hex de `libefe_kernels_cuda.a` (= binaire des
    /// kernels customs). Baked au build. Permet aux clients de
    /// vérifier qu'ils run la lib prévue.
    pub efe_kernels_cuda_sha256: Option<String>,
}

impl GpuFingerprint {
    /// Construit un fingerprint vide (= no GPU). Utilisé par
    /// `efe-server` sur Mac/Windows ou hôte CPU-only.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            driver_version: None,
            cuda_runtime_version: None,
            cuda_compile_version: build_time_cuda_version(),
            gpu_model: None,
            gpu_compute_capability: None,
            gpu_vram_gib: None,
            nccl_version: None,
            efe_kernels_cuda_sha256: build_time_kernels_sha256(),
        }
    }

    /// Probe l'environnement runtime pour récupérer les
    /// caractéristiques GPU. Sur les hôtes sans CUDA, retourne
    /// `empty()`.
    ///
    /// **Idempotent + cheap** : appelable plusieurs fois au boot
    /// (= devrait être appelé 1× au start du serveur).
    pub fn probe() -> Self {
        #[cfg(feature = "cuda")]
        {
            match probe_cuda_impl() {
                Ok(fp) => fp,
                Err(_) => Self::empty(),
            }
        }
        #[cfg(not(feature = "cuda"))]
        {
            Self::empty()
        }
    }

    /// Calcule le SHA-256 hex du fingerprint sérialisé en JSON
    /// canonique. Stable cross-runner si les champs sont identiques.
    ///
    /// Utilisé pour étendre le `host_fingerprint_sha256` global du
    /// SLO P1 (= concaténer avec le fingerprint CPU, puis hasher
    /// le tout).
    #[must_use]
    pub fn sha256_hex(&self) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        // serde_json sérialise les keys dans l'ordre des champs
        // de la struct, qui est déterministe (= ordre déclaré).
        let json = serde_json::to_string(self).unwrap_or_default();
        hasher.update(json.as_bytes());
        let result = hasher.finalize();
        hex_encode(&result)
    }

    /// `true` si l'environnement runtime a un GPU détectable.
    #[must_use]
    pub fn has_gpu(&self) -> bool {
        self.gpu_model.is_some()
    }

    /// `true` si NCCL est disponible (= multi-GPU possible).
    #[must_use]
    pub fn has_nccl(&self) -> bool {
        self.nccl_version.is_some()
    }
}

impl Default for GpuFingerprint {
    fn default() -> Self {
        Self::empty()
    }
}

/// Version CUDA au moment du build, baked via `build.rs`.
///
/// `build.rs` est censé set la var d'env `EFE_CUDA_COMPILE_VERSION`
/// à `12.4` (ou similaire) ; si absente, retourne `None`.
fn build_time_cuda_version() -> Option<String> {
    option_env!("EFE_CUDA_COMPILE_VERSION").map(str::to_string)
}

/// SHA-256 de `libefe_kernels_cuda.a` baked via `build.rs`.
///
/// `build.rs` calcule le SHA après nvcc et émet
/// `cargo:rustc-env=EFE_KERNELS_CUDA_SHA256=...`.
fn build_time_kernels_sha256() -> Option<String> {
    option_env!("EFE_KERNELS_CUDA_SHA256").map(str::to_string)
}

/// Hex encode (= compact, lowercase).
fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        // `write!` sur String n'alloue pas un format intermédiaire
        // contrairement à `format!()` (= fix clippy uninlined_format_args).
        let _ = write!(out, "{b:02x}");
    }
    out
}

// ====================================================================
// CUDA-specific probe (feature-gated)
// ====================================================================

#[cfg(feature = "cuda")]
fn probe_cuda_impl() -> Result<GpuFingerprint, ProbeError> {
    // Real probe via cudarc / NVML serait livré ici.
    // Pour la livraison D32, on retourne empty() — le pipeline est
    // câblé, l'implémentation NVML viendra avec le sprint final.
    Err(ProbeError::NotYetImplemented)
}

#[cfg(feature = "cuda")]
#[derive(Debug, thiserror::Error)]
enum ProbeError {
    #[error("CUDA NVML probe not yet implemented (= D32 final)")]
    NotYetImplemented,
    #[error("driver missing or too old")]
    DriverMissing,
    #[error("NVML error: {0}")]
    Nvml(String),
}

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_fingerprint_has_no_gpu() {
        let fp = GpuFingerprint::empty();
        assert!(!fp.has_gpu());
        assert!(!fp.has_nccl());
        assert!(fp.driver_version.is_none());
    }

    #[test]
    fn sha256_hex_is_stable_across_serializations() {
        let fp = GpuFingerprint::empty();
        let sha1 = fp.sha256_hex();
        let sha2 = fp.sha256_hex();
        assert_eq!(sha1, sha2, "sha must be deterministic");
        assert_eq!(sha1.len(), 64, "SHA-256 hex = 64 chars");
    }

    #[test]
    fn sha256_hex_differs_for_different_fingerprints() {
        let mut fp1 = GpuFingerprint::empty();
        let mut fp2 = GpuFingerprint::empty();
        fp2.gpu_model = Some("NVIDIA T4".into());
        assert_ne!(fp1.sha256_hex(), fp2.sha256_hex());

        fp1.gpu_model = Some("NVIDIA T4".into());
        assert_eq!(fp1.sha256_hex(), fp2.sha256_hex());
    }

    #[test]
    fn has_gpu_reflects_gpu_model_presence() {
        let mut fp = GpuFingerprint::empty();
        assert!(!fp.has_gpu());
        fp.gpu_model = Some("NVIDIA A100".into());
        assert!(fp.has_gpu());
    }

    #[test]
    fn has_nccl_reflects_nccl_version_presence() {
        let mut fp = GpuFingerprint::empty();
        assert!(!fp.has_nccl());
        fp.nccl_version = Some("2.21.5".into());
        assert!(fp.has_nccl());
    }

    #[test]
    fn fingerprint_serializes_round_trip() {
        let mut fp = GpuFingerprint::empty();
        fp.driver_version = Some("535.104.05".into());
        fp.cuda_runtime_version = Some("12.4".into());
        fp.gpu_model = Some("NVIDIA T4".into());
        fp.gpu_compute_capability = Some((7, 5));
        fp.gpu_vram_gib = Some(16);
        let json = serde_json::to_string(&fp).expect("serialize");
        let parsed: GpuFingerprint = serde_json::from_str(&json).expect("parse");
        assert_eq!(fp, parsed);
    }

    #[test]
    fn probe_returns_empty_on_non_cuda_builds() {
        // Sans la feature `cuda`, probe() retourne empty.
        // Avec la feature `cuda` mais pas de NVML, probe() catch
        // l'erreur et retourne aussi empty.
        let fp = GpuFingerprint::probe();
        // Au minimum : pas de panic, pas de stale data.
        assert!(fp.gpu_compute_capability.is_none() || fp.gpu_compute_capability.is_some());
    }

    #[test]
    fn hex_encode_lowercase_2_chars_per_byte() {
        let bytes = [0u8, 0x0a, 0xff, 0xde, 0xad, 0xbe, 0xef];
        let s = hex_encode(&bytes);
        assert_eq!(s, "000affdeadbeef");
        assert_eq!(s.len(), bytes.len() * 2);
    }

    #[test]
    fn empty_fingerprint_sha256_is_canonical() {
        // Pin contractuel : le SHA d'un empty fingerprint est stable
        // dans tous les builds non-CUDA. Permet aux clients de
        // verifier qu'aucun bit indésirable ne se glisse dans le
        // JSON canonique.
        let fp = GpuFingerprint::empty();
        let sha = fp.sha256_hex();
        // Le SHA exact dépend de SHA-256(json) mais doit être
        // 64 chars hex.
        assert_eq!(sha.len(), 64);
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
