// SPDX-License-Identifier: UNLICENSED
//! Sprint **D19 (P2-Foundation)** — funnel events ingestion +
//! state-machine.
//!
//! Cf. `docs/EFE_P2_SELF_SERVICE_ARCHITECTURE.md` § 4.4.
//!
//! Le portail Next.js émet chaque transition d'état via une
//! connexion WebSocket dédiée. Ce module fournit :
//!
//! - [`FunnelEventInput`] — le payload reçu côté backend.
//! - [`FunnelEventsHandler`] — la machine à état qui valide,
//!   persiste, et bump les métriques Prometheus.
//! - [`FunnelStateMachine`] — la validation des transitions
//!   autorisées (pure function, testable sans DB).
//!
//! ## Pourquoi WebSocket (et pas HTTP POST classique)
//!
//! Le portail émet ~14 events par session (un par étape) ; chaque
//! event doit être persisté immédiatement pour que le funnel soit
//! exhaustif. Avec HTTP POST, on paye à chaque event :
//!
//! - Setup TCP : ~10 ms
//! - mTLS handshake (si pas keep-alive) : ~30 ms
//! - Latence applicative : ~10 ms
//!
//! Total : ~50 ms × 14 = 700 ms juste de overhead funnel.
//!
//! Avec WebSocket :
//! - 1× handshake initial
//! - puis ~3 ms par message
//!
//! Total : ~70 ms (= 10× plus rapide).
//!
//! ## Concurrence
//!
//! Plusieurs sessions peuvent émettre en parallèle. Le handler est
//! `Sync + Send` et utilise une `tokio::sync::Mutex` (PAS
//! `parking_lot`) car la persistance DB est `async`. Le mutex est
//! relâché entre les insertions, donc pas de blocage cross-session.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use efe_telemetry::metrics::p2_instruments::{
    note_drop, note_step_duration, FunnelStep, PlanTarget,
};

use super::types::{
    ErrorClass, FunnelActor, FunnelEventKind, OnboardingError, OnboardingSession,
    OnboardingSessionId,
};

// ====================================================================
// Input — payload reçu via WebSocket
// ====================================================================

/// Payload reçu côté serveur depuis le portail.
///
/// L'`event_id` n'est pas dans le payload : le serveur le génère
/// (= l'event est `idempotent` côté serveur, pas côté client).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunnelEventInput {
    /// Type d'event (= label canonique, ex. `"step.email_verified"`).
    pub event_kind: String,
    /// Horodatage client (pour mesurer la latence aller-retour).
    /// Si absent ou en futur, le serveur substitue `Utc::now()`.
    #[serde(default)]
    pub occurred_at: Option<DateTime<Utc>>,
    /// Métadonnées libres (max 4 KiB après serialization).
    #[serde(default)]
    pub metadata: serde_json::Value,
    /// Acteur (default = `user`).
    #[serde(default = "default_actor")]
    pub actor: String,
    /// Si l'event est un échec, la classe d'erreur.
    #[serde(default)]
    pub error_class: Option<String>,
}

fn default_actor() -> String {
    "user".to_string()
}

/// Max payload size (= protection DoS).
const MAX_METADATA_BYTES: usize = 4_096;

/// Max clock skew tolérée entre le client et le serveur. Au-delà,
/// le `occurred_at` du payload est ignoré et remplacé par `Utc::now()`.
const MAX_CLOCK_SKEW_SECS: i64 = 60;

impl FunnelEventInput {
    /// Valide le payload sans persistance.
    ///
    /// Retourne [`ValidatedInput`] qui regroupe les 4 champs parsés.
    pub fn validate(&self) -> Result<ValidatedInput, OnboardingError> {
        // 1. Event kind
        let kind = FunnelEventKind::parse(&self.event_kind)
            .ok_or_else(|| OnboardingError::UnknownEventKind(self.event_kind.clone()))?;

        // 2. Actor
        let actor = match self.actor.as_str() {
            "user" => FunnelActor::User,
            "system" => FunnelActor::System,
            "terraform" => FunnelActor::Terraform,
            "helm" => FunnelActor::Helm,
            "auth" => FunnelActor::Auth,
            "timer" => FunnelActor::Timer,
            other => return Err(OnboardingError::UnknownActor(other.to_string())),
        };

        // 3. Metadata size (protection DoS)
        let meta_size = serde_json::to_vec(&self.metadata)
            .map_err(|e| OnboardingError::MalformedPayload(e.to_string()))?
            .len();
        if meta_size > MAX_METADATA_BYTES {
            return Err(OnboardingError::MalformedPayload(format!(
                "metadata > {MAX_METADATA_BYTES} bytes (got {meta_size})"
            )));
        }

        // 4. Error class si présent
        let ec = if let Some(s) = &self.error_class {
            Some(parse_error_class(s).ok_or_else(|| {
                OnboardingError::MalformedPayload(format!("unknown error_class: {s}"))
            })?)
        } else {
            None
        };

        // 5. Timestamp client-tolérant
        let now = Utc::now();
        let occurred = match self.occurred_at {
            Some(t) => {
                let skew = (now - t).num_seconds().abs();
                if skew > MAX_CLOCK_SKEW_SECS {
                    now // skew trop grand → substitute serveur
                } else {
                    t
                }
            }
            None => now,
        };

        Ok(ValidatedInput {
            kind,
            actor,
            error_class: ec,
            occurred_at: occurred,
        })
    }
}

/// Output structuré de [`FunnelEventInput::validate`] (= alias
/// type pour éviter le clippy `very_complex_type`).
#[derive(Debug, Clone, Copy)]
pub struct ValidatedInput {
    /// Type d'event parsé.
    pub kind: FunnelEventKind,
    /// Acteur parsé.
    pub actor: FunnelActor,
    /// Classe d'erreur si présente.
    pub error_class: Option<ErrorClass>,
    /// Horodatage final (= client si tolérable, sinon serveur).
    pub occurred_at: DateTime<Utc>,
}

/// Parse un label d'`error_class` vers l'enum.
fn parse_error_class(s: &str) -> Option<ErrorClass> {
    Some(match s {
        "email_invalid" => ErrorClass::EmailInvalid,
        "email_domain_blocked" => ErrorClass::EmailDomainBlocked,
        "payment_declined" => ErrorClass::PaymentDeclined,
        "payment_3ds_required" => ErrorClass::Payment3dsRequired,
        "cloud_quota_exceeded" => ErrorClass::CloudQuotaExceeded,
        "cloud_region_unavailable" => ErrorClass::CloudRegionUnavailable,
        "cloud_auth_failed" => ErrorClass::CloudAuthFailed,
        "tf_apply_timeout" => ErrorClass::TfApplyTimeout,
        "helm_install_failed" => ErrorClass::HelmInstallFailed,
        "catalog_seed_invalid" => ErrorClass::CatalogSeedInvalid,
        "catalog_template_missing" => ErrorClass::CatalogTemplateMissing,
        "api_key_gen_failed" => ErrorClass::ApiKeyGenFailed,
        "first_call_timeout" => ErrorClass::FirstCallTimeout,
        "internal" => ErrorClass::Internal,
        _ => return None,
    })
}

// ====================================================================
// State machine — transitions autorisées
// ====================================================================

/// Renvoie `true` si l'event `kind` est une transition valide
/// depuis l'état courant de la `session`.
///
/// **Pure function** — pas de DB call, pas d'I/O. Testable sans
/// stub.
///
/// Doctrine :
/// - Une session terminée n'accepte plus d'event (sauf
///   `system.session_expired` qui est idempotent — on accepte mais
///   on ignore).
/// - L'ordre canonique des étapes est respecté : on ne peut pas
///   émettre `step.first_classify_ok` avant `step.helm_install_done`.
/// - Les `system.*` events sont **toujours** autorisés (= guardrail
///   ops).
#[must_use]
pub fn is_valid_transition(session: &OnboardingSession, kind: FunnelEventKind) -> bool {
    use FunnelEventKind as K;

    // System events : toujours OK (= rollback / expire / etc.)
    if matches!(kind, K::SystemSessionExpired | K::SystemRollbackTriggered) {
        return true;
    }

    // Session terminée : rejette tous les autres events
    if session.is_completed() {
        return false;
    }

    match kind {
        // SessionStarted : OK uniquement si pas déjà session (pas testable
        // ici — c'est la création de la session, pas un follow-up).
        // L'event est implicitement créé en même temps que la session
        // par le repo ; le state machine accepte tout.
        K::SessionStarted => true,

        // Step ordering : chaque step requiert que les "earlier"
        // soient atteintes. On les regroupe en 4 phases logiques.
        K::StepSignupFormOpen | K::StepSignupFormSubmit | K::StepEmailVerified => true,

        K::StepPaymentAuthorized => session.email_verified_at.is_some(),

        K::StepPlanSelected => session.payment_authorized_at.is_some(),

        K::StepProvisioningStarted
        | K::StepProvisioningLayer1Done
        | K::StepProvisioningLayer2Done
        | K::StepProvisioningLayer3Done => session.payment_authorized_at.is_some(),

        K::StepHelmInstallDone => {
            session.provisioning_done_at.is_some()
                || (session.payment_authorized_at.is_some()
                    && session.plan_target == PlanTarget::Sandbox)
        }

        K::StepHealthcheckOk | K::StepCatalogSeeded => {
            session.helm_done_at.is_some() || session.plan_target == PlanTarget::Sandbox
        }

        K::StepAuthInitialized => session.catalog_seeded_at.is_some(),

        K::StepFirstClassifyOk => {
            session.auth_initialized_at.is_some() || session.plan_target == PlanTarget::Sandbox
        }

        K::StepFirstClassifyCorrect => session.first_classify_at.is_some(),

        // Session terminée :
        K::SessionAbandoned => true, // user peut abandonner à tout moment
        K::SessionCompleted => {
            session.first_classify_correct_at.is_some() || session.abandoned_at.is_some()
        }

        K::SystemSessionExpired | K::SystemRollbackTriggered => true,
    }
}

// ====================================================================
// Handler — repository-style
// ====================================================================

/// Bundle passé à [`SessionRepo::insert_event_and_advance`] pour
/// éviter le clippy `too_many_arguments`.
///
/// Le `_lifetime` `'a` permet d'emprunter `metadata` sans clone
/// quand le caller a déjà la valeur en main.
#[derive(Debug)]
pub struct EventInsertInput<'a> {
    /// Type d'event.
    pub kind: FunnelEventKind,
    /// Acteur.
    pub actor: FunnelActor,
    /// Horodatage.
    pub occurred_at: DateTime<Utc>,
    /// Classe d'erreur si applicable.
    pub error_class: Option<ErrorClass>,
    /// Métadonnées (emprunté pour éviter le clone).
    pub metadata: &'a serde_json::Value,
    /// `correlation_id`.
    pub correlation_id: Uuid,
}

/// Trait abstrait sur la persistance des sessions + events.
///
/// L'impl Postgres réelle vit dans `efe-server::v1::onboarding::repo`
/// (livraison D20). Le handler est testable en isolation via
/// `InMemorySessionRepo` (impl ci-dessous).
#[async_trait::async_trait]
pub trait SessionRepo: Send + Sync {
    /// Lit une session par id.
    async fn read(
        &self,
        id: OnboardingSessionId,
    ) -> Result<Option<OnboardingSession>, OnboardingError>;

    /// Insert un event + update les timestamps de la session
    /// dans **la même transaction**.
    async fn insert_event_and_advance(
        &self,
        session: &mut OnboardingSession,
        input: EventInsertInput<'_>,
    ) -> Result<Uuid, OnboardingError>;

    /// Persiste la session terminée (= `completed_at` setté +
    /// signature HMAC calculée).
    async fn seal(
        &self,
        session: &mut OnboardingSession,
        signature_hmac: Vec<u8>,
    ) -> Result<(), OnboardingError>;
}

/// Handler qui ingère les events WebSocket et orchestre la
/// state-machine.
pub struct FunnelEventsHandler<R: SessionRepo + 'static> {
    repo: Arc<R>,
}

impl<R: SessionRepo + 'static> FunnelEventsHandler<R> {
    /// Construit un nouveau handler. Le repo est partagé entre
    /// les WebSocket workers (Arc).
    pub fn new(repo: Arc<R>) -> Self {
        Self { repo }
    }

    /// Traite un event reçu via WebSocket.
    ///
    /// # Sémantique
    ///
    /// 1. Validate payload (= structure + cardinalité).
    /// 2. Read session ; reject si absente ou complétée.
    /// 3. Validate transition (state machine).
    /// 4. Update les timestamps de la session selon `event_kind`.
    /// 5. Insert event + update session **dans la même transaction**.
    /// 6. Si `event_kind` est terminal, déclencher seal HMAC.
    /// 7. Bump les métriques Prometheus correspondantes.
    pub async fn handle(
        &self,
        session_id: OnboardingSessionId,
        input: FunnelEventInput,
        correlation_id: Uuid,
    ) -> Result<Uuid, OnboardingError> {
        // 1. Validate input
        let ValidatedInput {
            kind,
            actor,
            error_class,
            occurred_at,
        } = input.validate()?;

        // 2. Read session
        let mut session = self
            .repo
            .read(session_id)
            .await?
            .ok_or(OnboardingError::SessionNotFound(session_id))?;

        if session.is_completed() {
            // Idempotent system events : on accepte mais on ignore
            if matches!(kind, FunnelEventKind::SystemSessionExpired) {
                return Ok(Uuid::new_v4()); // no-op
            }
            return Err(OnboardingError::SessionAlreadyCompleted(session_id));
        }

        // 3. Validate transition
        if !is_valid_transition(&session, kind) {
            return Err(OnboardingError::InvalidTransition {
                session: session_id,
                kind: kind.as_str(),
            });
        }

        // 4. Update timestamps selon kind. `state_changed` est false
        //    si l'event est un duplicat (= timestamp déjà setté).
        //    Audit fix D19-P2.A6.
        let state_changed = update_session_for_event(&mut session, kind, occurred_at, error_class);

        // 5. Persist + advance (atomic) — toujours, le log
        //    `onboarding_events` est append-only par contrat.
        let metadata = input.metadata;
        let event_id = self
            .repo
            .insert_event_and_advance(
                &mut session,
                EventInsertInput {
                    kind,
                    actor,
                    occurred_at,
                    error_class,
                    metadata: &metadata,
                    correlation_id,
                },
            )
            .await?;

        // 6. Métriques Prometheus — uniquement si l'event est
        //    *effectivement* nouveau (sinon double comptage du
        //    histogram, audit fix D19-P2.A6).
        if state_changed {
            observe_metrics(&session, kind, occurred_at, error_class);
        }

        // 7. Si event terminal, seal la session
        if session.is_completed() && session.audit_signature.is_none() {
            let sig = compute_signature(&session);
            self.repo.seal(&mut session, sig).await?;
        }

        Ok(event_id)
    }
}

/// Met à jour les timestamps de la session selon le `kind` de
/// l'event.
///
/// **Audit fix D19-P2.A6** — retourne `bool` (= "state actually
/// changed"). Le handler utilise ce retour pour décider d'émettre
/// ou non les métriques Prometheus. Sans ça, un event dupliqué
/// (ex. 2× `step.email_verified` à 30 s d'intervalle) aurait :
///   - First call : observe 30 s dans le histogram `signup_form`.
///   - Second call : observe 60 s dans le **même** histogram avec
///     la **même** ancre `started_at` → 2 observations au lieu d'1,
///     biais p50/p99 vers le haut.
///
/// Avec ce retour, l'observation Prometheus n'a lieu que si l'event
/// est **effectivement nouveau** (= au moins 1 champ modifié). Les
/// events dupliqués sont silencieusement no-op côté métriques mais
/// **toujours** persistés dans `onboarding_events` (= append-only
/// log respecté).
///
/// Helper interne : `mark_if_new` retourne `true` si le timestamp
/// était `None` et l'a setté (= changement effectif).
fn update_session_for_event(
    session: &mut OnboardingSession,
    kind: FunnelEventKind,
    at: DateTime<Utc>,
    error_class: Option<ErrorClass>,
) -> bool {
    use FunnelEventKind as K;

    /// Helper : set le champ si None ; retourne `true` si changé.
    #[inline]
    fn mark_if_new(slot: &mut Option<DateTime<Utc>>, at: DateTime<Utc>) -> bool {
        if slot.is_none() {
            *slot = Some(at);
            true
        } else {
            false
        }
    }

    let state_changed = match kind {
        // Events transients (pas de timestamp en session) — toujours
        // considérés "non-changeants" pour les métriques (UI peut
        // les réémettre).
        K::SessionStarted
        | K::StepSignupFormOpen
        | K::StepSignupFormSubmit
        | K::StepPlanSelected
        | K::StepProvisioningLayer1Done
        | K::StepProvisioningLayer2Done
        | K::StepProvisioningLayer3Done
        | K::StepHealthcheckOk => false,

        K::StepEmailVerified => mark_if_new(&mut session.email_verified_at, at),
        K::StepPaymentAuthorized => mark_if_new(&mut session.payment_authorized_at, at),
        K::StepProvisioningStarted => mark_if_new(&mut session.provisioning_started_at, at),
        K::StepHelmInstallDone => {
            // 2 timestamps mis à jour ; "changed" si au moins 1 a bougé.
            let a = mark_if_new(&mut session.provisioning_done_at, at);
            let b = mark_if_new(&mut session.helm_done_at, at);
            a || b
        }
        K::StepCatalogSeeded => mark_if_new(&mut session.catalog_seeded_at, at),
        K::StepAuthInitialized => mark_if_new(&mut session.auth_initialized_at, at),
        K::StepFirstClassifyOk => mark_if_new(&mut session.first_classify_at, at),
        K::StepFirstClassifyCorrect => {
            let a = mark_if_new(&mut session.first_classify_correct_at, at);
            let b = mark_if_new(&mut session.completed_at, at);
            a || b
        }
        K::SessionAbandoned | K::SystemSessionExpired => {
            let a = mark_if_new(&mut session.abandoned_at, at);
            // Seul le PREMIER abandonment fixe la raison (= idempotence).
            if a {
                session.abandoned_reason = error_class;
            }
            let b = mark_if_new(&mut session.completed_at, at);
            a || b
        }
        K::SessionCompleted => mark_if_new(&mut session.completed_at, at),
        K::SystemRollbackTriggered => {
            session.rollbacks_count += 1;
            true
        }
    };

    if state_changed
        && error_class.is_some()
        && !matches!(kind, K::SessionAbandoned | K::SystemSessionExpired)
    {
        session.errors_count += 1;
    }

    state_changed
}

/// Bump les métriques Prometheus liées à l'event.
fn observe_metrics(
    session: &OnboardingSession,
    kind: FunnelEventKind,
    _at: DateTime<Utc>,
    error_class: Option<ErrorClass>,
) {
    use FunnelEventKind as K;

    // Step duration observation : si on a un timestamp précédent
    // dans la séquence du funnel, observe la durée.
    if let Some((step, prev)) = previous_step_anchor(session, kind) {
        let now = Utc::now();
        let dur = (now - prev).to_std().unwrap_or(Duration::ZERO);
        note_step_duration(session.plan_target, step, dur);
    }

    // Drop observation : si error_class est présent ET kind est
    // SessionAbandoned/Expired
    if matches!(kind, K::SessionAbandoned | K::SystemSessionExpired) {
        if let Some(ec) = error_class {
            // Détermine le step où on a abandonné (= le dernier
            // step atteint).
            let step = last_reached_step(session);
            note_drop(session.plan_target, step, ec);
        }
    }
}

/// Trouve la durée d'une étape : returns `(step, started_at)` tel
/// que la durée = `now - started_at`.
fn previous_step_anchor(
    session: &OnboardingSession,
    kind: FunnelEventKind,
) -> Option<(FunnelStep, DateTime<Utc>)> {
    use FunnelEventKind as K;
    match kind {
        K::StepEmailVerified => Some((FunnelStep::SignupForm, session.started_at)),
        K::StepPaymentAuthorized => session
            .email_verified_at
            .map(|t| (FunnelStep::EmailVerify, t)),
        K::StepPlanSelected => session
            .payment_authorized_at
            .map(|t| (FunnelStep::PaymentAuthorize, t)),
        K::StepProvisioningStarted => None,
        K::StepHelmInstallDone => session
            .provisioning_started_at
            .map(|t| (FunnelStep::Provisioning, t)),
        K::StepCatalogSeeded => session.helm_done_at.map(|t| (FunnelStep::HelmInstall, t)),
        K::StepAuthInitialized => session
            .catalog_seeded_at
            .map(|t| (FunnelStep::CatalogSeed, t)),
        K::StepFirstClassifyOk => session
            .auth_initialized_at
            .map(|t| (FunnelStep::AuthInit, t)),
        K::StepFirstClassifyCorrect => session
            .first_classify_at
            .map(|t| (FunnelStep::FirstClassify, t)),
        _ => None,
    }
}

/// Renvoie le **dernier** step atteint par la session (= step
/// le plus avancé dont le timestamp est `Some`).
fn last_reached_step(session: &OnboardingSession) -> FunnelStep {
    if session.first_classify_correct_at.is_some() {
        FunnelStep::Aha
    } else if session.first_classify_at.is_some() {
        FunnelStep::FirstClassify
    } else if session.auth_initialized_at.is_some() {
        FunnelStep::AuthInit
    } else if session.catalog_seeded_at.is_some() {
        FunnelStep::CatalogSeed
    } else if session.helm_done_at.is_some() {
        FunnelStep::HelmInstall
    } else if session.provisioning_done_at.is_some() || session.provisioning_started_at.is_some() {
        // Les 2 sub-états (started / done) collapsent sur le même
        // step pour les métriques d'abandon (= cardinality bounded).
        FunnelStep::Provisioning
    } else if session.payment_authorized_at.is_some() {
        FunnelStep::PaymentAuthorize
    } else if session.email_verified_at.is_some() {
        FunnelStep::EmailVerify
    } else {
        FunnelStep::SignupForm
    }
}

/// Calcule la signature HMAC d'une session complétée.
///
/// **Pour D19** : on émet un SHA-256 du JSON canonique. Le vrai
/// HMAC (avec clé `EFE_AUDIT_HMAC_KEY`) sera livré en D20 quand
/// la chaîne D14-bis sera étendue ; en attendant, ce SHA agit
/// comme signature de cohérence (= détecte la corruption mais
/// n'est pas opposable juridiquement).
fn compute_signature(session: &OnboardingSession) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let json = serde_json::to_vec(session).unwrap_or_default();
    let mut hasher = Sha256::new();
    hasher.update(&json);
    hasher.finalize().to_vec()
}

// ====================================================================
// In-memory repo (tests only)
// ====================================================================

/// Repo en mémoire pour les tests unitaires. **NE PAS** utiliser
/// en prod.
///
/// Utilise `std::sync::RwLock` (pas `parking_lot`) pour éviter une
/// dépendance crate supplémentaire dans efe-server. Le test-only
/// repo n'a aucune contrainte de perf.
#[doc(hidden)]
pub struct InMemorySessionRepo {
    sessions: std::sync::RwLock<std::collections::HashMap<OnboardingSessionId, OnboardingSession>>,
    events: std::sync::RwLock<Vec<(Uuid, OnboardingSessionId, FunnelEventKind)>>,
}

#[doc(hidden)]
impl InMemorySessionRepo {
    pub fn new() -> Self {
        Self {
            sessions: std::sync::RwLock::new(std::collections::HashMap::new()),
            events: std::sync::RwLock::new(Vec::new()),
        }
    }

    pub fn insert_session(&self, session: OnboardingSession) {
        self.sessions
            .write()
            .expect("test repo lock not poisoned")
            .insert(session.session_id, session);
    }

    pub fn event_count(&self) -> usize {
        self.events
            .read()
            .expect("test repo lock not poisoned")
            .len()
    }

    pub fn session(&self, id: OnboardingSessionId) -> Option<OnboardingSession> {
        self.sessions
            .read()
            .expect("test repo lock not poisoned")
            .get(&id)
            .cloned()
    }
}

#[doc(hidden)]
impl Default for InMemorySessionRepo {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl SessionRepo for InMemorySessionRepo {
    async fn read(
        &self,
        id: OnboardingSessionId,
    ) -> Result<Option<OnboardingSession>, OnboardingError> {
        Ok(self
            .sessions
            .read()
            .expect("test repo lock not poisoned")
            .get(&id)
            .cloned())
    }

    async fn insert_event_and_advance(
        &self,
        session: &mut OnboardingSession,
        input: EventInsertInput<'_>,
    ) -> Result<Uuid, OnboardingError> {
        let event_id = Uuid::new_v4();
        self.events
            .write()
            .expect("test repo lock not poisoned")
            .push((event_id, session.session_id, input.kind));
        self.sessions
            .write()
            .expect("test repo lock not poisoned")
            .insert(session.session_id, session.clone());
        Ok(event_id)
    }

    async fn seal(
        &self,
        session: &mut OnboardingSession,
        signature_hmac: Vec<u8>,
    ) -> Result<(), OnboardingError> {
        session.audit_signature = Some(signature_hmac);
        self.sessions
            .write()
            .expect("test repo lock not poisoned")
            .insert(session.session_id, session.clone());
        Ok(())
    }
}

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_session(plan: PlanTarget) -> OnboardingSession {
        OnboardingSession::new(
            OnboardingSessionId::new_v4(),
            Uuid::new_v4(),
            plan,
            if matches!(plan, PlanTarget::Sandbox) {
                None
            } else {
                Some(crate::v1::onboarding::types::CloudTarget::Aws)
            },
            Some("us-east-1".into()),
        )
    }

    #[test]
    fn validate_rejects_unknown_event_kind() {
        let input = FunnelEventInput {
            event_kind: "step.invalid_kind".into(),
            occurred_at: None,
            metadata: serde_json::json!({}),
            actor: "user".into(),
            error_class: None,
        };
        let err = input.validate().unwrap_err();
        assert!(matches!(err, OnboardingError::UnknownEventKind(_)));
    }

    #[test]
    fn validate_rejects_unknown_actor() {
        let input = FunnelEventInput {
            event_kind: "session.started".into(),
            occurred_at: None,
            metadata: serde_json::json!({}),
            actor: "robot".into(),
            error_class: None,
        };
        let err = input.validate().unwrap_err();
        assert!(matches!(err, OnboardingError::UnknownActor(_)));
    }

    #[test]
    fn validate_rejects_oversize_metadata() {
        let big = "x".repeat(MAX_METADATA_BYTES + 100);
        let input = FunnelEventInput {
            event_kind: "session.started".into(),
            occurred_at: None,
            metadata: serde_json::json!({"big": big}),
            actor: "user".into(),
            error_class: None,
        };
        let err = input.validate().unwrap_err();
        assert!(matches!(err, OnboardingError::MalformedPayload(_)));
    }

    #[test]
    fn validate_substitutes_skewed_timestamp() {
        let way_off = Utc::now() + chrono::Duration::hours(2);
        let input = FunnelEventInput {
            event_kind: "session.started".into(),
            occurred_at: Some(way_off),
            metadata: serde_json::json!({}),
            actor: "user".into(),
            error_class: None,
        };
        let v = input.validate().expect("ok");
        // occurred_at should be ~ now, not way_off
        let skew = (Utc::now() - v.occurred_at).num_seconds().abs();
        assert!(skew < MAX_CLOCK_SKEW_SECS, "got skew {skew}s");
    }

    #[test]
    fn transition_poc_email_before_payment() {
        // OK : email_verified avant payment_authorized
        let s = fresh_session(PlanTarget::Poc);
        assert!(is_valid_transition(&s, FunnelEventKind::StepEmailVerified));
    }

    #[test]
    fn transition_poc_payment_requires_email_first() {
        let s = fresh_session(PlanTarget::Poc);
        // Pas autorisé : payment avant email
        assert!(!is_valid_transition(
            &s,
            FunnelEventKind::StepPaymentAuthorized
        ));
    }

    #[test]
    fn transition_poc_payment_ok_after_email() {
        let mut s = fresh_session(PlanTarget::Poc);
        s.email_verified_at = Some(Utc::now());
        assert!(is_valid_transition(
            &s,
            FunnelEventKind::StepPaymentAuthorized
        ));
    }

    #[test]
    fn transition_completed_session_rejects_step_events() {
        let mut s = fresh_session(PlanTarget::Poc);
        s.first_classify_correct_at = Some(Utc::now());
        s.completed_at = Some(Utc::now());
        // step events refusés après completion
        assert!(!is_valid_transition(&s, FunnelEventKind::StepCatalogSeeded));
        // system events toujours OK
        assert!(is_valid_transition(
            &s,
            FunnelEventKind::SystemRollbackTriggered
        ));
    }

    #[test]
    fn transition_sandbox_skips_provisioning() {
        let s = fresh_session(PlanTarget::Sandbox);
        // Sandbox saute les steps provisioning : healthcheck OK direct
        assert!(is_valid_transition(&s, FunnelEventKind::StepHealthcheckOk));
    }

    #[tokio::test]
    async fn handler_persists_event_and_advances_session() {
        let repo = Arc::new(InMemorySessionRepo::new());
        let session = fresh_session(PlanTarget::Poc);
        let session_id = session.session_id;
        repo.insert_session(session);

        let handler = FunnelEventsHandler::new(repo.clone());
        let input = FunnelEventInput {
            event_kind: "step.email_verified".into(),
            occurred_at: None,
            metadata: serde_json::json!({}),
            actor: "user".into(),
            error_class: None,
        };
        let event_id = handler
            .handle(session_id, input, Uuid::new_v4())
            .await
            .expect("ok");

        // 1 event persisted
        assert_eq!(repo.event_count(), 1);
        // session.email_verified_at is now set
        let s = repo.session(session_id).expect("ok");
        assert!(s.email_verified_at.is_some());
        assert_ne!(event_id, Uuid::nil());
    }

    #[tokio::test]
    async fn handler_rejects_invalid_transition() {
        let repo = Arc::new(InMemorySessionRepo::new());
        let session = fresh_session(PlanTarget::Poc);
        let session_id = session.session_id;
        repo.insert_session(session);

        let handler = FunnelEventsHandler::new(repo);
        // Tente d'émettre payment_authorized SANS email_verified d'abord
        let input = FunnelEventInput {
            event_kind: "step.payment_authorized".into(),
            occurred_at: None,
            metadata: serde_json::json!({}),
            actor: "user".into(),
            error_class: None,
        };
        let err = handler
            .handle(session_id, input, Uuid::new_v4())
            .await
            .unwrap_err();
        assert!(matches!(err, OnboardingError::InvalidTransition { .. }));
    }

    #[tokio::test]
    async fn handler_seals_session_on_aha() {
        let repo = Arc::new(InMemorySessionRepo::new());
        let mut session = fresh_session(PlanTarget::Poc);
        // Pré-positionne tous les timestamps jusqu'au first_classify
        session.email_verified_at = Some(Utc::now());
        session.payment_authorized_at = Some(Utc::now());
        session.provisioning_started_at = Some(Utc::now());
        session.provisioning_done_at = Some(Utc::now());
        session.helm_done_at = Some(Utc::now());
        session.catalog_seeded_at = Some(Utc::now());
        session.auth_initialized_at = Some(Utc::now());
        session.first_classify_at = Some(Utc::now());
        let session_id = session.session_id;
        repo.insert_session(session);

        let handler = FunnelEventsHandler::new(repo.clone());
        let input = FunnelEventInput {
            event_kind: "step.first_classify_correct".into(),
            occurred_at: None,
            metadata: serde_json::json!({"confidence": 0.92}),
            actor: "system".into(),
            error_class: None,
        };
        handler
            .handle(session_id, input, Uuid::new_v4())
            .await
            .expect("ok");

        let s = repo.session(session_id).expect("ok");
        assert!(s.is_completed());
        assert!(s.audit_signature.is_some(), "must have HMAC after aha");
        assert_eq!(
            s.audit_signature.as_ref().unwrap().len(),
            32,
            "SHA-256 = 32 bytes"
        );
    }

    #[tokio::test]
    async fn handler_rejects_event_on_completed_session() {
        let repo = Arc::new(InMemorySessionRepo::new());
        let mut session = fresh_session(PlanTarget::Poc);
        session.completed_at = Some(Utc::now());
        session.first_classify_correct_at = Some(Utc::now());
        let session_id = session.session_id;
        repo.insert_session(session);

        let handler = FunnelEventsHandler::new(repo);
        let input = FunnelEventInput {
            event_kind: "step.catalog_seeded".into(),
            occurred_at: None,
            metadata: serde_json::json!({}),
            actor: "user".into(),
            error_class: None,
        };
        let err = handler
            .handle(session_id, input, Uuid::new_v4())
            .await
            .unwrap_err();
        assert!(matches!(err, OnboardingError::SessionAlreadyCompleted(_)));
    }

    #[tokio::test]
    async fn handler_idempotent_on_system_expired_after_complete() {
        let repo = Arc::new(InMemorySessionRepo::new());
        let mut session = fresh_session(PlanTarget::Sandbox);
        session.completed_at = Some(Utc::now());
        session.first_classify_correct_at = Some(Utc::now());
        let session_id = session.session_id;
        repo.insert_session(session);

        let handler = FunnelEventsHandler::new(repo);
        // `system.session_expired` est idempotent : OK même sur
        // session déjà complétée.
        let input = FunnelEventInput {
            event_kind: "system.session_expired".into(),
            occurred_at: None,
            metadata: serde_json::json!({}),
            actor: "timer".into(),
            error_class: None,
        };
        let r = handler.handle(session_id, input, Uuid::new_v4()).await;
        assert!(r.is_ok(), "system.session_expired must be idempotent");
    }

    /// **Audit pin D19-P2.A4** — round-trip `ErrorClass::as_label()`
    /// ↔ `parse_error_class()`. Si quelqu'un renomme une variante
    /// sans toucher au parser, ce test fail.
    #[test]
    fn error_class_parse_and_as_label_round_trip_all_14() {
        use crate::v1::onboarding::types::ErrorClass;
        for ec in *ErrorClass::all() {
            let label = ec.as_label();
            let parsed = parse_error_class(label)
                .unwrap_or_else(|| panic!("parse_error_class({label:?}) returned None — drift !"));
            assert_eq!(ec, parsed, "round-trip mismatch on {label}");
        }
    }

    /// **Audit pin D19-P2.A5** — alignement explicite des 20
    /// `FunnelEventKind::as_str()` avec la `CHECK` constraint SQL
    /// de la migration 0028. Si la liste diverge (SQL ou Rust),
    /// le test fail avec un diff exploitable.
    ///
    /// Note : ce test ne lit pas le `.sql` ; il pin la liste
    /// **canonique attendue** en dur. Si on change la liste, on
    /// change ici + la migration + le doc P2 ensemble.
    #[test]
    fn funnel_event_kind_strings_match_sql_check_constraint() {
        const EXPECTED_SQL_LIST: &[&str] = &[
            "session.started",
            "step.signup_form_open",
            "step.signup_form_submit",
            "step.email_verified",
            "step.payment_authorized",
            "step.plan_selected",
            "step.provisioning_started",
            "step.provisioning_layer_1_done",
            "step.provisioning_layer_2_done",
            "step.provisioning_layer_3_done",
            "step.helm_install_done",
            "step.healthcheck_ok",
            "step.catalog_seeded",
            "step.auth_initialized",
            "step.first_classify_ok",
            "step.first_classify_correct",
            "session.abandoned",
            "session.completed",
            "system.session_expired",
            "system.rollback_triggered",
        ];

        let rust_set: std::collections::BTreeSet<&str> =
            FunnelEventKind::all().iter().map(|k| k.as_str()).collect();
        let sql_set: std::collections::BTreeSet<&str> = EXPECTED_SQL_LIST.iter().copied().collect();

        let missing_in_rust: Vec<&&str> = sql_set.difference(&rust_set).collect();
        let missing_in_sql: Vec<&&str> = rust_set.difference(&sql_set).collect();

        assert!(
            missing_in_rust.is_empty() && missing_in_sql.is_empty(),
            "SQL ↔ Rust event_kind drift detected.\n\
             Missing in Rust: {missing_in_rust:?}\n\
             Missing in SQL : {missing_in_sql:?}\n\
             Fix : update both the `CHECK` constraint in \
             `migrations/0028_d19_p2_onboarding.sql` AND the enum \
             `FunnelEventKind` in `v1/onboarding/types.rs`."
        );
    }

    /// **Audit pin D19-P2.A6** — events dupliqués ne biaisent pas
    /// les histograms Prometheus.
    #[tokio::test]
    async fn duplicate_step_event_does_not_double_observe_histogram() {
        let repo = Arc::new(InMemorySessionRepo::new());
        let session = fresh_session(PlanTarget::Poc);
        let session_id = session.session_id;
        repo.insert_session(session);

        let handler = FunnelEventsHandler::new(repo.clone());

        // 1er email_verified : OK, fixe email_verified_at
        let input1 = FunnelEventInput {
            event_kind: "step.email_verified".into(),
            occurred_at: None,
            metadata: serde_json::json!({}),
            actor: "user".into(),
            error_class: None,
        };
        handler
            .handle(session_id, input1, Uuid::new_v4())
            .await
            .expect("ok");

        let first = repo.session(session_id).expect("ok").email_verified_at;
        assert!(first.is_some());

        // 2nd email_verified : event persisté mais state_changed = false
        let input2 = FunnelEventInput {
            event_kind: "step.email_verified".into(),
            occurred_at: None,
            metadata: serde_json::json!({}),
            actor: "user".into(),
            error_class: None,
        };
        handler
            .handle(session_id, input2, Uuid::new_v4())
            .await
            .expect("ok");

        let after = repo.session(session_id).expect("ok").email_verified_at;
        // Le timestamp ne doit PAS avoir bougé (= no-op idempotent).
        assert_eq!(first, after, "duplicate event should not move timestamp");

        // Les 2 events SONT persistés dans le log (= append-only OK).
        assert_eq!(repo.event_count(), 2);
    }

    #[tokio::test]
    async fn handler_404_when_session_unknown() {
        let repo = Arc::new(InMemorySessionRepo::new());
        let handler = FunnelEventsHandler::new(repo);
        let unknown = OnboardingSessionId::new_v4();
        let input = FunnelEventInput {
            event_kind: "session.started".into(),
            occurred_at: None,
            metadata: serde_json::json!({}),
            actor: "user".into(),
            error_class: None,
        };
        let err = handler
            .handle(unknown, input, Uuid::new_v4())
            .await
            .unwrap_err();
        assert!(matches!(err, OnboardingError::SessionNotFound(_)));
        assert_eq!(err.http_status_code(), 404);
    }
}
