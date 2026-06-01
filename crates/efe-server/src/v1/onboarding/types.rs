// SPDX-License-Identifier: UNLICENSED
//! Types canoniques de la surface P2 self-service.
//!
//! Cf. `docs/EFE_P2_SELF_SERVICE_ARCHITECTURE.md` § 4.1.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// Re-export depuis efe-telemetry pour que les consumers aient une
// seule importation. La SOURCE DE VÉRITÉ des enums reste dans
// efe-telemetry (les labels Prometheus matchent les SQL CHECK
// constraints).
pub use efe_telemetry::metrics::p2_instruments::{
    CloudTarget, ErrorClass, FunnelStep, Outcome, PlanTarget, TerraformLayer,
};

/// UUID typé d'une session d'activation (= newtype pour éviter le
/// mix avec d'autres UUIDs côté handlers).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OnboardingSessionId(pub Uuid);

impl OnboardingSessionId {
    /// Génère un identifiant aléatoire.
    #[must_use]
    pub fn new_v4() -> Self {
        Self(Uuid::new_v4())
    }

    /// Lit la valeur UUID brute.
    #[must_use]
    pub const fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl std::fmt::Display for OnboardingSessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Snapshot complet d'une row `onboarding_sessions`.
///
/// Tous les `Option<DateTime>` matchent les colonnes nullable SQL.
/// Le champ `audit_signature` est `Some` uniquement après la
/// complétion de la session (= setté par `session_repo::seal`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardingSession {
    /// Identifiant unique de la session.
    pub session_id: OnboardingSessionId,
    /// Identifiant utilisateur.
    pub user_id: Uuid,
    /// Identifiant tenant (`None` avant `POST /v1/tenants`).
    pub tenant_id: Option<String>,

    /// Cible d'activation.
    pub plan_target: PlanTarget,
    /// Cible cloud (`None` pour Sandbox sur infra Fournisseur).
    pub cloud_target: Option<CloudTarget>,
    /// Région cloud.
    pub region: Option<String>,

    // Timestamps des transitions.
    /// T = 0 de la session.
    pub started_at: DateTime<Utc>,
    /// Email vérifié.
    pub email_verified_at: Option<DateTime<Utc>>,
    /// Paiement autorisé.
    pub payment_authorized_at: Option<DateTime<Utc>>,
    /// Provisioning démarré.
    pub provisioning_started_at: Option<DateTime<Utc>>,
    /// Provisioning Terraform terminé (layer 1-3).
    pub provisioning_done_at: Option<DateTime<Utc>>,
    /// Helm install terminé (layer 4).
    pub helm_done_at: Option<DateTime<Utc>>,
    /// Catalog seedé.
    pub catalog_seeded_at: Option<DateTime<Utc>>,
    /// Authentification initialisée.
    pub auth_initialized_at: Option<DateTime<Utc>>,
    /// 1er classify retourné (200 OK).
    pub first_classify_at: Option<DateTime<Utc>>,
    /// 1er classify correct (= aha moment).
    pub first_classify_correct_at: Option<DateTime<Utc>>,
    /// Session abandonnée (user a fermé l'onglet).
    pub abandoned_at: Option<DateTime<Utc>>,
    /// Code d'erreur si abandon ou error.
    pub abandoned_reason: Option<ErrorClass>,
    /// Session terminée (= aha OR abandoned OR error).
    pub completed_at: Option<DateTime<Utc>>,

    // Métriques agrégées matérialisées.
    /// Durée jusqu'au 1er classify (en secondes, NULL si pas atteint).
    pub duration_to_first_classify_seconds: Option<u32>,
    /// Durée jusqu'au aha-moment.
    pub duration_to_aha_seconds: Option<u32>,
    /// Nombre de rollbacks saga déclenchés.
    pub rollbacks_count: u32,
    /// Nombre d'erreurs durant la session.
    pub errors_count: u32,

    /// Snapshot head de la chaîne D14-bis au start.
    pub audit_chain_head_at_start: Option<Vec<u8>>,
    /// Snapshot head au end.
    pub audit_chain_head_at_end: Option<Vec<u8>>,
    /// HMAC-SHA-256 de la session (signature opposable).
    pub audit_signature: Option<Vec<u8>>,
}

impl OnboardingSession {
    /// Construit une session initiale (start uniquement).
    #[must_use]
    pub fn new(
        session_id: OnboardingSessionId,
        user_id: Uuid,
        plan_target: PlanTarget,
        cloud_target: Option<CloudTarget>,
        region: Option<String>,
    ) -> Self {
        Self {
            session_id,
            user_id,
            tenant_id: None,
            plan_target,
            cloud_target,
            region,
            started_at: Utc::now(),
            email_verified_at: None,
            payment_authorized_at: None,
            provisioning_started_at: None,
            provisioning_done_at: None,
            helm_done_at: None,
            catalog_seeded_at: None,
            auth_initialized_at: None,
            first_classify_at: None,
            first_classify_correct_at: None,
            abandoned_at: None,
            abandoned_reason: None,
            completed_at: None,
            duration_to_first_classify_seconds: None,
            duration_to_aha_seconds: None,
            rollbacks_count: 0,
            errors_count: 0,
            audit_chain_head_at_start: None,
            audit_chain_head_at_end: None,
            audit_signature: None,
        }
    }

    /// Renvoie `true` si la session est terminée (= `completed_at`
    /// setté).
    #[must_use]
    pub fn is_completed(&self) -> bool {
        self.completed_at.is_some()
    }

    /// Renvoie l'`Outcome` final si la session est complétée.
    ///
    /// Retourne `None` si pas encore complétée. Sinon retourne :
    /// - `Outcome::Success` si `first_classify_correct_at` set ;
    /// - `Outcome::Abandoned` si `abandoned_at` set sans error_class ;
    /// - `Outcome::Error` si `abandoned_reason` est `Some`.
    #[must_use]
    pub fn outcome(&self) -> Option<Outcome> {
        if !self.is_completed() {
            return None;
        }
        if self.first_classify_correct_at.is_some() {
            return Some(Outcome::Success);
        }
        if self.abandoned_reason.is_some() {
            return Some(Outcome::Error);
        }
        Some(Outcome::Abandoned)
    }

    /// Durée écoulée depuis `started_at` (en secondes, monotonic-style).
    /// Utile pour les timeouts et les métriques live.
    #[must_use]
    pub fn elapsed_seconds(&self) -> i64 {
        (Utc::now() - self.started_at).num_seconds()
    }
}

/// `event_kind` canonique — **doit** matcher la `CHECK` constraint
/// SQL de la table `onboarding_events` (cf. migration
/// `0028_d19_p2_onboarding.sql`).
///
/// L'ordre des variantes suit l'ordre du parcours user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FunnelEventKind {
    /// `session.started` — start de la session (= T = 0).
    SessionStarted,
    /// `step.signup_form_open` — affichage du form signup.
    StepSignupFormOpen,
    /// `step.signup_form_submit` — submit du form.
    StepSignupFormSubmit,
    /// `step.email_verified` — magic link cliqué.
    StepEmailVerified,
    /// `step.payment_authorized` — Stripe/SSO OK.
    StepPaymentAuthorized,
    /// `step.plan_selected` — plan + cloud choisi.
    StepPlanSelected,
    /// `step.provisioning_started` — Terraform apply commencé.
    StepProvisioningStarted,
    /// `step.provisioning_layer_1_done` — VPC done.
    StepProvisioningLayer1Done,
    /// `step.provisioning_layer_2_done` — Compute done.
    StepProvisioningLayer2Done,
    /// `step.provisioning_layer_3_done` — Data done.
    StepProvisioningLayer3Done,
    /// `step.helm_install_done` — Helm install OK.
    StepHelmInstallDone,
    /// `step.healthcheck_ok` — smoke E2E OK.
    StepHealthcheckOk,
    /// `step.catalog_seeded` — catalog seedé.
    StepCatalogSeeded,
    /// `step.auth_initialized` — API key gen + JWT.
    StepAuthInitialized,
    /// `step.first_classify_ok` — 1er classify retourné 200.
    StepFirstClassifyOk,
    /// `step.first_classify_correct` — 1er classify correct (= aha).
    StepFirstClassifyCorrect,
    /// `session.abandoned` — user a fermé l'onglet.
    SessionAbandoned,
    /// `session.completed` — session terminée (success ou error).
    SessionCompleted,
    /// `system.session_expired` — cron timeout 60 min.
    SystemSessionExpired,
    /// `system.rollback_triggered` — saga compensation.
    SystemRollbackTriggered,
}

impl FunnelEventKind {
    /// Stable string (= valeur stockée en DB).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SessionStarted => "session.started",
            Self::StepSignupFormOpen => "step.signup_form_open",
            Self::StepSignupFormSubmit => "step.signup_form_submit",
            Self::StepEmailVerified => "step.email_verified",
            Self::StepPaymentAuthorized => "step.payment_authorized",
            Self::StepPlanSelected => "step.plan_selected",
            Self::StepProvisioningStarted => "step.provisioning_started",
            Self::StepProvisioningLayer1Done => "step.provisioning_layer_1_done",
            Self::StepProvisioningLayer2Done => "step.provisioning_layer_2_done",
            Self::StepProvisioningLayer3Done => "step.provisioning_layer_3_done",
            Self::StepHelmInstallDone => "step.helm_install_done",
            Self::StepHealthcheckOk => "step.healthcheck_ok",
            Self::StepCatalogSeeded => "step.catalog_seeded",
            Self::StepAuthInitialized => "step.auth_initialized",
            Self::StepFirstClassifyOk => "step.first_classify_ok",
            Self::StepFirstClassifyCorrect => "step.first_classify_correct",
            Self::SessionAbandoned => "session.abandoned",
            Self::SessionCompleted => "session.completed",
            Self::SystemSessionExpired => "system.session_expired",
            Self::SystemRollbackTriggered => "system.rollback_triggered",
        }
    }

    /// Parse depuis un string. Renvoie `None` si non-reconnu.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "session.started" => Self::SessionStarted,
            "step.signup_form_open" => Self::StepSignupFormOpen,
            "step.signup_form_submit" => Self::StepSignupFormSubmit,
            "step.email_verified" => Self::StepEmailVerified,
            "step.payment_authorized" => Self::StepPaymentAuthorized,
            "step.plan_selected" => Self::StepPlanSelected,
            "step.provisioning_started" => Self::StepProvisioningStarted,
            "step.provisioning_layer_1_done" => Self::StepProvisioningLayer1Done,
            "step.provisioning_layer_2_done" => Self::StepProvisioningLayer2Done,
            "step.provisioning_layer_3_done" => Self::StepProvisioningLayer3Done,
            "step.helm_install_done" => Self::StepHelmInstallDone,
            "step.healthcheck_ok" => Self::StepHealthcheckOk,
            "step.catalog_seeded" => Self::StepCatalogSeeded,
            "step.auth_initialized" => Self::StepAuthInitialized,
            "step.first_classify_ok" => Self::StepFirstClassifyOk,
            "step.first_classify_correct" => Self::StepFirstClassifyCorrect,
            "session.abandoned" => Self::SessionAbandoned,
            "session.completed" => Self::SessionCompleted,
            "system.session_expired" => Self::SystemSessionExpired,
            "system.rollback_triggered" => Self::SystemRollbackTriggered,
            _ => return None,
        })
    }

    /// Renvoie tous les event_kind canoniques. Utilisé par le test
    /// `event_kind_whitelist_matches_sql_check`.
    #[must_use]
    pub const fn all() -> &'static [FunnelEventKind; 20] {
        &[
            Self::SessionStarted,
            Self::StepSignupFormOpen,
            Self::StepSignupFormSubmit,
            Self::StepEmailVerified,
            Self::StepPaymentAuthorized,
            Self::StepPlanSelected,
            Self::StepProvisioningStarted,
            Self::StepProvisioningLayer1Done,
            Self::StepProvisioningLayer2Done,
            Self::StepProvisioningLayer3Done,
            Self::StepHelmInstallDone,
            Self::StepHealthcheckOk,
            Self::StepCatalogSeeded,
            Self::StepAuthInitialized,
            Self::StepFirstClassifyOk,
            Self::StepFirstClassifyCorrect,
            Self::SessionAbandoned,
            Self::SessionCompleted,
            Self::SystemSessionExpired,
            Self::SystemRollbackTriggered,
        ]
    }
}

/// Acteur de l'event (= source de l'émission).
///
/// Doit matcher la CHECK constraint SQL `actor IN
/// ('user','system','terraform','helm','auth','timer')`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FunnelActor {
    /// Émis par l'UI portail (= action user).
    User,
    /// Émis par un service interne EFE.
    System,
    /// Émis par le runner Terraform.
    Terraform,
    /// Émis par Helm install.
    Helm,
    /// Émis par le service d'authentification.
    Auth,
    /// Émis par un cron timer.
    Timer,
}

impl FunnelActor {
    /// Stable string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::System => "system",
            Self::Terraform => "terraform",
            Self::Helm => "helm",
            Self::Auth => "auth",
            Self::Timer => "timer",
        }
    }
}

/// Row complet d'`onboarding_events`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardingEvent {
    /// UUID unique.
    pub event_id: Uuid,
    /// Session associée.
    pub session_id: OnboardingSessionId,
    /// Type d'event.
    pub event_kind: FunnelEventKind,
    /// Horodatage.
    pub occurred_at: DateTime<Utc>,
    /// Métadonnées libres (JSONB).
    pub metadata: serde_json::Value,
    /// `correlation_id` (= OTel trace_id du request).
    pub correlation_id: Uuid,
    /// Source de l'émission.
    pub actor: FunnelActor,
    /// Code d'erreur si l'event correspond à un échec.
    pub error_class: Option<ErrorClass>,
}

// ====================================================================
// OnboardingError — erreurs publiques côté handlers.
// ====================================================================

/// Erreurs publiques retournées par les handlers P2.
#[derive(Debug, thiserror::Error)]
pub enum OnboardingError {
    /// Le `event_kind` du payload WebSocket n'est pas dans la
    /// whitelist canonique.
    #[error("unknown event_kind: `{0}` (cf. FunnelEventKind whitelist)")]
    UnknownEventKind(String),

    /// L'`actor` du payload n'est pas dans la whitelist.
    #[error("unknown actor: `{0}` (must be user/system/terraform/helm/auth/timer)")]
    UnknownActor(String),

    /// La session référencée n'existe pas.
    #[error("session not found: `{0}`")]
    SessionNotFound(OnboardingSessionId),

    /// Tentative d'émettre un event pour une session déjà
    /// `completed_at` setté.
    #[error("session `{0}` already completed (no more events accepted)")]
    SessionAlreadyCompleted(OnboardingSessionId),

    /// La transition d'état n'est pas autorisée (e.g. on essaie de
    /// faire `first_classify_correct` avant `first_classify_ok`).
    #[error("invalid transition: cannot apply `{kind}` to session `{session}` in current state")]
    InvalidTransition {
        /// Session id.
        session: OnboardingSessionId,
        /// Event kind invalide.
        kind: &'static str,
    },

    /// Payload mal formé (JSON invalide, fields manquants).
    #[error("malformed payload: {0}")]
    MalformedPayload(String),

    /// Erreur DB (= persistance event ou session échec).
    #[error("DB error: {0}")]
    Db(String),

    /// Erreur signature HMAC (clé absente, format invalide).
    #[error("HMAC signature error: {0}")]
    Hmac(String),
}

impl OnboardingError {
    /// HTTP status code recommandé pour cette erreur.
    #[must_use]
    pub const fn http_status_code(&self) -> u16 {
        match self {
            Self::UnknownEventKind(_)
            | Self::UnknownActor(_)
            | Self::MalformedPayload(_)
            | Self::InvalidTransition { .. } => 400,
            Self::SessionNotFound(_) => 404,
            Self::SessionAlreadyCompleted(_) => 409,
            Self::Db(_) | Self::Hmac(_) => 500,
        }
    }

    /// Code stable RFC 7807 (`type` field).
    #[must_use]
    pub const fn rfc7807_type(&self) -> &'static str {
        match self {
            Self::UnknownEventKind(_) => "invalid_event_kind",
            Self::UnknownActor(_) => "invalid_actor",
            Self::SessionNotFound(_) => "session_not_found",
            Self::SessionAlreadyCompleted(_) => "session_already_completed",
            Self::InvalidTransition { .. } => "invalid_transition",
            Self::MalformedPayload(_) => "malformed_payload",
            Self::Db(_) => "internal_db",
            Self::Hmac(_) => "internal_hmac",
        }
    }
}

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_kind_count_matches_sql_check_whitelist() {
        // La SQL CHECK constraint de la migration 0028 liste 20
        // valeurs ; l'enum Rust doit en avoir exactement 20 pour
        // garantir la cohérence DB ↔ code.
        assert_eq!(FunnelEventKind::all().len(), 20);
    }

    #[test]
    fn event_kind_round_trip_through_string() {
        for k in *FunnelEventKind::all() {
            let s = k.as_str();
            let parsed = FunnelEventKind::parse(s).expect("must round-trip");
            assert_eq!(k, parsed, "round-trip fail on {:?} ↔ {s}", k);
        }
    }

    #[test]
    fn event_kind_labels_are_distinct() {
        let mut labels: Vec<&str> = FunnelEventKind::all().iter().map(|k| k.as_str()).collect();
        labels.sort();
        labels.dedup();
        assert_eq!(labels.len(), 20);
    }

    #[test]
    fn event_kind_unknown_returns_none() {
        assert!(FunnelEventKind::parse("nope.invalid").is_none());
        assert!(FunnelEventKind::parse("").is_none());
        assert!(FunnelEventKind::parse("SESSION.STARTED").is_none()); // case-sensitive
    }

    #[test]
    fn outcome_classification_by_session_state() {
        let mut s = OnboardingSession::new(
            OnboardingSessionId::new_v4(),
            Uuid::new_v4(),
            PlanTarget::Poc,
            Some(CloudTarget::Aws),
            Some("us-east-1".into()),
        );
        assert!(!s.is_completed());
        assert_eq!(s.outcome(), None);

        // Success path
        s.first_classify_correct_at = Some(Utc::now());
        s.completed_at = Some(Utc::now());
        assert_eq!(s.outcome(), Some(Outcome::Success));
    }

    #[test]
    fn outcome_abandoned_no_error() {
        let mut s = OnboardingSession::new(
            OnboardingSessionId::new_v4(),
            Uuid::new_v4(),
            PlanTarget::Sandbox,
            None,
            None,
        );
        s.abandoned_at = Some(Utc::now());
        s.completed_at = Some(Utc::now());
        assert_eq!(s.outcome(), Some(Outcome::Abandoned));
    }

    #[test]
    fn outcome_error_has_reason() {
        let mut s = OnboardingSession::new(
            OnboardingSessionId::new_v4(),
            Uuid::new_v4(),
            PlanTarget::Poc,
            Some(CloudTarget::Gcp),
            None,
        );
        s.abandoned_at = Some(Utc::now());
        s.abandoned_reason = Some(ErrorClass::CloudQuotaExceeded);
        s.completed_at = Some(Utc::now());
        assert_eq!(s.outcome(), Some(Outcome::Error));
    }

    #[test]
    fn elapsed_seconds_is_non_negative() {
        let s = OnboardingSession::new(
            OnboardingSessionId::new_v4(),
            Uuid::new_v4(),
            PlanTarget::Poc,
            Some(CloudTarget::Aws),
            None,
        );
        assert!(s.elapsed_seconds() >= 0);
    }

    #[test]
    fn error_http_status_codes_are_in_valid_ranges() {
        let errors: Vec<OnboardingError> = vec![
            OnboardingError::UnknownEventKind("x".into()),
            OnboardingError::SessionNotFound(OnboardingSessionId::new_v4()),
            OnboardingError::SessionAlreadyCompleted(OnboardingSessionId::new_v4()),
            OnboardingError::InvalidTransition {
                session: OnboardingSessionId::new_v4(),
                kind: "step.x",
            },
            OnboardingError::MalformedPayload("x".into()),
            OnboardingError::Db("x".into()),
        ];
        for e in errors {
            let code = e.http_status_code();
            assert!(
                (400..=599).contains(&code),
                "{e}: invalid http status {code}"
            );
        }
    }

    #[test]
    fn error_rfc7807_types_are_distinct() {
        let types = [
            OnboardingError::UnknownEventKind("x".into()).rfc7807_type(),
            OnboardingError::UnknownActor("x".into()).rfc7807_type(),
            OnboardingError::SessionNotFound(OnboardingSessionId::new_v4()).rfc7807_type(),
            OnboardingError::SessionAlreadyCompleted(OnboardingSessionId::new_v4()).rfc7807_type(),
            OnboardingError::InvalidTransition {
                session: OnboardingSessionId::new_v4(),
                kind: "x",
            }
            .rfc7807_type(),
            OnboardingError::MalformedPayload("x".into()).rfc7807_type(),
            OnboardingError::Db("x".into()).rfc7807_type(),
            OnboardingError::Hmac("x".into()).rfc7807_type(),
        ];
        let mut sorted: Vec<&str> = types.to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 8, "all rfc7807 types must be distinct");
    }
}
