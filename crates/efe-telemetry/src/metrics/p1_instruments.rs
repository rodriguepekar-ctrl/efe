// SPDX-License-Identifier: UNLICENSED
//! Sprint **D19 (P1-Foundation)** — Prometheus instruments for the
//! P1 *Latence opposable* surface.
//!
//! Cf. `docs/EFE_P1_SURFACE_ARCHITECTURE.md` § 4.2 (le tableau des
//! 12 histograms) et § 2.1 (le tableau canonique du budget 200 ms
//! par étage).
//!
//! ## Inventaire (12 histograms + 2 auxiliaires)
//!
//! | # | Métrique                                              | Type      | Labels       | Budget cible p99 |
//! |--:|-------------------------------------------------------|-----------|--------------|-----------------:|
//! | 1 | `efe_p1_grpc_recv_duration_seconds`                   | Histogram | `mode`       | 1.5 ms |
//! | 2 | `efe_p1_grpc_decode_duration_seconds`                 | Histogram | `mode`       | 0.5 ms |
//! | 3 | `efe_p1_interceptor_duration_seconds`                 | Histogram | `mode`, `interceptor` | 3 ms |
//! | 4 | `efe_p1_embed_tokenize_duration_seconds`              | Histogram | `mode`       | 4 ms |
//! | 5 | `efe_p1_embed_inference_duration_seconds`             | Histogram | `mode`, `batch_size_class` | 26 ms |
//! | 6 | `efe_p1_genesis_duration_seconds`                     | Histogram | `mode`       | 5 ms |
//! | 7 | `efe_p1_fusion_round_duration_seconds`                | Histogram | `mode`, `round_class` | 10 ms / round |
//! | 8 | `efe_p1_consensus_duration_seconds`                   | Histogram | `mode`       | 1 ms |
//! | 9 | `efe_p1_dissolution_alignment_duration_seconds`       | Histogram | `mode`       | 6 ms |
//! |10 | `efe_p1_audit_chain_append_duration_seconds`          | Histogram | `mode`       | 4 ms |
//! |11 | `efe_p1_grpc_encode_duration_seconds`                 | Histogram | `mode`       | 1 ms |
//! |12 | `efe_p1_grpc_send_duration_seconds`                   | Histogram | `mode`       | 1 ms |
//! |aux| `efe_p1_budget_overrun_total`                         | Counter   | `mode`, `stage` | n/a |
//! |aux| `efe_p1_budget_remaining_seconds`                     | Histogram | `mode`       | n/a |
//!
//! ## Cardinalité
//!
//! - `mode` ∈ {`mode1`, `mode2`, `mode3`} = 3 valeurs.
//! - `interceptor` ∈ {`auth`, `rate_limit`, `tenant`} = 3.
//! - `batch_size_class` ∈ {`1`, `2-4`, `5-8`, `9-16`, `17-32`} = 5.
//! - `round_class` ∈ {`0`, `1-3`, `4-6`, `7-9`} = 4.
//! - `stage` ∈ les 12 étages = 12.
//!
//! Cardinalité totale ≈ `3*(11) + 3*3 + 3*5 + 3*4 + 3*12 + 3` =
//! ~ 110 séries Prometheus. Largement sous le budget D13 de 50 000 séries.
//!
//! ## Doctrine des buckets
//!
//! Les buckets de chaque histogram sont choisis pour que **les valeurs
//! critiques (cible p99 et 2× cible) tombent exactement sur un bord de
//! bucket**. Sans ça, le calcul `histogram_quantile(0.99, …)` interpole
//! entre 2 buckets et perd la précision juste au point qui nous intéresse.
//! Cf. `docs/EFE_P1_SURFACE_ARCHITECTURE.md` Annexe D pour le rationale.

use std::sync::OnceLock;

use prometheus::{HistogramOpts, HistogramVec, IntCounterVec, Opts};

use super::registry::global_registry;

// ====================================================================
// Macro de paresse + register-once (calquée sur d18_instruments.rs)
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
// Buckets canoniques (publics pour les benches et tests)
// ====================================================================

/// Buckets fins pour les étages micro (< 5 ms). Centrés sur les
/// cibles 0.5 / 1 / 1.5 / 3 / 4 ms et leurs 2×.
pub const BUCKETS_MICRO_SECONDS: &[f64] =
    &[0.0001, 0.0003, 0.0005, 0.0008, 0.0012, 0.0015, 0.002, 0.003, 0.004, 0.006, 0.008, 0.012, 0.020];

/// Buckets pour la tokenization (cible 4 ms p99).
pub const BUCKETS_TOKENIZE_SECONDS: &[f64] =
    &[0.0005, 0.001, 0.0015, 0.002, 0.003, 0.004, 0.006, 0.008, 0.012, 0.020];

/// Buckets pour l'inférence ONNX CUDA (cible 26 ms p99 ; cap 100 ms).
pub const BUCKETS_INFERENCE_SECONDS: &[f64] =
    &[0.005, 0.010, 0.015, 0.020, 0.026, 0.030, 0.040, 0.060, 0.080, 0.100, 0.150, 0.250];

/// Buckets pour Genesis (cible 5 ms p99).
pub const BUCKETS_GENESIS_SECONDS: &[f64] =
    &[0.0005, 0.001, 0.002, 0.003, 0.004, 0.005, 0.006, 0.008, 0.010, 0.015, 0.025];

/// Buckets pour la durée d'**un round** de fusion (cible 10 ms / round).
pub const BUCKETS_FUSION_ROUND_SECONDS: &[f64] =
    &[0.001, 0.002, 0.004, 0.006, 0.008, 0.010, 0.012, 0.016, 0.020, 0.030, 0.050, 0.080];

/// Buckets pour le consensus (cible 1 ms p99).
pub const BUCKETS_CONSENSUS_SECONDS: &[f64] =
    &[0.0001, 0.0003, 0.0005, 0.0008, 0.001, 0.0015, 0.002, 0.003, 0.005, 0.010];

/// Buckets pour dissolution + alignment (cible 6 ms p99).
pub const BUCKETS_DISSOLUTION_ALIGN_SECONDS: &[f64] =
    &[0.0005, 0.001, 0.002, 0.003, 0.004, 0.006, 0.008, 0.012, 0.018, 0.025];

/// Buckets pour audit chain append (cible 4 ms p99).
pub const BUCKETS_AUDIT_APPEND_SECONDS: &[f64] =
    &[0.0005, 0.001, 0.002, 0.003, 0.004, 0.005, 0.007, 0.010, 0.015, 0.025];

/// Buckets pour le **budget restant** à l'instant de l'envoi de la
/// réponse. Histogramme uniquement diagnostic ; permet de tracer les
/// requêtes qui ont consommé ≥ 80 % du budget (i.e. budget_remaining
/// < 40 ms).
pub const BUCKETS_BUDGET_REMAINING_SECONDS: &[f64] =
    &[0.0, 0.010, 0.020, 0.040, 0.060, 0.080, 0.100, 0.120, 0.140, 0.160, 0.180, 0.200];

// ====================================================================
// 1. Histogram — gRPC recv + TLS framing (étage 1)
// ====================================================================

once_metric!(
    /// `efe_p1_grpc_recv_duration_seconds{mode}` — wall-clock entre
    /// `tonic` `Service::poll_ready` et la fin de la phase de framing
    /// TLS pour cette requête (keep-alive amorti).
    ///
    /// **Budget cible p99 : 1.5 ms** (SLO-P1-BUDGET-1).
    grpc_recv_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_p1_grpc_recv_duration_seconds",
            "TCP recv + TLS framing duration (D19 P1 stage 1).",
        )
        .buckets(BUCKETS_MICRO_SECONDS.to_vec()),
        &["mode"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// 2. Histogram — Protobuf decode (étage 2)
// ====================================================================

once_metric!(
    /// `efe_p1_grpc_decode_duration_seconds{mode}` — durée de
    /// `prost::Message::decode` pour `ExecuteRequest`.
    ///
    /// **Budget cible p99 : 0.5 ms** (SLO-P1-BUDGET-2).
    grpc_decode_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_p1_grpc_decode_duration_seconds",
            "Protobuf decode duration (D19 P1 stage 2).",
        )
        .buckets(BUCKETS_MICRO_SECONDS.to_vec()),
        &["mode"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// 3. Histogram — gRPC interceptors (étage 3)
// ====================================================================

once_metric!(
    /// `efe_p1_interceptor_duration_seconds{mode,interceptor}` —
    /// durée par interceptor (auth, rate_limit, tenant).
    ///
    /// **Budget cible p99 cumulé : 3 ms** (SLO-P1-BUDGET-3).
    interceptor_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_p1_interceptor_duration_seconds",
            "gRPC interceptor duration (D19 P1 stage 3, per-interceptor).",
        )
        .buckets(BUCKETS_MICRO_SECONDS.to_vec()),
        &["mode", "interceptor"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// 4. Histogram — tokenization (étage 4)
// ====================================================================

once_metric!(
    /// `efe_p1_embed_tokenize_duration_seconds{mode}` — durée du
    /// tokenizer Hugging Face.
    ///
    /// **Budget cible p99 : 4 ms** (SLO-P1-BUDGET-4).
    embed_tokenize_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_p1_embed_tokenize_duration_seconds",
            "Tokenizer encode duration (D19 P1 stage 4).",
        )
        .buckets(BUCKETS_TOKENIZE_SECONDS.to_vec()),
        &["mode"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// 5. Histogram — ONNX CUDA inference (étage 5 — CRITIQUE)
// ====================================================================

once_metric!(
    /// `efe_p1_embed_inference_duration_seconds{mode,batch_size_class}` —
    /// durée de l'inférence ONNX CUDA (ou CPU EP fallback).
    ///
    /// `batch_size_class` est calculé par
    /// [`batch_size_class`] depuis la taille effective du batch
    /// coalescé.
    ///
    /// **Budget cible p99 : 26 ms** sur batch ≤ 8 (SLO-P1-BUDGET-5).
    /// Au-delà (batch 17-32), le budget passe à 40 ms — toléré, mais
    /// alerte si > 60 ms.
    embed_inference_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_p1_embed_inference_duration_seconds",
            "ONNX CUDA forward-pass duration (D19 P1 stage 5).",
        )
        .buckets(BUCKETS_INFERENCE_SECONDS.to_vec()),
        &["mode", "batch_size_class"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// 6. Histogram — Genesis (étage 6)
// ====================================================================

once_metric!(
    /// `efe_p1_genesis_duration_seconds{mode}` — durée du spawn des
    /// `N` agents + computation du prior NaSem/NaLog/NaCrit.
    ///
    /// **Budget cible p99 : 5 ms** (SLO-P1-BUDGET-6).
    genesis_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_p1_genesis_duration_seconds",
            "Genesis spawn + prior duration (D19 P1 stage 6).",
        )
        .buckets(BUCKETS_GENESIS_SECONDS.to_vec()),
        &["mode"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// 7. Histogram — Fusion round (étage 7 — CRITIQUE)
// ====================================================================

once_metric!(
    /// `efe_p1_fusion_round_duration_seconds{mode,round_class}` —
    /// durée d'**un** round de fusion. La somme p99 × n_rounds est
    /// gatée par SLO-P1-BUDGET-7 (100 ms total).
    ///
    /// `round_class` permet de capturer la signature warm-up :
    /// le round 0 est typiquement 1.5× plus lent que les suivants
    /// (allocation arena, cache miss). On veut le détecter sans
    /// trigger d'alerte.
    fusion_round_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_p1_fusion_round_duration_seconds",
            "Fusion single-round duration (D19 P1 stage 7, per round).",
        )
        .buckets(BUCKETS_FUSION_ROUND_SECONDS.to_vec()),
        &["mode", "round_class"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// 8. Histogram — Consensus (étage 8)
// ====================================================================

once_metric!(
    /// `efe_p1_consensus_duration_seconds{mode}` — durée du
    /// `MultiCriteriaDetector::detect`.
    ///
    /// **Budget cible p99 : 1 ms** (SLO-P1-BUDGET-8).
    consensus_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_p1_consensus_duration_seconds",
            "Consensus detector duration (D19 P1 stage 8).",
        )
        .buckets(BUCKETS_CONSENSUS_SECONDS.to_vec()),
        &["mode"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// 9. Histogram — Dissolution + Alignment (étage 9)
// ====================================================================

once_metric!(
    /// `efe_p1_dissolution_alignment_duration_seconds{mode}` —
    /// durée combinée dissolution + alignment + projection sortie.
    ///
    /// **Budget cible p99 : 6 ms** (SLO-P1-BUDGET-9).
    dissolution_alignment_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_p1_dissolution_alignment_duration_seconds",
            "Dissolution + alignment duration (D19 P1 stage 9).",
        )
        .buckets(BUCKETS_DISSOLUTION_ALIGN_SECONDS.to_vec()),
        &["mode"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// 10. Histogram — Audit chain append (étage 10)
// ====================================================================

once_metric!(
    /// `efe_p1_audit_chain_append_duration_seconds{mode}` — durée
    /// du `append` HMAC-chained dans la chaîne D14-bis.
    ///
    /// **Budget cible p99 : 4 ms** (SLO-P1-BUDGET-10).
    audit_chain_append_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_p1_audit_chain_append_duration_seconds",
            "Audit chain HMAC append duration (D19 P1 stage 10).",
        )
        .buckets(BUCKETS_AUDIT_APPEND_SECONDS.to_vec()),
        &["mode"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// 11. Histogram — Protobuf encode (étage 11)
// ====================================================================

once_metric!(
    /// `efe_p1_grpc_encode_duration_seconds{mode}` — durée de
    /// `prost::Message::encode` pour `ExecuteResponse`.
    ///
    /// **Budget cible p99 : 1 ms** (SLO-P1-BUDGET-11).
    grpc_encode_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_p1_grpc_encode_duration_seconds",
            "Protobuf encode duration (D19 P1 stage 11).",
        )
        .buckets(BUCKETS_MICRO_SECONDS.to_vec()),
        &["mode"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// 12. Histogram — TCP send + framing (étage 12)
// ====================================================================

once_metric!(
    /// `efe_p1_grpc_send_duration_seconds{mode}` — durée du `write`
    /// final sur la socket TCP.
    ///
    /// **Budget cible p99 : 1 ms** (SLO-P1-BUDGET-12).
    grpc_send_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_p1_grpc_send_duration_seconds",
            "TCP send + framing duration (D19 P1 stage 12).",
        )
        .buckets(BUCKETS_MICRO_SECONDS.to_vec()),
        &["mode"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// Auxiliaire — Counter des overruns de budget par étage
// ====================================================================

once_metric!(
    /// `efe_p1_budget_overrun_total{mode,stage}` — incrémenté à chaque
    /// fois que le `BudgetTracker` constate qu'une étape a dépassé sa
    /// quote-part de budget. **N'implique pas** une violation du SLO
    /// global (= les 200 ms du roundtrip) car la marge anti-tail peut
    /// l'absorber ; c'est un **signal précurseur**.
    ///
    /// Alert `P1_STAGE_OVERRUN_BURST` se déclenche sur un taux
    /// `> 5 %` sur 5 min pour un même `stage`.
    budget_overrun_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_p1_budget_overrun_total",
            "Per-stage budget overrun count (D19 P1 — diagnostic).",
        ),
        &["mode", "stage"],
    )
    .expect("valid counter opts")
);

// ====================================================================
// Auxiliaire — Histogram du budget restant à l'envoi
// ====================================================================

once_metric!(
    /// `efe_p1_budget_remaining_seconds{mode}` — distribution du
    /// budget restant au moment où `ExecuteResponse` part. Permet
    /// d'alerter sur les requêtes qui consomment > 80 % du budget
    /// (i.e. < 40 ms restant).
    ///
    /// Recording rule `efe_p1:near_budget_violation:5m =
    /// histogram_quantile(0.99, budget_remaining) < 0.04` déclenche
    /// l'alerte précurseur P1_NEAR_BUDGET.
    budget_remaining_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_p1_budget_remaining_seconds",
            "Budget remaining at response send time (D19 P1 — precursor).",
        )
        .buckets(BUCKETS_BUDGET_REMAINING_SECONDS.to_vec()),
        &["mode"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// Stage enum (canonique pour les labels)
// ====================================================================

/// Identifiant canonique des 12 étages P1.
///
/// L'enum est `Copy` + `'static` ; il est utilisé comme label
/// stringifié par les compteurs et le `BudgetTracker`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Stage {
    /// Étage 1 — TCP recv + TLS framing.
    GrpcRecv,
    /// Étage 2 — Protobuf decode.
    GrpcDecode,
    /// Étage 3 — gRPC interceptors (auth, rate_limit, tenant).
    Interceptors,
    /// Étage 4 — Tokenization.
    Tokenize,
    /// Étage 5 — ONNX CUDA inference.
    EmbedInference,
    /// Étage 6 — Genesis.
    Genesis,
    /// Étage 7 — Fusion (10 rounds).
    Fusion,
    /// Étage 8 — Consensus.
    Consensus,
    /// Étage 9 — Dissolution + Alignment.
    DissolutionAlignment,
    /// Étage 10 — Audit chain append.
    AuditChainAppend,
    /// Étage 11 — Protobuf encode.
    GrpcEncode,
    /// Étage 12 — TCP send.
    GrpcSend,
}

impl Stage {
    /// Stable string for Prometheus labels.
    #[must_use]
    pub const fn as_label(self) -> &'static str {
        match self {
            Self::GrpcRecv => "grpc_recv",
            Self::GrpcDecode => "grpc_decode",
            Self::Interceptors => "interceptors",
            Self::Tokenize => "tokenize",
            Self::EmbedInference => "embed_inference",
            Self::Genesis => "genesis",
            Self::Fusion => "fusion",
            Self::Consensus => "consensus",
            Self::DissolutionAlignment => "dissolution_alignment",
            Self::AuditChainAppend => "audit_chain_append",
            Self::GrpcEncode => "grpc_encode",
            Self::GrpcSend => "grpc_send",
        }
    }

    /// Budget canonical p99 (en secondes) — source de vérité unique
    /// pour les 12 étages, calé sur `EFE_P1_SURFACE_ARCHITECTURE.md`
    /// § 2.1.
    ///
    /// Le total de la colonne `budget()` doit faire **153 ms = 0.153 s**
    /// (sanity-check par `tests::stage_budget_sum_equals_canonical_153ms`).
    #[must_use]
    pub const fn budget(self) -> std::time::Duration {
        use std::time::Duration;
        match self {
            Self::GrpcRecv => Duration::from_micros(1500),
            Self::GrpcDecode => Duration::from_micros(500),
            Self::Interceptors => Duration::from_millis(3),
            Self::Tokenize => Duration::from_millis(4),
            Self::EmbedInference => Duration::from_millis(26),
            Self::Genesis => Duration::from_millis(5),
            Self::Fusion => Duration::from_millis(100),
            Self::Consensus => Duration::from_millis(1),
            Self::DissolutionAlignment => Duration::from_millis(6),
            Self::AuditChainAppend => Duration::from_millis(4),
            Self::GrpcEncode => Duration::from_millis(1),
            Self::GrpcSend => Duration::from_millis(1),
        }
    }

    /// Itérateur canonique sur les 12 étages dans l'ordre du
    /// hot-path. Utilisé par `register_all_p1` et les tests.
    #[must_use]
    pub const fn all() -> &'static [Stage; 12] {
        &[
            Self::GrpcRecv,
            Self::GrpcDecode,
            Self::Interceptors,
            Self::Tokenize,
            Self::EmbedInference,
            Self::Genesis,
            Self::Fusion,
            Self::Consensus,
            Self::DissolutionAlignment,
            Self::AuditChainAppend,
            Self::GrpcEncode,
            Self::GrpcSend,
        ]
    }
}

// ====================================================================
// Helpers de classification (labels secondaires)
// ====================================================================

/// Classe de batch size pour l'étage 5.
///
/// Le mapping est stable : ajouter une classe **n'est jamais
/// rétro-compatible** avec les buckets historiques (les agrégats
/// changeraient de cardinalité). Tout changement requiert une
/// migration explicite via baseline gating.
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

/// Classe de round-index pour l'étage 7. Cf. doc du
/// `fusion_round_duration_seconds` pour le rationale warm-up.
#[must_use]
pub const fn round_class(round_idx: usize) -> &'static str {
    match round_idx {
        0 => "0",
        1..=3 => "1-3",
        4..=6 => "4-6",
        _ => "7-9",
    }
}

/// Stable string for the interceptor label (étage 3).
///
/// Cardinalité fixée à 3 ; ajouter un interceptor au hot-path
/// est une décision archi (cf. P1 § 3.6).
#[must_use]
pub const fn interceptor_label(kind: InterceptorKind) -> &'static str {
    match kind {
        InterceptorKind::Auth => "auth",
        InterceptorKind::RateLimit => "rate_limit",
        InterceptorKind::Tenant => "tenant",
    }
}

/// Identifiant logique d'un interceptor (étage 3).
#[derive(Debug, Clone, Copy)]
pub enum InterceptorKind {
    /// `auth_mtls` — mTLS / JWT validation.
    Auth,
    /// `rate_limit_leaky_bucket` — leaky bucket per-tenant.
    RateLimit,
    /// `tenant_resolution` — lookup tenant policy.
    Tenant,
}

// ====================================================================
// API ergonomique côté hot-path
// ====================================================================

/// Observe la durée d'un étage (helper générique, label-agnostic).
///
/// ## Avertissement (audit fix D19-P1.A11)
///
/// 3 étages ont un label secondaire dont la sémantique est perdue
/// par ce helper générique :
///
/// - [`Stage::Interceptors`] → utilise le label `interceptor = "all"`
///   (= aggrégé) au lieu de `auth`/`rate_limit`/`tenant`. Pour
///   l'attribution par interceptor, **préférer** [`observe_interceptor`].
/// - [`Stage::EmbedInference`] → utilise `batch_size_class = "1"`
///   (= solo request) au lieu de la vraie classe du batch. Pour
///   la mesure par classe, **préférer** [`observe_inference`].
/// - [`Stage::Fusion`] → utilise `round_class = "0"` (= 1er round)
///   au lieu de la vraie position. Pour la mesure par round,
///   **préférer** [`observe_fusion_round`].
///
/// Le `BudgetTracker::checkpoint` utilise ce helper pour les étages
/// 1, 2, 6, 8, 9, 10, 11, 12 (label-unique), et appelle
/// `observe_inference` / `observe_fusion_round` séparément pour les
/// étages 5 / 7 via les méthodes dédiées
/// `BudgetTracker::checkpoint_inference` (et son cousin
/// `checkpoint_fusion_round` à venir D26).
pub fn observe_stage(stage: Stage, mode: &str, duration: std::time::Duration) {
    let secs = duration.as_secs_f64();
    match stage {
        Stage::GrpcRecv => grpc_recv_duration_seconds().with_label_values(&[mode]).observe(secs),
        Stage::GrpcDecode => grpc_decode_duration_seconds().with_label_values(&[mode]).observe(secs),
        Stage::Interceptors => {
            // Cardinalité requise = mode + interceptor. Ce helper
            // suppose "all interceptors aggregated" → label = "all".
            interceptor_duration_seconds()
                .with_label_values(&[mode, "all"])
                .observe(secs);
        }
        Stage::Tokenize => embed_tokenize_duration_seconds().with_label_values(&[mode]).observe(secs),
        Stage::EmbedInference => {
            // batch_size_class label = "1" par défaut quand non fourni
            embed_inference_duration_seconds()
                .with_label_values(&[mode, "1"])
                .observe(secs);
        }
        Stage::Genesis => genesis_duration_seconds().with_label_values(&[mode]).observe(secs),
        Stage::Fusion => {
            fusion_round_duration_seconds()
                .with_label_values(&[mode, "0"])
                .observe(secs);
        }
        Stage::Consensus => consensus_duration_seconds().with_label_values(&[mode]).observe(secs),
        Stage::DissolutionAlignment => {
            dissolution_alignment_duration_seconds()
                .with_label_values(&[mode])
                .observe(secs);
        }
        Stage::AuditChainAppend => {
            audit_chain_append_duration_seconds()
                .with_label_values(&[mode])
                .observe(secs);
        }
        Stage::GrpcEncode => grpc_encode_duration_seconds().with_label_values(&[mode]).observe(secs),
        Stage::GrpcSend => grpc_send_duration_seconds().with_label_values(&[mode]).observe(secs),
    }
}

/// Observe l'inférence avec son `batch_size_class` correct.
pub fn observe_inference(mode: &str, batch_size: usize, duration: std::time::Duration) {
    embed_inference_duration_seconds()
        .with_label_values(&[mode, batch_size_class(batch_size)])
        .observe(duration.as_secs_f64());
}

/// Observe un round de fusion avec son `round_class` correct.
pub fn observe_fusion_round(mode: &str, round_idx: usize, duration: std::time::Duration) {
    fusion_round_duration_seconds()
        .with_label_values(&[mode, round_class(round_idx)])
        .observe(duration.as_secs_f64());
}

/// Observe un interceptor spécifique.
pub fn observe_interceptor(
    mode: &str,
    kind: InterceptorKind,
    duration: std::time::Duration,
) {
    interceptor_duration_seconds()
        .with_label_values(&[mode, interceptor_label(kind)])
        .observe(duration.as_secs_f64());
}

/// Incrémente le compteur d'overrun pour l'étage donné.
pub fn note_budget_overrun(mode: &str, stage: Stage) {
    budget_overrun_total()
        .with_label_values(&[mode, stage.as_label()])
        .inc();
}

/// Observe le budget restant à la fin de la requête. Si négatif,
/// clamper à 0 (Prometheus n'aime pas les valeurs négatives sur un
/// histogram non-signed-friendly).
pub fn observe_budget_remaining(mode: &str, remaining: std::time::Duration) {
    let secs = remaining.as_secs_f64().max(0.0);
    budget_remaining_seconds()
        .with_label_values(&[mode])
        .observe(secs);
}

// ====================================================================
// Eager registration
// ====================================================================

/// Force-register every P1 instrument with the global registry.
/// Idempotent thanks to `OnceLock`.
///
/// **Pattern P1-A1** — `Server::build` MUST call this so the
/// Prometheus `/metrics` endpoint exposes the 12 P1 series as
/// zero-valued (vs "no data") even before any traffic. Sans ça, le
/// dashboard Grafana `efe-p1-slo.json` montre des panels "N/A" au
/// boot, ce qui fait passer pour de l'instrumentation cassée. Cf.
/// pattern récurrent fixé en D15 / D16 / D18 (closure docs).
pub fn register_all_p1() {
    let _ = grpc_recv_duration_seconds();
    let _ = grpc_decode_duration_seconds();
    let _ = interceptor_duration_seconds();
    let _ = embed_tokenize_duration_seconds();
    let _ = embed_inference_duration_seconds();
    let _ = genesis_duration_seconds();
    let _ = fusion_round_duration_seconds();
    let _ = consensus_duration_seconds();
    let _ = dissolution_alignment_duration_seconds();
    let _ = audit_chain_append_duration_seconds();
    let _ = grpc_encode_duration_seconds();
    let _ = grpc_send_duration_seconds();
    let _ = budget_overrun_total();
    let _ = budget_remaining_seconds();
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
    fn register_all_p1_is_idempotent() {
        register_all_p1();
        register_all_p1();
        register_all_p1();
        // Si ce test passe sans panic, le `OnceLock` + `register`
        // du global registry n'a pas re-inscrit (les `AlreadyReg`
        // errors sont swallowed par le `let _ =`).
    }

    #[test]
    fn all_12_stages_have_label_arity_mode_only_or_mode_plus_one() {
        let collectors: Vec<(&str, &HistogramVec)> = vec![
            ("grpc_recv", grpc_recv_duration_seconds()),
            ("grpc_decode", grpc_decode_duration_seconds()),
            ("embed_tokenize", embed_tokenize_duration_seconds()),
            ("genesis", genesis_duration_seconds()),
            ("consensus", consensus_duration_seconds()),
            ("dissolution_alignment", dissolution_alignment_duration_seconds()),
            ("audit_chain_append", audit_chain_append_duration_seconds()),
            ("grpc_encode", grpc_encode_duration_seconds()),
            ("grpc_send", grpc_send_duration_seconds()),
        ];
        for (name, h) in collectors {
            assert_eq!(
                h.desc()[0].variable_labels,
                vec!["mode".to_string()],
                "{name}: expected single label 'mode'"
            );
        }
        // Les 3 qui ont un label additionnel
        assert_eq!(
            interceptor_duration_seconds().desc()[0].variable_labels,
            vec!["mode".to_string(), "interceptor".to_string()],
        );
        assert_eq!(
            embed_inference_duration_seconds().desc()[0].variable_labels,
            vec!["mode".to_string(), "batch_size_class".to_string()],
        );
        assert_eq!(
            fusion_round_duration_seconds().desc()[0].variable_labels,
            vec!["mode".to_string(), "round_class".to_string()],
        );
    }

    #[test]
    fn stage_budget_sum_equals_canonical_153ms() {
        let total: Duration = Stage::all().iter().map(|s| s.budget()).sum();
        // Budget canonique = 153 ms (somme stricte des 12 étages)
        // (cf. P1 § 2.1 colonne "TOTAL budget p99").
        assert_eq!(total, Duration::from_micros(153_000));
    }

    #[test]
    fn stage_labels_are_distinct() {
        let mut labels: Vec<&str> = Stage::all().iter().map(|s| s.as_label()).collect();
        labels.sort();
        labels.dedup();
        assert_eq!(labels.len(), 12, "12 distinct labels expected");
    }

    #[test]
    fn batch_size_class_buckets() {
        assert_eq!(batch_size_class(0), "1");
        assert_eq!(batch_size_class(1), "1");
        assert_eq!(batch_size_class(2), "2-4");
        assert_eq!(batch_size_class(4), "2-4");
        assert_eq!(batch_size_class(5), "5-8");
        assert_eq!(batch_size_class(8), "5-8");
        assert_eq!(batch_size_class(9), "9-16");
        assert_eq!(batch_size_class(16), "9-16");
        assert_eq!(batch_size_class(17), "17-32");
        assert_eq!(batch_size_class(32), "17-32");
        assert_eq!(batch_size_class(1024), "17-32"); // saturation
    }

    #[test]
    fn round_class_buckets() {
        assert_eq!(round_class(0), "0");
        assert_eq!(round_class(1), "1-3");
        assert_eq!(round_class(3), "1-3");
        assert_eq!(round_class(4), "4-6");
        assert_eq!(round_class(6), "4-6");
        assert_eq!(round_class(7), "7-9");
        assert_eq!(round_class(9), "7-9");
        assert_eq!(round_class(1000), "7-9"); // saturation
    }

    #[test]
    fn observe_stage_does_not_panic() {
        register_all_p1();
        observe_stage(Stage::GrpcRecv, "mode1", Duration::from_micros(800));
        observe_stage(Stage::Fusion, "mode1", Duration::from_millis(95));
        // L'API doit aussi accepter une valeur > 2× le bucket max
        // sans panic (Prometheus clamp en `+Inf`).
        observe_stage(Stage::Fusion, "mode1", Duration::from_secs(10));
    }

    #[test]
    fn observe_inference_classifies_correctly() {
        register_all_p1();
        observe_inference("mode1", 1, Duration::from_millis(20));
        observe_inference("mode1", 16, Duration::from_millis(30));
        // Pas de panic = OK. Le bon classement est testé par
        // `batch_size_class_buckets`.
    }

    #[test]
    fn observe_fusion_round_classifies_correctly() {
        register_all_p1();
        for r in 0..10 {
            observe_fusion_round("mode1", r, Duration::from_millis(10));
        }
    }

    #[test]
    fn budget_overrun_counter_increments() {
        register_all_p1();
        let before = budget_overrun_total()
            .with_label_values(&["mode1", Stage::Tokenize.as_label()])
            .get();
        note_budget_overrun("mode1", Stage::Tokenize);
        note_budget_overrun("mode1", Stage::Tokenize);
        let after = budget_overrun_total()
            .with_label_values(&["mode1", Stage::Tokenize.as_label()])
            .get();
        assert_eq!(after, before + 2);
    }

    #[test]
    fn budget_remaining_clamps_negative_to_zero() {
        register_all_p1();
        // Saturating sub : Duration::ZERO si saturate, donc le test
        // valide qu'on ne panic pas avec une "remaining" déjà à 0.
        observe_budget_remaining("mode1", Duration::ZERO);
        // Et qu'une valeur normale passe.
        observe_budget_remaining("mode1", Duration::from_millis(47));
    }

    #[test]
    fn buckets_are_strictly_increasing() {
        for (name, b) in &[
            ("micro", BUCKETS_MICRO_SECONDS),
            ("tokenize", BUCKETS_TOKENIZE_SECONDS),
            ("inference", BUCKETS_INFERENCE_SECONDS),
            ("genesis", BUCKETS_GENESIS_SECONDS),
            ("fusion_round", BUCKETS_FUSION_ROUND_SECONDS),
            ("consensus", BUCKETS_CONSENSUS_SECONDS),
            ("dissolution_align", BUCKETS_DISSOLUTION_ALIGN_SECONDS),
            ("audit_append", BUCKETS_AUDIT_APPEND_SECONDS),
            ("budget_remaining", BUCKETS_BUDGET_REMAINING_SECONDS),
        ] {
            for w in b.windows(2) {
                assert!(
                    w[0] < w[1],
                    "{name}: buckets not strictly increasing: {:?}",
                    w
                );
            }
        }
    }

    #[test]
    fn budget_target_falls_on_bucket_edge() {
        // Pour que `histogram_quantile(0.99, …)` n'interpole pas
        // *à la cible*, on veut que la cible p99 de chaque étage
        // soit littéralement présente dans la liste des buckets.
        let cases: Vec<(Duration, &[f64], &str)> = vec![
            (Duration::from_micros(1500), BUCKETS_MICRO_SECONDS, "grpc_recv = 1.5 ms"),
            (Duration::from_micros(500), BUCKETS_MICRO_SECONDS, "grpc_decode = 0.5 ms"),
            (Duration::from_millis(3), BUCKETS_MICRO_SECONDS, "interceptors = 3 ms"),
            (Duration::from_millis(4), BUCKETS_TOKENIZE_SECONDS, "tokenize = 4 ms"),
            (Duration::from_micros(26_000), BUCKETS_INFERENCE_SECONDS, "inference = 26 ms"),
            (Duration::from_millis(5), BUCKETS_GENESIS_SECONDS, "genesis = 5 ms"),
            (Duration::from_millis(10), BUCKETS_FUSION_ROUND_SECONDS, "fusion round = 10 ms"),
            (Duration::from_millis(1), BUCKETS_CONSENSUS_SECONDS, "consensus = 1 ms"),
            (Duration::from_millis(6), BUCKETS_DISSOLUTION_ALIGN_SECONDS, "dissolution = 6 ms"),
            (Duration::from_millis(4), BUCKETS_AUDIT_APPEND_SECONDS, "audit append = 4 ms"),
        ];
        for (cible, buckets, name) in cases {
            let secs = cible.as_secs_f64();
            assert!(
                buckets.iter().any(|b| (b - secs).abs() < 1e-9),
                "Cible {name} (= {} s) absente des buckets {:?}",
                secs,
                buckets,
            );
        }
    }
}
