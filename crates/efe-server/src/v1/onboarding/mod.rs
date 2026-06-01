// SPDX-License-Identifier: UNLICENSED
//! Sprint **D19 (P2-Foundation)** — Self-service onboarding /
//! activation funnel.
//!
//! Cf. `docs/EFE_P2_SELF_SERVICE_ARCHITECTURE.md`.
//!
//! ## Vue d'ensemble
//!
//! Ce module implémente la **surface d'activation opposable** (P2) :
//!
//! - [`types`] — modèles de données : `OnboardingSession`,
//!   `OnboardingEvent`, `FunnelStep` (re-export depuis efe-telemetry),
//!   `ErrorClass`, etc.
//! - [`funnel_events`] — endpoint WebSocket `/v1/funnel/{session_id}`
//!   qui reçoit les transitions d'état émises par le portail
//!   Next.js et les persiste en DB + Prometheus.
//! - [`session_repo`] — abstraction sur la table `onboarding_sessions`
//!   (read/write + signature HMAC à la complétion).
//! - [`saga`] (D20+) — saga compensable pour le provisioning.
//! - [`expire`] (D20+) — cron qui force-cleanup les sessions
//!   abandonnées (timeout 60 min).
//!
//! ## Invariants pinés
//!
//! 1. `started_at` est setté **avant** la 1ère row dans `onboarding_events` —
//!    enforced par contrainte FK + transaction unique.
//! 2. `completed_at` est setté uniquement par la transaction qui
//!    persiste le **dernier** event (= `session.completed` ou
//!    `session.abandoned` ou `system.session_expired`).
//! 3. La signature HMAC est calculée et stockée dans la **même**
//!    transaction que le set de `completed_at` (= atomicité).
//! 4. Le bitset `event_kind` est validé à 2 niveaux : (a) la CHECK
//!    constraint SQL whitelist, (b) le pattern match Rust dans
//!    `types::FunnelEventKind`. Un event_kind absent de l'enum
//!    Rust ne peut PAS être inséré (= cohérence forte).

pub mod funnel_events;
pub mod types;

pub use funnel_events::{FunnelEventInput, FunnelEventsHandler};
pub use types::{
    FunnelEventKind, OnboardingError, OnboardingEvent, OnboardingSession, OnboardingSessionId,
};
