// SPDX-License-Identifier: UNLICENSED
//! Sprint **D19 (P2-Foundation)** — Prometheus instruments for the
//! P2 *Activation opposable / Self-service* surface.
//!
//! Cf. `docs/EFE_P2_SELF_SERVICE_ARCHITECTURE.md` § 4.2.
//!
//! ## Inventaire (10 instruments)
//!
//! | # | Métrique                                          | Type      | Labels                |
//! |--:|---------------------------------------------------|-----------|-----------------------|
//! | 1 | `efe_p2_funnel_started_total`                     | Counter   | `plan_target`,`cloud` |
//! | 2 | `efe_p2_funnel_completed_total`                   | Counter   | `plan_target`,`cloud`,`outcome` |
//! | 3 | `efe_p2_funnel_step_duration_seconds`             | Histogram | `plan_target`,`step`  |
//! | 4 | `efe_p2_funnel_t_to_first_classify_seconds`       | Histogram | `plan_target`,`cloud` |
//! | 5 | `efe_p2_funnel_t_to_aha_seconds`                  | Histogram | `plan_target`         |
//! | 6 | `efe_p2_funnel_drop_total`                        | Counter   | `plan_target`,`step`,`reason` |
//! | 7 | `efe_p2_funnel_provisioning_seconds`              | Histogram | `cloud`,`layer`       |
//! | 8 | `efe_p2_funnel_quickstart_choice_total`           | Counter   | `language`            |
//! | 9 | `efe_p2_funnel_rollback_total`                    | Counter   | `step`,`reason`       |
//! |10 | `efe_p2_funnel_active_sessions`                   | Gauge     | `plan_target`         |
//!
//! ## Cardinalité
//!
//! - `plan_target` ∈ {`sandbox`, `poc`, `pro`, `enterprise`} = 4.
//! - `cloud`       ∈ {`aws`, `gcp`, `azure`, `none`} = 4.
//! - `outcome`     ∈ {`success`, `abandoned`, `error`} = 3.
//! - `step`        ∈ 14 valeurs (cf. [`FunnelStep`]) = 14.
//! - `reason`      ∈ 14 valeurs (cf. [`ErrorClass`]) = 14.
//! - `language`    ∈ {`curl`, `python`, `typescript`, `java`, `go`, `rust`} = 6.
//! - `layer`       ∈ {`layer_1`, `layer_2`, `layer_3`, `layer_4`} = 4.
//!
//! Cardinalité totale projetée : ~ 1 500 séries au pic, largement
//! sous le budget D13 50 000.
//!
//! ## Doctrine des buckets
//!
//! Les histograms P2 mesurent des durées **de l'ordre de la minute**
//! (vs P1 qui mesure des millisecondes). Les buckets canoniques :
//!
//! ```text
//! [1, 5, 10, 30, 60, 120, 300, 600, 1200, 1800, 3600, 7200, 14400] s
//! = [1s, 5s, 10s, 30s, 1m, 2m, 5m, 10m, 20m, 30m, 1h, 2h, 4h]
//! ```
//!
//! Les seuils SLO (300 s sandbox, 1 800 s POC, 86 400 s prod)
//! tombent exactement sur des bords de bucket — sanity-check pin
//! par `slo_targets_fall_on_bucket_edges`.

use std::sync::OnceLock;

use prometheus::{HistogramOpts, HistogramVec, IntCounterVec, IntGaugeVec, Opts};
use serde::{Deserialize, Serialize};

use super::registry::global_registry;

// ====================================================================
// Macro paresse + register-once (= calquée sur p1_instruments.rs).
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
// Buckets canoniques (publics)
// ====================================================================

/// Buckets pour toutes les durées du funnel (en secondes).
///
/// Construits pour que les 3 seuils SLO (5 / 30 min / 24 h) tombent
/// exactement sur un bord. Sans ça, `histogram_quantile(0.90, …)`
/// interpole entre 2 buckets et perd la précision à la cible.
pub const BUCKETS_FUNNEL_SECONDS: &[f64] = &[
    1.0, 5.0, 10.0, 30.0,
    60.0,        // 1 min
    120.0,       // 2 min
    300.0,       // 5 min  ← SLO Sandbox (T-T-Sandbox)
    600.0,       // 10 min
    1200.0,      // 20 min
    1800.0,      // 30 min ← SLO POC (T-T-POC)
    3600.0,      // 1 h
    7200.0,      // 2 h
    14400.0,     // 4 h
    86400.0,     // 24 h   ← SLO Production (T-T-Prod)
];

/// Buckets pour le `step_duration` (= durée d'une **étape** du funnel,
/// granularité plus fine puisque la plupart < 1 min).
pub const BUCKETS_STEP_SECONDS: &[f64] = &[
    0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0, 600.0, 1200.0,
];

/// Buckets pour `provisioning_seconds` (= durée d'**un layer**
/// Terraform, typiquement 1-10 min selon layer × cloud).
pub const BUCKETS_PROVISIONING_SECONDS: &[f64] = &[
    10.0, 30.0, 60.0, 120.0, 180.0, 240.0, 300.0, 480.0, 600.0,
    900.0, 1200.0, 1800.0,
];

// ====================================================================
// 1. Counter — funnel_started_total
// ====================================================================

once_metric!(
    /// `efe_p2_funnel_started_total{plan_target,cloud}` —
    /// nombre de sessions initiées (= row inserted in
    /// `onboarding_sessions` avec `started_at` set).
    funnel_started_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_p2_funnel_started_total",
            "Activation funnel sessions started (D19 P2 § 4.2 #1).",
        ),
        &["plan_target", "cloud"],
    )
    .expect("valid counter opts")
);

// ====================================================================
// 2. Counter — funnel_completed_total
// ====================================================================

once_metric!(
    /// `efe_p2_funnel_completed_total{plan_target,cloud,outcome}` —
    /// sessions terminées avec leur outcome.
    funnel_completed_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_p2_funnel_completed_total",
            "Activation funnel sessions completed by outcome (D19 P2 § 4.2 #2).",
        ),
        &["plan_target", "cloud", "outcome"],
    )
    .expect("valid counter opts")
);

// ====================================================================
// 3. Histogram — funnel_step_duration_seconds
// ====================================================================

once_metric!(
    /// `efe_p2_funnel_step_duration_seconds{plan_target,step}` —
    /// durée d'**une** étape du funnel.
    funnel_step_duration_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_p2_funnel_step_duration_seconds",
            "Per-step duration in the activation funnel (D19 P2 § 4.2 #3).",
        )
        .buckets(BUCKETS_STEP_SECONDS.to_vec()),
        &["plan_target", "step"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// 4. Histogram — t_to_first_classify_seconds (LE SLO RACINE)
// ====================================================================

once_metric!(
    /// `efe_p2_funnel_t_to_first_classify_seconds{plan_target,cloud}` —
    /// la **métrique racine** de P2 : `completed_at - started_at`
    /// pour les sessions qui ont atteint `first_classify_at`.
    ///
    /// Source de `SLO-P2-TTFC-{1,2,3}`.
    funnel_t_to_first_classify_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_p2_funnel_t_to_first_classify_seconds",
            "Time from session start to first successful classify (D19 P2 § 4.2 #4).",
        )
        .buckets(BUCKETS_FUNNEL_SECONDS.to_vec()),
        &["plan_target", "cloud"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// 5. Histogram — t_to_aha_seconds
// ====================================================================

once_metric!(
    /// `efe_p2_funnel_t_to_aha_seconds{plan_target}` — temps
    /// entre le start de la session et le **premier classify
    /// correct** (= aha moment, `first_classify_correct_at`).
    ///
    /// Source de `SLO-P2-AHA-{1,2}`.
    funnel_t_to_aha_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_p2_funnel_t_to_aha_seconds",
            "Time from session start to first CORRECT classify (D19 P2 § 4.2 #5).",
        )
        .buckets(BUCKETS_FUNNEL_SECONDS.to_vec()),
        &["plan_target"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// 6. Counter — funnel_drop_total (par étape × raison)
// ====================================================================

once_metric!(
    /// `efe_p2_funnel_drop_total{plan_target,step,reason}` —
    /// abandons par étape × raison structurée.
    ///
    /// `reason` est un `error_class` canonique parmi 14
    /// (cf. [`ErrorClass`]).
    funnel_drop_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_p2_funnel_drop_total",
            "Funnel abandons per step × error_class (D19 P2 § 4.2 #6).",
        ),
        &["plan_target", "step", "reason"],
    )
    .expect("valid counter opts")
);

// ====================================================================
// 7. Histogram — provisioning_seconds (cloud × layer)
// ====================================================================

once_metric!(
    /// `efe_p2_funnel_provisioning_seconds{cloud,layer}` — durée
    /// du provisioning par cloud × layer Terraform.
    funnel_provisioning_seconds,
    HistogramVec,
    HistogramVec::new(
        HistogramOpts::new(
            "efe_p2_funnel_provisioning_seconds",
            "Per-layer Terraform apply duration (D19 P2 § 4.2 #7).",
        )
        .buckets(BUCKETS_PROVISIONING_SECONDS.to_vec()),
        &["cloud", "layer"],
    )
    .expect("valid histogram opts")
);

// ====================================================================
// 8. Counter — quickstart_choice (préférence dev)
// ====================================================================

once_metric!(
    /// `efe_p2_funnel_quickstart_choice_total{language}` — popularité
    /// des snippets quickstart par langage.
    funnel_quickstart_choice_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_p2_funnel_quickstart_choice_total",
            "Quickstart language picked by user (D19 P2 § 4.2 #8).",
        ),
        &["language"],
    )
    .expect("valid counter opts")
);

// ====================================================================
// 9. Counter — rollback_total
// ====================================================================

once_metric!(
    /// `efe_p2_funnel_rollback_total{step,reason}` — rollbacks
    /// déclenchés (saga compensation).
    funnel_rollback_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_p2_funnel_rollback_total",
            "Saga rollbacks per step × reason (D19 P2 § 4.2 #9).",
        ),
        &["step", "reason"],
    )
    .expect("valid counter opts")
);

// ====================================================================
// 10. Gauge — active_sessions
// ====================================================================

once_metric!(
    /// `efe_p2_funnel_active_sessions{plan_target}` — sessions
    /// in-flight (= `started_at` set, `completed_at IS NULL`).
    ///
    /// Mis à jour par le `funnel_events::tick_active_gauge` cron
    /// (toutes les 30 s par défaut, atomique-safe vs Prometheus
    /// scrape).
    funnel_active_sessions,
    IntGaugeVec,
    IntGaugeVec::new(
        Opts::new(
            "efe_p2_funnel_active_sessions",
            "In-flight activation sessions (D19 P2 § 4.2 #10).",
        ),
        &["plan_target"],
    )
    .expect("valid gauge opts")
);

// ====================================================================
// Énumérations canoniques (= source de vérité unique)
// ====================================================================

/// Cible d'activation. **L'ordre du `match` est sémantique** :
/// `Sandbox` < `Poc` < `Pro` < `Enterprise` (= up-tier).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanTarget {
    /// Trial gratuit, infra Fournisseur partagée (5 min p90).
    Sandbox,
    /// POC tenant, infra dédiée (30 min p90).
    Poc,
    /// Tenant Pro paying (24 h p90).
    Pro,
    /// Tenant Enterprise (custom SLA).
    Enterprise,
}

impl PlanTarget {
    /// Stable string for Prometheus labels + DB CHECK constraint.
    #[must_use]
    pub const fn as_label(self) -> &'static str {
        match self {
            Self::Sandbox => "sandbox",
            Self::Poc => "poc",
            Self::Pro => "pro",
            Self::Enterprise => "enterprise",
        }
    }

    /// Cible p90 du T-T-First-Classify pour ce plan (en secondes).
    /// Source de vérité pour les SLO. Sanity-check pin par
    /// `plan_target_slo_targets_match_doc`.
    #[must_use]
    pub const fn ttfc_p90_seconds(self) -> u64 {
        match self {
            Self::Sandbox => 300,       // 5 min
            Self::Poc => 1_800,         // 30 min
            Self::Pro | Self::Enterprise => 86_400, // 24 h
        }
    }

    /// Itérateur canonique.
    #[must_use]
    pub const fn all() -> &'static [PlanTarget; 4] {
        &[Self::Sandbox, Self::Poc, Self::Pro, Self::Enterprise]
    }
}

/// Cloud-cible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudTarget {
    /// AWS (any region).
    Aws,
    /// Google Cloud Platform.
    Gcp,
    /// Microsoft Azure.
    Azure,
    /// Pas de cloud (= sandbox sur infra Fournisseur).
    None,
}

impl CloudTarget {
    /// Stable string for Prometheus labels.
    #[must_use]
    pub const fn as_label(self) -> &'static str {
        match self {
            Self::Aws => "aws",
            Self::Gcp => "gcp",
            Self::Azure => "azure",
            Self::None => "none",
        }
    }
}

/// Étape du funnel (alignée avec `event_kind` CHECK constraint en SQL).
///
/// **L'ordre est sémantique** — c'est l'ordre canonique du parcours
/// (cf. doc P2 § 2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FunnelStep {
    /// Étape 1 — Landing → click (≤ 30 s).
    LandingClick,
    /// Étape 2 — Signup form fill.
    SignupForm,
    /// Étape 3 — Email verify.
    EmailVerify,
    /// Étape 4 — Card / SSO.
    PaymentAuthorize,
    /// Étape 5 — Plan + cloud picker.
    PlanSelect,
    /// Étape 6 — Cloud provisioning (Terraform apply, all layers).
    Provisioning,
    /// Étape 7 — Helm install.
    HelmInstall,
    /// Étape 8 — Health smoke.
    Healthcheck,
    /// Étape 9 — Catalog seed.
    CatalogSeed,
    /// Étape 10 — API key + JWT init.
    AuthInit,
    /// Étape 11 — First classify quickstart.
    FirstClassify,
    /// Étape 12 — Verdict render.
    RenderVerdict,
    /// Étape 13 — Tutorial nudge.
    Tutorial,
    /// Étape 14 — Aha moment.
    Aha,
}

impl FunnelStep {
    /// Stable string for Prometheus labels.
    #[must_use]
    pub const fn as_label(self) -> &'static str {
        match self {
            Self::LandingClick => "landing_click",
            Self::SignupForm => "signup_form",
            Self::EmailVerify => "email_verify",
            Self::PaymentAuthorize => "payment_authorize",
            Self::PlanSelect => "plan_select",
            Self::Provisioning => "provisioning",
            Self::HelmInstall => "helm_install",
            Self::Healthcheck => "healthcheck",
            Self::CatalogSeed => "catalog_seed",
            Self::AuthInit => "auth_init",
            Self::FirstClassify => "first_classify",
            Self::RenderVerdict => "render_verdict",
            Self::Tutorial => "tutorial",
            Self::Aha => "aha",
        }
    }

    /// Budget canonique p90 (en secondes) — source de vérité pour
    /// les SLO par étape (cf. doc P2 § 2.1).
    ///
    /// La **somme** des `budget_seconds` des 14 étapes vaut
    /// **1 355 s** ; la différence avec le plafond POC
    /// (1 800 s = 30 min) constitue la **marge anti-tail** de
    /// 445 s ≈ 7 min, absorbée par les latences inter-étape
    /// (think-time user, retry réseau, propagation DNS, etc.).
    /// Sanity-check pin par `funnel_step_budget_sum_under_poc_cap`.
    #[must_use]
    pub const fn budget_seconds(self) -> u64 {
        match self {
            Self::LandingClick => 30,
            Self::SignupForm => 90,
            Self::EmailVerify => 60,
            Self::PaymentAuthorize => 60,
            Self::PlanSelect => 30,
            Self::Provisioning => 480,
            Self::HelmInstall => 240,
            Self::Healthcheck => 30,
            Self::CatalogSeed => 120,
            Self::AuthInit => 30,
            Self::FirstClassify => 90,
            Self::RenderVerdict => 5,
            Self::Tutorial => 60,
            Self::Aha => 30,
        }
    }

    /// Itérateur canonique des 14 étapes dans l'ordre du parcours.
    #[must_use]
    pub const fn all() -> &'static [FunnelStep; 14] {
        &[
            Self::LandingClick,
            Self::SignupForm,
            Self::EmailVerify,
            Self::PaymentAuthorize,
            Self::PlanSelect,
            Self::Provisioning,
            Self::HelmInstall,
            Self::Healthcheck,
            Self::CatalogSeed,
            Self::AuthInit,
            Self::FirstClassify,
            Self::RenderVerdict,
            Self::Tutorial,
            Self::Aha,
        ]
    }
}

/// Layer Terraform (= 4 niveaux d'apply).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerraformLayer {
    /// VPC + subnets + NAT.
    Layer1,
    /// Compute control plane (EKS/GKE/AKS).
    Layer2,
    /// Data plane (RDS + Redis).
    Layer3,
    /// Application stack (Helm install).
    Layer4,
}

impl TerraformLayer {
    /// Stable string for Prometheus labels.
    #[must_use]
    pub const fn as_label(self) -> &'static str {
        match self {
            Self::Layer1 => "layer_1",
            Self::Layer2 => "layer_2",
            Self::Layer3 => "layer_3",
            Self::Layer4 => "layer_4",
        }
    }
}

/// Outcome d'une session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// `first_classify_correct_at` setté → aha atteint.
    Success,
    /// `abandoned_at` setté (user a fermé la session).
    Abandoned,
    /// Erreur côté Fournisseur (`error_class != null`).
    Error,
}

impl Outcome {
    /// Stable string for Prometheus labels.
    #[must_use]
    pub const fn as_label(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Abandoned => "abandoned",
            Self::Error => "error",
        }
    }
}

/// Quickstart language (= popularité des snippets).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuickstartLang {
    /// Plain `curl` (= shell).
    Curl,
    /// Python SDK.
    Python,
    /// TypeScript SDK.
    Typescript,
    /// Java SDK.
    Java,
    /// Go SDK.
    Go,
    /// Rust client SDK.
    Rust,
}

impl QuickstartLang {
    /// Stable string for Prometheus labels.
    #[must_use]
    pub const fn as_label(self) -> &'static str {
        match self {
            Self::Curl => "curl",
            Self::Python => "python",
            Self::Typescript => "typescript",
            Self::Java => "java",
            Self::Go => "go",
            Self::Rust => "rust",
        }
    }
}

// ====================================================================
// ErrorClass — 14 codes structurés (= cardinalité bornée)
// ====================================================================

/// Code d'erreur structuré, **stable**, qui apparaît dans :
/// - `onboarding_events.error_class` (DB)
/// - `efe_p2_funnel_drop_total{reason}` (Prometheus)
/// - Réponses HTTP `application/problem+json` (`type` field)
/// - i18n FR/EN dans le portail
///
/// Ajouter un code requiert :
/// 1. ajouter une variante ici,
/// 2. ajouter le mapping `as_label()`,
/// 3. ajouter au test `error_class_count_matches_canonical_14`,
/// 4. ajouter les traductions FR + EN dans le portail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorClass {
    /// Email mal formé (RFC 5322).
    EmailInvalid,
    /// Domaine bloqué (gmail consumer sur plan enterprise, etc.).
    EmailDomainBlocked,
    /// Carte refusée par l'issuer.
    PaymentDeclined,
    /// 3-D Secure required, user n'a pas complété.
    Payment3dsRequired,
    /// Quota cloud dépassé (EIP, EBS, etc.).
    CloudQuotaExceeded,
    /// Région non disponible / non supportée.
    CloudRegionUnavailable,
    /// OIDC trust mal configuré.
    CloudAuthFailed,
    /// Terraform apply timeout (> 15 min).
    TfApplyTimeout,
    /// Helm install failed (image pull, template error).
    HelmInstallFailed,
    /// JSONL catalog malformé ou > 1 000 cas.
    CatalogSeedInvalid,
    /// Template catalog demandé inexistant.
    CatalogTemplateMissing,
    /// KMS / Vault inaccessible.
    ApiKeyGenFailed,
    /// gRPC call > 30 s.
    FirstCallTimeout,
    /// Tout le reste (= ticket support).
    Internal,
}

impl ErrorClass {
    /// Stable string label (lowercase, snake_case).
    #[must_use]
    pub const fn as_label(self) -> &'static str {
        match self {
            Self::EmailInvalid => "email_invalid",
            Self::EmailDomainBlocked => "email_domain_blocked",
            Self::PaymentDeclined => "payment_declined",
            Self::Payment3dsRequired => "payment_3ds_required",
            Self::CloudQuotaExceeded => "cloud_quota_exceeded",
            Self::CloudRegionUnavailable => "cloud_region_unavailable",
            Self::CloudAuthFailed => "cloud_auth_failed",
            Self::TfApplyTimeout => "tf_apply_timeout",
            Self::HelmInstallFailed => "helm_install_failed",
            Self::CatalogSeedInvalid => "catalog_seed_invalid",
            Self::CatalogTemplateMissing => "catalog_template_missing",
            Self::ApiKeyGenFailed => "api_key_gen_failed",
            Self::FirstCallTimeout => "first_call_timeout",
            Self::Internal => "internal",
        }
    }

    /// Itérateur canonique. Le test `error_class_count_matches_canonical_14`
    /// vérifie que la longueur = 14 (= contract avec le doc + l'i18n).
    #[must_use]
    pub const fn all() -> &'static [ErrorClass; 14] {
        &[
            Self::EmailInvalid,
            Self::EmailDomainBlocked,
            Self::PaymentDeclined,
            Self::Payment3dsRequired,
            Self::CloudQuotaExceeded,
            Self::CloudRegionUnavailable,
            Self::CloudAuthFailed,
            Self::TfApplyTimeout,
            Self::HelmInstallFailed,
            Self::CatalogSeedInvalid,
            Self::CatalogTemplateMissing,
            Self::ApiKeyGenFailed,
            Self::FirstCallTimeout,
            Self::Internal,
        ]
    }

    /// Indique si cet `error_class` est **récupérable côté user**
    /// (= remediation possible sans intervention support).
    ///
    /// Utilisé pour décider si on affiche un bouton "Retry" vs
    /// "Open support ticket" dans le portal.
    #[must_use]
    pub const fn is_user_recoverable(self) -> bool {
        match self {
            // User peut corriger : reformuler email, autre carte,
            // 3-D Secure, autre région, autre template, JSONL plus petit
            Self::EmailInvalid
            | Self::EmailDomainBlocked
            | Self::PaymentDeclined
            | Self::Payment3dsRequired
            | Self::CloudRegionUnavailable
            | Self::CatalogSeedInvalid
            | Self::CatalogTemplateMissing => true,

            // User doit contacter le support : quota cloud, auth
            // failed, timeouts, internal
            Self::CloudQuotaExceeded
            | Self::CloudAuthFailed
            | Self::TfApplyTimeout
            | Self::HelmInstallFailed
            | Self::ApiKeyGenFailed
            | Self::FirstCallTimeout
            | Self::Internal => false,
        }
    }
}

// ====================================================================
// API ergonomique — observers côté backend
// ====================================================================

/// Observe le start d'une session.
pub fn note_session_started(plan: PlanTarget, cloud: CloudTarget) {
    funnel_started_total()
        .with_label_values(&[plan.as_label(), cloud.as_label()])
        .inc();
    funnel_active_sessions()
        .with_label_values(&[plan.as_label()])
        .inc();
}

/// Observe la fin d'une session.
pub fn note_session_completed(
    plan: PlanTarget,
    cloud: CloudTarget,
    outcome: Outcome,
    duration: std::time::Duration,
) {
    funnel_completed_total()
        .with_label_values(&[plan.as_label(), cloud.as_label(), outcome.as_label()])
        .inc();
    funnel_active_sessions()
        .with_label_values(&[plan.as_label()])
        .dec();
    if matches!(outcome, Outcome::Success) {
        funnel_t_to_first_classify_seconds()
            .with_label_values(&[plan.as_label(), cloud.as_label()])
            .observe(duration.as_secs_f64());
    }
}

/// Observe l'atteinte du aha-moment.
pub fn note_aha(plan: PlanTarget, duration_to_aha: std::time::Duration) {
    funnel_t_to_aha_seconds()
        .with_label_values(&[plan.as_label()])
        .observe(duration_to_aha.as_secs_f64());
}

/// Observe la durée d'une étape du funnel.
pub fn note_step_duration(
    plan: PlanTarget,
    step: FunnelStep,
    duration: std::time::Duration,
) {
    funnel_step_duration_seconds()
        .with_label_values(&[plan.as_label(), step.as_label()])
        .observe(duration.as_secs_f64());
}

/// Observe un abandon de session à une étape.
pub fn note_drop(plan: PlanTarget, step: FunnelStep, reason: ErrorClass) {
    funnel_drop_total()
        .with_label_values(&[plan.as_label(), step.as_label(), reason.as_label()])
        .inc();
}

/// Observe la durée d'un layer Terraform.
pub fn note_provisioning(
    cloud: CloudTarget,
    layer: TerraformLayer,
    duration: std::time::Duration,
) {
    funnel_provisioning_seconds()
        .with_label_values(&[cloud.as_label(), layer.as_label()])
        .observe(duration.as_secs_f64());
}

/// Note quel langage l'user a choisi dans le quickstart.
pub fn note_quickstart_choice(language: QuickstartLang) {
    funnel_quickstart_choice_total()
        .with_label_values(&[language.as_label()])
        .inc();
}

/// Note un rollback déclenché.
pub fn note_rollback(step: FunnelStep, reason: ErrorClass) {
    funnel_rollback_total()
        .with_label_values(&[step.as_label(), reason.as_label()])
        .inc();
}

/// Met à jour le gauge `active_sessions` depuis un compte
/// authoritative (= query SQL dans la cron `tick_active_gauge`).
///
/// Cette méthode est **idempotente** : appeler plusieurs fois avec
/// la même valeur ne change rien. Utile pour resync après un crash
/// du serveur (les compteurs in-process ont perdu leur état).
pub fn set_active_sessions(plan: PlanTarget, count: i64) {
    funnel_active_sessions()
        .with_label_values(&[plan.as_label()])
        .set(count);
}

// ====================================================================
// Eager registration
// ====================================================================

/// Force-register every P2 instrument with the global registry.
/// Idempotent thanks to `OnceLock`.
///
/// **Pattern P2-A1** — `Server::build` MUST call this so the
/// Grafana `efe-p2-funnel.json` dashboard panels render
/// zero-valued series at boot rather than "no data". Cf.
/// `crates/efe-server/src/runtime.rs` et le pattern récurrent
/// D15-H5 / D16-C1 / D18-A3 / D19-P1.A1.
pub fn register_all_p2() {
    let _ = funnel_started_total();
    let _ = funnel_completed_total();
    let _ = funnel_step_duration_seconds();
    let _ = funnel_t_to_first_classify_seconds();
    let _ = funnel_t_to_aha_seconds();
    let _ = funnel_drop_total();
    let _ = funnel_provisioning_seconds();
    let _ = funnel_quickstart_choice_total();
    let _ = funnel_rollback_total();
    let _ = funnel_active_sessions();
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
    fn register_all_p2_is_idempotent() {
        register_all_p2();
        register_all_p2();
        register_all_p2();
    }

    #[test]
    fn all_10_instruments_have_expected_label_arity() {
        // 10 instruments, leur cardinalité de label doit matcher
        // exactement le doc P2 § 4.2.
        let cases: Vec<(&str, Vec<&str>)> = vec![
            ("funnel_started_total", vec!["plan_target", "cloud"]),
            ("funnel_completed_total", vec!["plan_target", "cloud", "outcome"]),
            ("funnel_step_duration_seconds", vec!["plan_target", "step"]),
            ("funnel_t_to_first_classify_seconds", vec!["plan_target", "cloud"]),
            ("funnel_t_to_aha_seconds", vec!["plan_target"]),
            ("funnel_drop_total", vec!["plan_target", "step", "reason"]),
            ("funnel_provisioning_seconds", vec!["cloud", "layer"]),
            ("funnel_quickstart_choice_total", vec!["language"]),
            ("funnel_rollback_total", vec!["step", "reason"]),
            ("funnel_active_sessions", vec!["plan_target"]),
        ];
        // Note : la map ↔ getter est tested ci-dessous one-by-one
        // car on ne peut pas trivialement boxer les diverses
        // `&'static *Vec` types.
        assert_eq!(
            funnel_started_total().desc()[0].variable_labels,
            cases[0].1.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        );
        assert_eq!(
            funnel_completed_total().desc()[0].variable_labels,
            cases[1].1.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        );
        assert_eq!(
            funnel_step_duration_seconds().desc()[0].variable_labels,
            cases[2].1.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        );
        assert_eq!(
            funnel_t_to_first_classify_seconds().desc()[0].variable_labels,
            cases[3].1.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        );
        assert_eq!(
            funnel_t_to_aha_seconds().desc()[0].variable_labels,
            cases[4].1.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        );
        assert_eq!(
            funnel_drop_total().desc()[0].variable_labels,
            cases[5].1.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        );
        assert_eq!(
            funnel_provisioning_seconds().desc()[0].variable_labels,
            cases[6].1.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        );
        assert_eq!(
            funnel_quickstart_choice_total().desc()[0].variable_labels,
            cases[7].1.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        );
        assert_eq!(
            funnel_rollback_total().desc()[0].variable_labels,
            cases[8].1.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        );
        assert_eq!(
            funnel_active_sessions().desc()[0].variable_labels,
            cases[9].1.iter().map(|s| s.to_string()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn funnel_step_budget_sum_under_poc_cap() {
        // Pin contractuel : la somme des budgets canoniques par
        // étape doit rester < le plafond SLO POC (1 800 s).
        // L'écart constitue la **marge anti-tail** absorbant les
        // latences inter-étape (think-time, retry réseau, DNS).
        //
        // Valeur attendue actuelle : 1 355 s (= ~22.6 min), soit
        // 445 s = ~7.4 min de marge. Toute dérive du `budget_seconds`
        // d'un step est détectée par ce test (la somme bouge).
        let total: u64 = FunnelStep::all().iter().map(|s| s.budget_seconds()).sum();
        const POC_CAP_SECONDS: u64 = 1_800;
        assert!(
            total <= POC_CAP_SECONDS,
            "step budgets sum to {total} s, exceeds POC cap {POC_CAP_SECONDS} s"
        );
        // Sanity inférieur : ≥ 1 000 s (= la marge ne doit pas
        // excéder ~45 % du budget, sinon on sur-estime la marge
        // anti-tail et on biaise le SLO).
        assert!(
            total >= 1_000,
            "step budgets sum to only {total} s — under 1000 s suspicious"
        );
        // Pin numérique exact : si quelqu'un change un budget, le
        // test fail explicitement avec la nouvelle valeur attendue.
        assert_eq!(
            total, 1_355,
            "if intentionally changed, update doc § 2.1 + this constant"
        );
    }

    #[test]
    fn step_labels_are_distinct() {
        let mut labels: Vec<&str> = FunnelStep::all().iter().map(|s| s.as_label()).collect();
        labels.sort();
        labels.dedup();
        assert_eq!(labels.len(), 14, "14 distinct step labels expected");
    }

    #[test]
    fn error_class_count_matches_canonical_14() {
        // Pin contractuel : 14 codes documentés dans le doc P2 § 5.4.
        // Toute addition requiert update du doc + tests + i18n.
        assert_eq!(ErrorClass::all().len(), 14);

        let mut labels: Vec<&str> = ErrorClass::all().iter().map(|e| e.as_label()).collect();
        labels.sort();
        labels.dedup();
        assert_eq!(labels.len(), 14, "14 distinct error_class labels expected");
    }

    #[test]
    fn plan_target_slo_targets_match_doc() {
        assert_eq!(PlanTarget::Sandbox.ttfc_p90_seconds(), 300);
        assert_eq!(PlanTarget::Poc.ttfc_p90_seconds(), 1_800);
        assert_eq!(PlanTarget::Pro.ttfc_p90_seconds(), 86_400);
        assert_eq!(PlanTarget::Enterprise.ttfc_p90_seconds(), 86_400);
    }

    #[test]
    fn slo_targets_fall_on_bucket_edges() {
        // Les 3 cibles SLO doivent tomber sur un bord exact de
        // bucket pour que histogram_quantile(0.90, …) retourne la
        // cible sans interpolation.
        let targets: &[f64] = &[300.0, 1_800.0, 86_400.0];
        for t in targets {
            assert!(
                BUCKETS_FUNNEL_SECONDS.iter().any(|b| (b - t).abs() < 1e-9),
                "SLO target {t} s absente des buckets {:?}",
                BUCKETS_FUNNEL_SECONDS
            );
        }
    }

    #[test]
    fn buckets_strictly_increasing() {
        for (name, b) in &[
            ("funnel", BUCKETS_FUNNEL_SECONDS),
            ("step", BUCKETS_STEP_SECONDS),
            ("provisioning", BUCKETS_PROVISIONING_SECONDS),
        ] {
            for w in b.windows(2) {
                assert!(w[0] < w[1], "{name}: not strictly increasing: {:?}", w);
            }
        }
    }

    #[test]
    fn error_class_recoverable_split() {
        // Distinction user-recoverable vs support-required.
        let user_count = ErrorClass::all().iter()
            .filter(|e| e.is_user_recoverable())
            .count();
        let support_count = ErrorClass::all().iter()
            .filter(|e| !e.is_user_recoverable())
            .count();
        // 7 + 7 = 14 ; vérifie que la partition est totale.
        assert_eq!(user_count + support_count, 14);
        // Au moins 1 de chaque côté pour éviter une régression
        // accidentelle (tout passe à user-recoverable ou inverse).
        assert!(user_count > 0);
        assert!(support_count > 0);
    }

    #[test]
    fn note_session_started_increments_counter_and_gauge() {
        register_all_p2();
        let before = funnel_started_total()
            .with_label_values(&["poc", "aws"])
            .get();
        let g_before = funnel_active_sessions()
            .with_label_values(&["poc"])
            .get();
        note_session_started(PlanTarget::Poc, CloudTarget::Aws);
        assert_eq!(
            funnel_started_total().with_label_values(&["poc", "aws"]).get(),
            before + 1
        );
        assert_eq!(
            funnel_active_sessions().with_label_values(&["poc"]).get(),
            g_before + 1
        );
    }

    #[test]
    fn note_session_completed_records_outcome_and_decrements_gauge() {
        register_all_p2();
        // Pump up active count d'abord
        note_session_started(PlanTarget::Sandbox, CloudTarget::None);
        let g_after_start = funnel_active_sessions()
            .with_label_values(&["sandbox"])
            .get();
        note_session_completed(
            PlanTarget::Sandbox,
            CloudTarget::None,
            Outcome::Success,
            Duration::from_secs(240),
        );
        assert_eq!(
            funnel_active_sessions().with_label_values(&["sandbox"]).get(),
            g_after_start - 1
        );
    }

    #[test]
    fn note_drop_emits_label() {
        register_all_p2();
        let before = funnel_drop_total()
            .with_label_values(&["poc", "provisioning", "cloud_quota_exceeded"])
            .get();
        note_drop(
            PlanTarget::Poc,
            FunnelStep::Provisioning,
            ErrorClass::CloudQuotaExceeded,
        );
        assert_eq!(
            funnel_drop_total()
                .with_label_values(&["poc", "provisioning", "cloud_quota_exceeded"])
                .get(),
            before + 1
        );
    }

    #[test]
    fn set_active_sessions_is_idempotent() {
        register_all_p2();
        set_active_sessions(PlanTarget::Pro, 42);
        set_active_sessions(PlanTarget::Pro, 42);
        assert_eq!(
            funnel_active_sessions().with_label_values(&["pro"]).get(),
            42
        );
        set_active_sessions(PlanTarget::Pro, 0);
        assert_eq!(
            funnel_active_sessions().with_label_values(&["pro"]).get(),
            0
        );
    }
}
