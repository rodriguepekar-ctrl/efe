//! Breach reporting — `BreachReport`, `BreachContext`, `BreachFilter`,
//! `BreachLog` (ring buffer).
//!
//! Un breach est émis par M8 (ou un module de fusion intermédiaire) quand
//! la distance observée `observed_delta` excède le rayon autorisé
//! `cone_radius`. Le registre conserve les N derniers breaches dans un
//! ring buffer borné pour l'audit et les dashboards.

use std::collections::VecDeque;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::alignment::key::ConeKey;
use crate::errors::{EfeError, Result};
use crate::types::SwarmId;

/// Longueur maximale autorisée pour la raison d'un [`BreachContext::Manual`].
///
/// Borne stricte — protège contre les payloads abusifs qui saturent
/// le ring buffer de breaches, la sérialisation des événements et les
/// logs M11.
pub const BREACH_REASON_MAX_LEN: usize = 256;

/// Contexte d'émission d'un breach — où est-ce que la projection a eu lieu.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub enum BreachContext {
    /// Émis en dissolution finale (M8).
    Dissolution,
    /// Émis pendant un round de fusion (M5 avec projection soft activée).
    FusionRound {
        /// Numéro du round (1-based).
        round: u32,
    },
    /// Signalement manuel via admin / batch audit.
    ///
    /// Construction externe préférée via [`BreachContext::manual`] qui
    /// valide la longueur de `reason` (≤ [`BREACH_REASON_MAX_LEN`]).
    /// La variante reste construite directement depuis la crate pour
    /// la désérialisation serde.
    Manual {
        /// Raison textuelle libre.
        reason: String,
    },
}

impl BreachContext {
    /// Construit un `BreachContext::Manual` en validant la longueur de
    /// `reason`.
    ///
    /// # Errors
    ///
    /// [`EfeError::InvalidConfig`] si `reason.len() > BREACH_REASON_MAX_LEN`.
    pub fn manual(reason: impl Into<String>) -> Result<Self> {
        let reason = reason.into();
        if reason.len() > BREACH_REASON_MAX_LEN {
            return Err(EfeError::InvalidConfig(format!(
                "BreachContext::manual: reason length {} exceeds max {BREACH_REASON_MAX_LEN}",
                reason.len()
            )));
        }
        Ok(Self::Manual { reason })
    }
}

impl fmt::Display for BreachContext {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dissolution => write!(f, "dissolution"),
            Self::FusionRound { round } => write!(f, "fusion@round{round}"),
            Self::Manual { reason } => write!(f, "manual: {reason}"),
        }
    }
}

/// Rapport d'un breach observé.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BreachReport {
    /// Clé du cône qui a été violé.
    pub cone_key: ConeKey,
    /// Swarm en cours au moment du breach.
    pub swarm_id: SwarmId,
    /// Distance observée.
    pub observed_delta: f32,
    /// Rayon configuré du cône (copié pour évolution indépendante).
    pub cone_radius: f32,
    /// Timestamp Unix en nanosecondes.
    pub timestamp_nanos: u64,
    /// Contexte d'émission.
    pub context: BreachContext,
}

impl BreachReport {
    /// Construit un rapport de breach **sans** validation de longueur de
    /// `context.Manual.reason`. Préférer [`BreachReport::new_validated`]
    /// si l'on accepte la petite surcharge de vérification.
    ///
    /// Nécessaire pour les callers externes au crate qui ne peuvent pas
    /// construire la struct directement (elle est `#[non_exhaustive]`).
    #[must_use]
    pub fn new(
        cone_key: ConeKey,
        swarm_id: SwarmId,
        observed_delta: f32,
        cone_radius: f32,
        timestamp_nanos: u64,
        context: BreachContext,
    ) -> Self {
        Self {
            cone_key,
            swarm_id,
            observed_delta,
            cone_radius,
            timestamp_nanos,
            context,
        }
    }

    /// Construit un rapport en validant la longueur de `reason` pour
    /// `BreachContext::Manual`. Recommandé pour le chemin où le
    /// caller ne passe pas par [`BreachContext::manual`].
    ///
    /// # Errors
    ///
    /// Propage [`EfeError::InvalidConfig`] si la `reason` excède
    /// [`BREACH_REASON_MAX_LEN`].
    pub fn new_validated(
        cone_key: ConeKey,
        swarm_id: SwarmId,
        observed_delta: f32,
        cone_radius: f32,
        timestamp_nanos: u64,
        context: BreachContext,
    ) -> Result<Self> {
        if let BreachContext::Manual { reason } = &context {
            if reason.len() > BREACH_REASON_MAX_LEN {
                return Err(EfeError::InvalidConfig(format!(
                    "BreachReport: Manual reason length {} exceeds max {BREACH_REASON_MAX_LEN}",
                    reason.len()
                )));
            }
        }
        Ok(Self::new(
            cone_key,
            swarm_id,
            observed_delta,
            cone_radius,
            timestamp_nanos,
            context,
        ))
    }

    /// Magnitude du breach (peut être négative dans des cas de
    /// reporting spéculatif ; normalement strictement positive).
    #[must_use]
    pub fn magnitude(&self) -> f32 {
        self.observed_delta - self.cone_radius
    }
}

impl fmt::Display for BreachReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "breach[{}] swarm={} delta={:.6} radius={:.6} ctx={}",
            self.cone_key,
            self.swarm_id.as_uuid(),
            self.observed_delta,
            self.cone_radius,
            self.context
        )
    }
}

// =====================================================================
// BreachFilter
// =====================================================================

/// Filtre optionnel pour `breach_history`.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct BreachFilter {
    /// Si `Some`, ne retient que les rapports dont `cone_key.tenant == t`.
    pub tenant: Option<String>,
    /// Si `Some`, ne retient que les rapports dont `cone_key.task == t`.
    pub task: Option<String>,
    /// Si `Some`, ne retient que les rapports pour ce `swarm_id`.
    pub swarm_id: Option<SwarmId>,
    /// Timestamp nanos minimum inclus.
    pub since_nanos: Option<u64>,
    /// Timestamp nanos maximum inclus.
    pub until_nanos: Option<u64>,
    /// Magnitude minimale (inclusive).
    pub min_magnitude: Option<f32>,
}

impl BreachFilter {
    /// Filtre sans aucun critère — accepte tout.
    #[must_use]
    pub fn none() -> Self {
        Self::default()
    }

    /// Configure le filtre tenant (builder-style).
    #[must_use]
    pub fn with_tenant(mut self, tenant: impl Into<String>) -> Self {
        self.tenant = Some(tenant.into());
        self
    }

    /// Configure le filtre task.
    #[must_use]
    pub fn with_task(mut self, task: impl Into<String>) -> Self {
        self.task = Some(task.into());
        self
    }

    /// Configure le filtre swarm_id.
    #[must_use]
    pub fn with_swarm_id(mut self, sid: SwarmId) -> Self {
        self.swarm_id = Some(sid);
        self
    }

    /// Configure la fenêtre temporelle (inclusive des deux côtés).
    #[must_use]
    pub fn with_time_window(mut self, since: u64, until: u64) -> Self {
        self.since_nanos = Some(since);
        self.until_nanos = Some(until);
        self
    }

    /// Configure la magnitude minimale retenue.
    #[must_use]
    pub fn with_min_magnitude(mut self, m: f32) -> Self {
        self.min_magnitude = Some(m);
        self
    }

    /// Retourne `true` si `report` passe le filtre.
    #[must_use]
    pub fn matches(&self, report: &BreachReport) -> bool {
        if let Some(t) = &self.tenant {
            if &report.cone_key.tenant != t {
                return false;
            }
        }
        if let Some(t) = &self.task {
            if &report.cone_key.task != t {
                return false;
            }
        }
        if let Some(sid) = self.swarm_id {
            if report.swarm_id != sid {
                return false;
            }
        }
        if let Some(since) = self.since_nanos {
            if report.timestamp_nanos < since {
                return false;
            }
        }
        if let Some(until) = self.until_nanos {
            if report.timestamp_nanos > until {
                return false;
            }
        }
        if let Some(m) = self.min_magnitude {
            if report.magnitude() < m {
                return false;
            }
        }
        true
    }
}

// =====================================================================
// BreachLog (ring buffer)
// =====================================================================

/// Ring buffer borné de `BreachReport`.
#[derive(Debug)]
pub struct BreachLog {
    capacity: usize,
    buffer: VecDeque<BreachReport>,
}

impl BreachLog {
    /// Construit un ring de capacité `capacity` (≥ 1).
    ///
    /// # Panics
    ///
    /// Panique si `capacity == 0` — ne doit jamais survenir en pratique
    /// car [`crate::alignment::policy::RegistryConfig::validate`] rejette
    /// cette configuration avant instanciation du registre.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "BreachLog capacity must be > 0");
        Self {
            capacity,
            buffer: VecDeque::with_capacity(capacity),
        }
    }

    /// Taille courante.
    #[must_use]
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// `true` si le buffer est vide.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// Capacité maximale.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Ajoute un rapport ; évince le plus ancien si saturé.
    pub fn push(&mut self, report: BreachReport) {
        if self.buffer.len() == self.capacity {
            self.buffer.pop_front();
        }
        self.buffer.push_back(report);
    }

    /// Retourne jusqu'à `n` rapports du plus récent au plus ancien,
    /// filtrés par `filter` (si fourni).
    #[must_use]
    pub fn recent(&self, n: usize, filter: Option<&BreachFilter>) -> Vec<BreachReport> {
        self.buffer
            .iter()
            .rev()
            .filter(|r| filter.is_none_or(|f| f.matches(r)))
            .take(n)
            .cloned()
            .collect()
    }

    /// Itère sur le buffer **du plus ancien au plus récent**.
    ///
    /// Utile pour l'audit ou l'export complet. Pour obtenir les N
    /// rapports les plus récents filtrés, préférer
    /// [`BreachLog::recent`].
    pub fn iter(&self) -> impl Iterator<Item = &BreachReport> {
        self.buffer.iter()
    }
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_report(tenant: &str, magnitude: f32, ts: u64) -> BreachReport {
        BreachReport {
            cone_key: ConeKey::new(tenant, "task", 1).unwrap(),
            swarm_id: SwarmId::new(),
            observed_delta: 1.0 + magnitude,
            cone_radius: 1.0,
            timestamp_nanos: ts,
            context: BreachContext::Dissolution,
        }
    }

    // --- BreachContext ---

    #[test]
    fn breach_context_display() {
        assert_eq!(BreachContext::Dissolution.to_string(), "dissolution");
        assert_eq!(
            BreachContext::FusionRound { round: 3 }.to_string(),
            "fusion@round3"
        );
        assert_eq!(
            BreachContext::Manual {
                reason: "audit".into()
            }
            .to_string(),
            "manual: audit"
        );
    }

    // --- BreachReport ---

    #[test]
    fn report_magnitude() {
        let r = sample_report("t", 0.3, 100);
        assert!((r.magnitude() - 0.3).abs() < 1e-6);
    }

    #[test]
    fn report_display_contains_key() {
        let r = sample_report("acme", 0.2, 100);
        let s = r.to_string();
        assert!(s.contains("acme"));
        assert!(s.contains("task"));
    }

    #[test]
    fn report_serde_roundtrip() {
        let r = sample_report("acme", 0.25, 123_456_789);
        let s = serde_json::to_string(&r).unwrap();
        let back: BreachReport = serde_json::from_str(&s).unwrap();
        assert_eq!(back.cone_key, r.cone_key);
        assert!((back.observed_delta - r.observed_delta).abs() < 1e-6);
    }

    // --- BreachFilter ---

    #[test]
    fn filter_default_matches_all() {
        let r = sample_report("acme", 0.1, 100);
        assert!(BreachFilter::default().matches(&r));
    }

    #[test]
    fn filter_by_tenant() {
        let r = sample_report("acme", 0.1, 100);
        let ok = BreachFilter {
            tenant: Some("acme".into()),
            ..Default::default()
        };
        let ko = BreachFilter {
            tenant: Some("other".into()),
            ..Default::default()
        };
        assert!(ok.matches(&r));
        assert!(!ko.matches(&r));
    }

    #[test]
    fn filter_by_task() {
        let r = sample_report("acme", 0.1, 100);
        let ko = BreachFilter {
            task: Some("wrong".into()),
            ..Default::default()
        };
        assert!(!ko.matches(&r));
    }

    #[test]
    fn filter_by_time_window() {
        let r = sample_report("t", 0.1, 100);
        let ok = BreachFilter {
            since_nanos: Some(50),
            until_nanos: Some(200),
            ..Default::default()
        };
        let ko = BreachFilter {
            since_nanos: Some(200),
            ..Default::default()
        };
        assert!(ok.matches(&r));
        assert!(!ko.matches(&r));
    }

    #[test]
    fn filter_by_magnitude() {
        let r = sample_report("t", 0.3, 100);
        let ok = BreachFilter {
            min_magnitude: Some(0.2),
            ..Default::default()
        };
        let ko = BreachFilter {
            min_magnitude: Some(0.5),
            ..Default::default()
        };
        assert!(ok.matches(&r));
        assert!(!ko.matches(&r));
    }

    #[test]
    fn filter_by_swarm_id() {
        let sid = SwarmId::new();
        let other = SwarmId::new();
        let mut r = sample_report("t", 0.1, 100);
        r.swarm_id = sid;
        let ok = BreachFilter {
            swarm_id: Some(sid),
            ..Default::default()
        };
        let ko = BreachFilter {
            swarm_id: Some(other),
            ..Default::default()
        };
        assert!(ok.matches(&r));
        assert!(!ko.matches(&r));
    }

    // --- BreachLog ---

    #[test]
    fn log_push_and_len() {
        let mut log = BreachLog::new(3);
        assert!(log.is_empty());
        log.push(sample_report("t", 0.1, 1));
        assert_eq!(log.len(), 1);
        log.push(sample_report("t", 0.1, 2));
        log.push(sample_report("t", 0.1, 3));
        assert_eq!(log.len(), 3);
    }

    #[test]
    fn log_fifo_eviction() {
        let mut log = BreachLog::new(2);
        log.push(sample_report("t", 0.1, 1));
        log.push(sample_report("t", 0.1, 2));
        log.push(sample_report("t", 0.1, 3));
        assert_eq!(log.len(), 2);
        let timestamps: Vec<u64> = log.iter().map(|r| r.timestamp_nanos).collect();
        // Oldest (ts=1) evicted ; kept 2 and 3.
        assert_eq!(timestamps, vec![2, 3]);
    }

    #[test]
    fn log_recent_returns_reverse_chronological() {
        let mut log = BreachLog::new(5);
        for i in 1..=5 {
            log.push(sample_report("t", 0.1, i));
        }
        let recent = log.recent(3, None);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].timestamp_nanos, 5);
        assert_eq!(recent[1].timestamp_nanos, 4);
        assert_eq!(recent[2].timestamp_nanos, 3);
    }

    #[test]
    fn log_recent_with_filter() {
        let mut log = BreachLog::new(10);
        log.push(sample_report("acme", 0.1, 1));
        log.push(sample_report("other", 0.1, 2));
        log.push(sample_report("acme", 0.1, 3));
        let filter = BreachFilter {
            tenant: Some("acme".into()),
            ..Default::default()
        };
        let r = log.recent(10, Some(&filter));
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].timestamp_nanos, 3);
        assert_eq!(r[1].timestamp_nanos, 1);
    }

    #[test]
    fn log_recent_n_zero_empty() {
        let mut log = BreachLog::new(3);
        log.push(sample_report("t", 0.1, 1));
        assert!(log.recent(0, None).is_empty());
    }

    #[test]
    #[should_panic(expected = "capacity must be > 0")]
    fn log_zero_capacity_panics() {
        let _ = BreachLog::new(0);
    }

    #[test]
    fn log_capacity_accessor() {
        let log = BreachLog::new(42);
        assert_eq!(log.capacity(), 42);
    }

    // --- BreachContext::manual validation ---

    #[test]
    fn manual_accepts_short_reason() {
        assert!(BreachContext::manual("short").is_ok());
        assert!(BreachContext::manual("").is_ok()); // empty is OK
    }

    #[test]
    fn manual_accepts_exact_boundary() {
        let boundary = "x".repeat(BREACH_REASON_MAX_LEN);
        assert!(BreachContext::manual(boundary).is_ok());
    }

    #[test]
    fn manual_rejects_over_limit() {
        let too_long = "x".repeat(BREACH_REASON_MAX_LEN + 1);
        let err = BreachContext::manual(too_long).unwrap_err();
        assert!(matches!(err, EfeError::InvalidConfig(_)));
    }

    #[test]
    fn new_validated_rejects_over_limit_manual() {
        let too_long = "x".repeat(BREACH_REASON_MAX_LEN + 1);
        // Bypass : construction directe via variant (intra-crate OK).
        let ctx = BreachContext::Manual { reason: too_long };
        let err = BreachReport::new_validated(
            ConeKey::new("t", "a", 1).unwrap(),
            SwarmId::new(),
            0.9,
            0.5,
            1,
            ctx,
        )
        .unwrap_err();
        assert!(matches!(err, EfeError::InvalidConfig(_)));
    }

    #[test]
    fn new_validated_accepts_non_manual_context() {
        let report = BreachReport::new_validated(
            ConeKey::new("t", "a", 1).unwrap(),
            SwarmId::new(),
            0.9,
            0.5,
            1,
            BreachContext::Dissolution,
        )
        .unwrap();
        assert_eq!(report.context, BreachContext::Dissolution);
    }
}
