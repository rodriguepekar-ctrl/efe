// SPDX-License-Identifier: UNLICENSED
//! Sprint **Φ-C D26 (Bundle embedder réel)** — Prometheus
//! instruments pour l'inférence bundle production-grade.
//!
//! Cf. `docs/EFE_BUNDLE_EMBEDDER_REAL_ARCHITECTURE.md` § 11.4.
//!
//! ## Inventaire (8 instruments)
//!
//! | # | Métrique                                     | Type      | Labels                                       |
//! |--:|----------------------------------------------|-----------|----------------------------------------------|
//! | 1 | `efe_bundle_loaded_at_boot`                  | Gauge     | (no labels)                                  |
//! | 2 | `efe_bundle_fingerprint_info`                | Gauge     | `bundle_id`, `version`, `manifest_sha256`    |
//! | 3 | `efe_bundle_prewarm_p99_seconds`             | Gauge     | `bundle_id`, `version`                       |
//! | 4 | `efe_bundle_recall_at_1`                     | Gauge     | `bundle_id`, `version`, `language`           |
//! | 5 | `efe_bundle_size_bytes`                      | Gauge     | `bundle_id`, `version`                       |
//! | 6 | `efe_bundle_inference_duration_seconds`      | Histogram | `bundle_id`, `provider`, `batch_size_class`, `seq_len_class` |
//! | 7 | `efe_bundle_verify_total`                    | Counter   | `outcome`                                    |
//! | 8 | `efe_bundle_revoked_total`                   | Counter   | `bundle_id`, `version`                       |
//!
//! ## Cardinalité
//!
//! - `bundle_id` ∈ valeurs canoniques (4 actuels : sample, prod-multi,
//!   prod-en-heavy, custom-per-tenant futur) = 4.
//! - `version` ∈ semver, ≤ 20 versions actives simultanément en
//!   pratique (= mix rotation 6 mois + canary parallèle).
//! - `language` ∈ canonical 10 langs majeures = 10.
//! - `provider` ∈ {`cpu`, `cuda`} = 2.
//! - `batch_size_class` ∈ {`1`, `2-4`, `5-8`, `9-16`, `17-32`} = 5.
//! - `seq_len_class` ∈ {`16`, `64`, `128`, `256`, `512`} = 5.
//! - `outcome` ∈ {`pass`, `hash_mismatch`, `sig_invalid`,
//!   `rekor_missing`, `onnx_load_fail`, `tokenizer_load_fail`} = 6.
//!
//! Cardinalité totale projetée : ~ 1 000 séries au pic, largement
//! sous le budget D13 50 000.
//!
//! ## Doctrine des buckets latence
//!
//! Les buckets de `efe_bundle_inference_duration_seconds` sont
//! calibrés pour que les cibles SLO (CPU fp32 ~ 150 ms, GPU T4 fp16
//! 26 ms) tombent **exactement** sur un bord de bucket — sinon
//! `histogram_quantile(0.99, …)` interpole et perd la précision
//! aux seuils opposables (sanity-check `slo_targets_fall_on_bucket_edges`).

use std::sync::OnceLock;

use prometheus::{
    HistogramOpts, HistogramVec, IntCounterVec, IntGauge, IntGaugeVec, Opts,
};
use serde::{Deserialize, Serialize};

use super::registry::global_registry;

// ====================================================================
// Macro paresse + register-once (= calquée sur p1/p2_instruments).
// ====================================================================

macro_rules! once_metric {
    ($(#[$doc:meta])* $fn_name:ident, $ty:ty, $init:expr) => {
        $(#[$doc])*
        pub fn $fn_name() -> &'static $ty {
            static CELL: OnceLock<$ty> = OnceLock::new();
            CELL.get_or_init(|| {
                let m: $ty = $init;
                let _ = global_registry().register(Box::new(m.clone()));
                m
            })
        }
    };
}

// ====================================================================
// Buckets canoniques inférence (publics pour les benches)
// ====================================================================

/// Buckets pour `efe_bundle_inference_duration_seconds`. Calibrés
/// pour que les cibles canoniques (26 ms GPU fp16 + 180 ms CPU fp32)
/// tombent exactement sur un bord.
pub const BUCKETS_INFERENCE_SECONDS: &[f64] = &[
    0.001, 0.002, 0.005, 0.010, 0.015, 0.020,
    0.026,    // ← cible GPU T4 fp16 SLO-P1-BUDGET-5
    0.030, 0.040, 0.060, 0.080, 0.100, 0.150,
    0.180,    // ← cible CPU fp32 SLO-BUNDLE-CPU
    0.250, 0.500, 1.000,
];

// ====================================================================
// 1. Gauge — bundle_loaded_at_boot
// ====================================================================

once_metric!(
    /// `efe_bundle_loaded_at_boot` — set à 1 quand le bootloader
    /// a vérifié + chargé un bundle, 0 si fail.
    ///
    /// Alerte `BUNDLE_BOOT_FAILED` fire dès que la valeur est 0 ;
    /// indique typiquement qu'un replica boot avec un bundle
    /// corrompu / mal signé / non-trouvé.
    bundle_loaded_at_boot,
    IntGauge,
    IntGauge::with_opts(Opts::new(
        "efe_bundle_loaded_at_boot",
        "1 iff the bundle was verified + loaded at boot (Φ-C § 11.4 #1).",
    ))
    .expect("valid gauge opts")
);

// ====================================================================
// 2. Gauge — bundle_fingerprint_info
// ====================================================================

once_metric!(
    /// `efe_bundle_fingerprint_info{bundle_id, version, manifest_sha256}` —
    /// gauge constant à 1 qui sert d'**info tag** pour identifier
    /// le bundle actif sur ce replica.
    ///
    /// Pattern Prometheus standard : `info` metrics qui exposent
    /// des labels sans valeur de mesure. Permet aux queries de
    /// joindre via `* on(bundle_id, version) group_left(...)`.
    bundle_fingerprint_info,
    IntGaugeVec,
    IntGaugeVec::new(
        Opts::new(
            "efe_bundle_fingerprint_info",
            "Info-style gauge tagging the active bundle (Φ-C § 11.4 #2).",
        ),
        &["bundle_id", "version", "manifest_sha256"],
    )
    .expect("valid gauge opts")
);

// ====================================================================
// 3. Gauge — bundle_prewarm_p99_seconds
// ====================================================================

once_metric!(
    /// `efe_bundle_prewarm_p99_seconds{bundle_id, version}` —
    /// p99 de la phase pre-warm au boot (= 100 inférences dummy
    /// avant `ready=true`).
    ///
    /// Source de **SLO-BUNDLE-PREWARM** (cible ≤ 50 ms).
    ///
    /// Alerte `BUNDLE_PREWARM_SLOW` fire si > 100 ms.
    bundle_prewarm_p99_seconds,
    prometheus::GaugeVec,
    prometheus::GaugeVec::new(
        Opts::new(
            "efe_bundle_prewarm_p99_seconds",
            "p99 of pre-warm inferences at boot (Φ-C § 11.4 #3).",
        ),
        &["bundle_id", "version"],
    )
    .expect("valid gauge opts")
);

// ====================================================================
// 4. Gauge — bundle_recall_at_1
// ====================================================================

once_metric!(
    /// `efe_bundle_recall_at_1{bundle_id, version, language}` —
    /// recall@1 mesuré au build (= benché par
    /// `bundle_recall_bench.py`), persisté dans le manifest puis
    /// exposé au boot.
    ///
    /// Source de **SLO-BUNDLE-RECALL** (cible ≥ 0.92) et
    /// **SLO-BUNDLE-LINGUAL** (max/min ratio ≤ 1.05×).
    bundle_recall_at_1,
    prometheus::GaugeVec,
    prometheus::GaugeVec::new(
        Opts::new(
            "efe_bundle_recall_at_1",
            "recall@1 per language as measured at build time (Φ-C § 11.4 #4).",
        ),
        &["bundle_id", "version", "language"],
    )
    .expect("valid gauge opts")
);

// ====================================================================
// 5. Gauge — bundle_size_bytes
// ====================================================================

once_metric!(
    /// `efe_bundle_size_bytes{bundle_id, version}` — taille
    /// `.tar.gz` du bundle (= compressed wire format).
    ///
    /// Source de **SLO-BUNDLE-SIZE** (cible ≤ 200 MiB =
    /// 209 715 200 bytes).
    bundle_size_bytes,
    IntGaugeVec,
    IntGaugeVec::new(
        Opts::new(
            "efe_bundle_size_bytes",
            "Compressed .tar.gz size of the active bundle (Φ-C § 11.4 #5).",
        ),
        &["bundle_id", "version"],
    )
    .expect("valid gauge opts")
);

// ====================================================================
// 6. Histogram — bundle_inference_duration_seconds (= hot-path)
// ====================================================================

once_metric!(
    /// `efe_bundle_inference_duration_seconds{bundle_id, provider, batch_size_class, seq_len_class}` —
    /// la **métrique de hot-path** de l'étage 5 P1 (= ONNX
    /// inference).
    ///
    /// Source primaire de **SLO-P1-BUDGET-5** (cible 26 ms p99
    /// GPU T4 fp16 batch=1 seq=128). Source secondaire de
    /// **SLO-C-2** (= 800 ms CPU global ; ce histogram en consomme
    /// ~ 180 ms typique).
    bundle_inference_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_bundle_inference_duration_seconds",
            "ONNX inference duration per call (Φ-C § 11.4 #6).",
        )
        .buckets(BUCKETS_INFERENCE_SECONDS.to_vec()),
        &["bundle_id", "provider", "batch_size_class", "seq_len_class"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// 7. Counter — bundle_verify_total (= outcome canonique 6 valeurs)
// ====================================================================

once_metric!(
    /// `efe_bundle_verify_total{outcome}` — résultats des 9 checks
    /// du `efe-bundle verify`. Bumpé au boot ET par la cron de
    /// re-verify horaire (cf. § 5.4 du doc).
    ///
    /// `outcome` est l'un de 6 valeurs canoniques
    /// (cf. [`VerifyOutcome`]). Tout outcome ≠ `pass` déclenche
    /// au minimum une alerte `BUNDLE_VERIFY_DEGRADED`.
    bundle_verify_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_bundle_verify_total",
            "Outcomes of `efe-bundle verify` (Φ-C § 11.4 #7).",
        ),
        &["outcome"],
    )
    .expect("valid counter opts")
);

// ====================================================================
// 8. Counter — bundle_revoked_total
// ====================================================================

once_metric!(
    /// `efe_bundle_revoked_total{bundle_id, version}` — incrémenté
    /// à chaque détection de bundle révoqué (= manifest hash
    /// présent dans la liste `trust-store.toml::revoked`).
    ///
    /// Devrait rester à 0 en steady-state. Toute valeur > 0
    /// déclenche **BUNDLE_REVOKED_DETECTED** (critical) —
    /// cf. runbook `bundle_revocation_emergency.md`.
    bundle_revoked_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_bundle_revoked_total",
            "Revoked bundle detections (Φ-C § 11.4 #8).",
        ),
        &["bundle_id", "version"],
    )
    .expect("valid counter opts")
);

// ====================================================================
// VerifyOutcome — énum canonique pour le label `outcome` #7
// ====================================================================

/// Résultat structuré du `efe-bundle verify`. **6 valeurs
/// canoniques** ; alignées avec la structure `VerifyReport` du
/// binary `efe-bundle` (= cardinalité bornée, anti-DoS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifyOutcome {
    /// Les 9 checks passent.
    Pass,
    /// SHA-256 d'un component mismatch manifest.
    HashMismatch,
    /// Signature Ed25519 invalide ou clé non-trusted.
    SigInvalid,
    /// Entrée Rekor introuvable ou non-Sigstore.
    RekorMissing,
    /// ORT session init échoue (= modèle ONNX corrompu).
    OnnxLoadFail,
    /// Tokenizer JSON malformé.
    TokenizerLoadFail,
}

impl VerifyOutcome {
    /// Stable string pour le label Prometheus.
    #[must_use]
    pub const fn as_label(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::HashMismatch => "hash_mismatch",
            Self::SigInvalid => "sig_invalid",
            Self::RekorMissing => "rekor_missing",
            Self::OnnxLoadFail => "onnx_load_fail",
            Self::TokenizerLoadFail => "tokenizer_load_fail",
        }
    }

    /// Itérateur canonique. Test `all_outcomes_distinct` pin.
    #[must_use]
    pub const fn all() -> &'static [VerifyOutcome; 6] {
        &[
            Self::Pass,
            Self::HashMismatch,
            Self::SigInvalid,
            Self::RekorMissing,
            Self::OnnxLoadFail,
            Self::TokenizerLoadFail,
        ]
    }

    /// `true` si l'outcome déclenche refus de boot.
    #[must_use]
    pub const fn is_fatal(self) -> bool {
        !matches!(self, Self::Pass)
    }
}

// ====================================================================
// Helpers de classification (= labels secondaires)
// ====================================================================

/// Classe de batch size pour `efe_bundle_inference_duration_seconds`.
/// Aligné avec [`crate::metrics::p1_instruments::batch_size_class`]
/// pour cohérence inter-surfaces.
#[must_use]
pub const fn batch_size_class(size: usize) -> &'static str {
    match size {
        0 | 1 => "1",
        2..=4 => "2-4",
        5..=8 => "5-8",
        9..=16 => "9-16",
        _ => "17-32",
    }
}

/// Classe de seq_len pour le label `seq_len_class`. Buckets
/// alignés sur les puissances de 2 du padding tokenizer (= 16, 64,
/// 128, 256, 512).
#[must_use]
pub const fn seq_len_class(seq_len: usize) -> &'static str {
    match seq_len {
        0..=16 => "16",
        17..=64 => "64",
        65..=128 => "128",
        129..=256 => "256",
        _ => "512",
    }
}

/// Provider canonique ONNX runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InferenceProvider {
    /// CPU EP (= ONNX Runtime CPU provider).
    Cpu,
    /// CUDA EP (= ONNX Runtime CUDA provider, T4 / L4 / A10 / A100).
    Cuda,
}

impl InferenceProvider {
    /// Stable string.
    #[must_use]
    pub const fn as_label(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Cuda => "cuda",
        }
    }
}

// ====================================================================
// API ergonomique côté hot-path / bootloader
// ====================================================================

/// Observe l'inférence ONNX (= étage 5 P1).
///
/// Appelé par `OnnxBundleSession::encode_*` après chaque `session.run()`.
/// Le `seq_len` est la longueur effective post-tokenize (avant padding).
pub fn observe_inference(
    bundle_id: &str,
    provider: InferenceProvider,
    batch_size: usize,
    seq_len: usize,
    duration: std::time::Duration,
) {
    bundle_inference_duration_seconds()
        .with_label_values(&[
            bundle_id,
            provider.as_label(),
            batch_size_class(batch_size),
            seq_len_class(seq_len),
        ])
        .observe(duration.as_secs_f64());
}

/// Met à jour les gauges info au boot, après un load réussi.
///
/// Appelé une fois par `Server::build` après que le `efe-bundle
/// verify` ait passé.
pub fn note_bundle_loaded(
    bundle_id: &str,
    version: &str,
    manifest_sha256: &str,
    size_bytes: u64,
    prewarm_p99: std::time::Duration,
) {
    bundle_loaded_at_boot().set(1);
    bundle_fingerprint_info()
        .with_label_values(&[bundle_id, version, manifest_sha256])
        .set(1);
    bundle_size_bytes()
        .with_label_values(&[bundle_id, version])
        .set(size_bytes as i64);
    bundle_prewarm_p99_seconds()
        .with_label_values(&[bundle_id, version])
        .set(prewarm_p99.as_secs_f64());
}

/// Note le recall@1 par langue (= lu du manifest au boot).
pub fn note_recall_at_1(bundle_id: &str, version: &str, language: &str, recall: f64) {
    bundle_recall_at_1()
        .with_label_values(&[bundle_id, version, language])
        .set(recall);
}

/// Note le résultat d'un `efe-bundle verify`. Appelé au boot et
/// par la cron horaire de re-verify.
pub fn note_verify(outcome: VerifyOutcome) {
    bundle_verify_total()
        .with_label_values(&[outcome.as_label()])
        .inc();
}

/// Note une détection de bundle révoqué. **Devrait rester à 0
/// en steady-state** — incrémenter trigger PagerDuty critical
/// via l'alerte `BUNDLE_REVOKED_DETECTED`.
pub fn note_revoked(bundle_id: &str, version: &str) {
    bundle_revoked_total()
        .with_label_values(&[bundle_id, version])
        .inc();
}

// ====================================================================
// Eager registration
// ====================================================================

/// Force-register every bundle instrument with the global registry.
/// Idempotent thanks to `OnceLock`.
///
/// **Pattern Φ-C-A1** — `Server::build` MUST call this après
/// `register_all_p2()` pour que les panels Grafana du dashboard
/// bundle (= à venir) affichent 0 au boot plutôt que "no data".
pub fn register_all_bundle() {
    let _ = bundle_loaded_at_boot();
    let _ = bundle_fingerprint_info();
    let _ = bundle_prewarm_p99_seconds();
    let _ = bundle_recall_at_1();
    let _ = bundle_size_bytes();
    let _ = bundle_inference_duration_seconds();
    let _ = bundle_verify_total();
    let _ = bundle_revoked_total();
}

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus::core::Collector;
    use std::time::Duration;

    #[test]
    fn register_all_bundle_is_idempotent() {
        register_all_bundle();
        register_all_bundle();
        register_all_bundle();
    }

    #[test]
    fn all_8_instruments_have_expected_label_arity() {
        assert_eq!(
            bundle_loaded_at_boot().desc()[0].variable_labels.len(),
            0,
            "bundle_loaded_at_boot must have no labels"
        );
        assert_eq!(
            bundle_fingerprint_info().desc()[0].variable_labels,
            vec![
                "bundle_id".to_string(),
                "version".to_string(),
                "manifest_sha256".to_string()
            ]
        );
        assert_eq!(
            bundle_prewarm_p99_seconds().desc()[0].variable_labels,
            vec!["bundle_id".to_string(), "version".to_string()]
        );
        assert_eq!(
            bundle_recall_at_1().desc()[0].variable_labels,
            vec![
                "bundle_id".to_string(),
                "version".to_string(),
                "language".to_string()
            ]
        );
        assert_eq!(
            bundle_size_bytes().desc()[0].variable_labels,
            vec!["bundle_id".to_string(), "version".to_string()]
        );
        assert_eq!(
            bundle_inference_duration_seconds().desc()[0].variable_labels,
            vec![
                "bundle_id".to_string(),
                "provider".to_string(),
                "batch_size_class".to_string(),
                "seq_len_class".to_string()
            ]
        );
        assert_eq!(
            bundle_verify_total().desc()[0].variable_labels,
            vec!["outcome".to_string()]
        );
        assert_eq!(
            bundle_revoked_total().desc()[0].variable_labels,
            vec!["bundle_id".to_string(), "version".to_string()]
        );
    }

    #[test]
    fn slo_targets_fall_on_bucket_edges() {
        // Les 2 seuils contractuels (26 ms GPU, 180 ms CPU) doivent
        // tomber exactement sur un bord de bucket pour que
        // histogram_quantile(0.99, …) retourne LA cible, pas une
        // interpolation. Sanity-check chirurgical.
        let targets_s: &[f64] = &[0.026, 0.180];
        for t in targets_s {
            assert!(
                BUCKETS_INFERENCE_SECONDS.iter().any(|b| (b - t).abs() < 1e-9),
                "SLO target {t} s absente des buckets {:?}",
                BUCKETS_INFERENCE_SECONDS
            );
        }
    }

    #[test]
    fn buckets_strictly_increasing() {
        for w in BUCKETS_INFERENCE_SECONDS.windows(2) {
            assert!(w[0] < w[1], "non-strict at {:?}", w);
        }
    }

    #[test]
    fn batch_size_class_buckets() {
        assert_eq!(batch_size_class(0), "1");
        assert_eq!(batch_size_class(1), "1");
        assert_eq!(batch_size_class(2), "2-4");
        assert_eq!(batch_size_class(4), "2-4");
        assert_eq!(batch_size_class(5), "5-8");
        assert_eq!(batch_size_class(8), "5-8");
        assert_eq!(batch_size_class(16), "9-16");
        assert_eq!(batch_size_class(32), "17-32");
        assert_eq!(batch_size_class(1024), "17-32"); // saturation
    }

    #[test]
    fn seq_len_class_buckets() {
        assert_eq!(seq_len_class(0), "16");
        assert_eq!(seq_len_class(16), "16");
        assert_eq!(seq_len_class(17), "64");
        assert_eq!(seq_len_class(64), "64");
        assert_eq!(seq_len_class(128), "128");
        assert_eq!(seq_len_class(256), "256");
        assert_eq!(seq_len_class(512), "512");
        assert_eq!(seq_len_class(99999), "512"); // saturation
    }

    #[test]
    fn verify_outcome_count_and_labels_canonical() {
        // 6 outcomes canoniques (pin contractuel).
        assert_eq!(VerifyOutcome::all().len(), 6);
        let mut labels: Vec<&str> =
            VerifyOutcome::all().iter().map(|o| o.as_label()).collect();
        labels.sort();
        labels.dedup();
        assert_eq!(labels.len(), 6, "outcomes must have distinct labels");
    }

    #[test]
    fn verify_outcome_pass_is_only_non_fatal() {
        assert!(!VerifyOutcome::Pass.is_fatal());
        for o in VerifyOutcome::all() {
            if !matches!(o, VerifyOutcome::Pass) {
                assert!(o.is_fatal(), "{:?} should be fatal", o);
            }
        }
    }

    #[test]
    fn observe_inference_does_not_panic_on_extreme_inputs() {
        register_all_bundle();
        // Extrême batch + seq_len
        observe_inference(
            "test",
            InferenceProvider::Cuda,
            64,        // > 32, satured to "17-32"
            1024,      // > 512, satured to "512"
            Duration::from_secs(5), // way over biggest bucket → +Inf
        );
        // Zero-time
        observe_inference("test", InferenceProvider::Cpu, 0, 0, Duration::ZERO);
    }

    #[test]
    fn note_bundle_loaded_sets_all_gauges() {
        register_all_bundle();
        note_bundle_loaded(
            "paraphrase-multilingual-MiniLM-L12-v2",
            "1.0.0",
            "abc123",
            78_458_123,
            Duration::from_millis(42),
        );
        assert_eq!(bundle_loaded_at_boot().get(), 1);
        let size = bundle_size_bytes()
            .with_label_values(&["paraphrase-multilingual-MiniLM-L12-v2", "1.0.0"])
            .get();
        assert_eq!(size, 78_458_123);
    }

    #[test]
    fn note_verify_increments_counter() {
        register_all_bundle();
        let before = bundle_verify_total()
            .with_label_values(&["pass"])
            .get();
        note_verify(VerifyOutcome::Pass);
        assert_eq!(
            bundle_verify_total().with_label_values(&["pass"]).get(),
            before + 1
        );
        note_verify(VerifyOutcome::HashMismatch);
        assert_eq!(
            bundle_verify_total()
                .with_label_values(&["hash_mismatch"])
                .get(),
            1
        );
    }

    #[test]
    fn note_revoked_increments_counter() {
        register_all_bundle();
        note_revoked("test-bundle", "1.0.0");
        note_revoked("test-bundle", "1.0.0");
        assert_eq!(
            bundle_revoked_total()
                .with_label_values(&["test-bundle", "1.0.0"])
                .get(),
            2
        );
    }

    #[test]
    fn inference_provider_label_distinct() {
        assert_eq!(InferenceProvider::Cpu.as_label(), "cpu");
        assert_eq!(InferenceProvider::Cuda.as_label(), "cuda");
        assert_ne!(
            InferenceProvider::Cpu.as_label(),
            InferenceProvider::Cuda.as_label()
        );
    }

    /// **Audit fix Φ-C.A18** — test pour `note_recall_at_1`,
    /// manquant dans la livraison initiale (coverage gap).
    #[test]
    fn note_recall_at_1_sets_gauge_per_language() {
        register_all_bundle();
        note_recall_at_1("test-bundle", "1.0.0", "fr", 0.918);
        note_recall_at_1("test-bundle", "1.0.0", "en", 0.934);
        note_recall_at_1("test-bundle", "1.0.0", "de", 0.921);

        let fr = bundle_recall_at_1()
            .with_label_values(&["test-bundle", "1.0.0", "fr"])
            .get();
        let en = bundle_recall_at_1()
            .with_label_values(&["test-bundle", "1.0.0", "en"])
            .get();
        let de = bundle_recall_at_1()
            .with_label_values(&["test-bundle", "1.0.0", "de"])
            .get();
        assert!((fr - 0.918).abs() < 1e-6, "fr recall = {fr}");
        assert!((en - 0.934).abs() < 1e-6, "en recall = {en}");
        assert!((de - 0.921).abs() < 1e-6, "de recall = {de}");

        // Idempotence : ré-écrire la même valeur ne change rien.
        note_recall_at_1("test-bundle", "1.0.0", "fr", 0.918);
        let fr2 = bundle_recall_at_1()
            .with_label_values(&["test-bundle", "1.0.0", "fr"])
            .get();
        assert_eq!(fr, fr2);
    }
}
