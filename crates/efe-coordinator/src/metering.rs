// SPDX-License-Identifier: UNLICENSED
//! **R3.S7 — Télémétrie Prometheus : registre lock-free + rendu texte.**
//!
//! R2/R3 produisent une mine de données par mission (`ChewingMetrics`, verdicts
//! `FactCheck`, état `SlmCircuitBreaker`) mais uniquement observables au fil de
//! l'eau via SSE. R3.S7 expose ces données au **format d'exposition
//! Prometheus** pour le monitoring prod (Grafana / Alertmanager).
//!
//! ## Design (cf. architecture R3.S7)
//!
//! - **Hook lossless** : les compteurs sont alimentés par un [`MissionMetricGuard`]
//!   RAII posé en tête de chaque workflow. Il capture **succès** (via
//!   [`MissionMetricGuard::record_success`]) **ET erreur** (via `Drop` quand la
//!   mission `?`-retourne). Le bus serait lossy (mpsc) et n'émet `SwarmFailed`
//!   que pour Mission B → inadapté.
//! - **Zéro dépendance** : compteurs/histogrammes en `AtomicU64`, rendu texte
//!   fait main (le format Prometheus est trivial). Pas de crate
//!   `prometheus`/`hyper`.
//! - **Cardinalité bornée** : `tenant_id` (R3.S5, non borné) n'est **jamais**
//!   un label (anti explosion TSDB). Tous les labels sont énumérés.
//! - **Anti dead-config** : `efe_verdicts_total{verdict}` (counters
//!   idiomatiques) remplace le `quality_pass_rate_gauge` de la roadmap
//!   (rate dérivé en PromQL) ; `savings{mode="baseline"|"full"}` est descopé
//!   faute de source de coût-savings live (seul `speculative` en a une).
//! - **Backward-compat** : `metrics: None` ⇒ overhead nul (le guard est inerte).
//!
//! Le `MetricsRegistry` + le rendu + le guard sont **toujours** compilés (zéro
//! dépendance). Seul le serveur HTTP `/metrics` (`metrics_server.rs`) est gated
//! derrière la feature `metrics` (qui n'active que `tokio/net`).

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use dashmap::DashMap;
use efe_nano_app_coordinator::SlmCircuitState;

use crate::failure::FailureMode;
use crate::metrics::ChewingMetrics;
use crate::request::{MissionAResponse, MissionBResponse, SpeculativeResponse};

/// Bornes (ms) des buckets cumulatifs de l'histogramme de latence.
/// Une observation > 10000 ms n'incrémente aucun bucket fini → comptée
/// uniquement dans `le="+Inf"` (= `_count`), conforme à Prometheus.
pub const LATENCY_BUCKETS_MS: &[u64] =
    &[5, 10, 25, 50, 100, 250, 500, 1000, 2500, 5000, 10000];

// ─────────────────────────────────────────────────────────────────────────
// Primitives atomiques
// ─────────────────────────────────────────────────────────────────────────

/// Compteur monotone (u64 atomique, `Relaxed`).
#[derive(Debug, Default)]
pub struct Counter(AtomicU64);

impl Counter {
    fn add(&self, v: u64) {
        self.0.fetch_add(v, Ordering::Relaxed);
    }
    /// Valeur courante (snapshot).
    #[must_use]
    pub fn get(&self) -> u64 {
        self.0.load(Ordering::Relaxed)
    }
}

/// Histogramme à buckets fixes. Chaque `observe` incrémente **exactement** le
/// plus petit bucket dont la borne ≥ valeur ; le rendu cumule.
#[derive(Debug)]
struct Histogram {
    buckets: Vec<AtomicU64>,
    sum: AtomicU64,
    count: AtomicU64,
}

impl Histogram {
    fn new() -> Self {
        Self {
            buckets: LATENCY_BUCKETS_MS.iter().map(|_| AtomicU64::new(0)).collect(),
            sum: AtomicU64::new(0),
            count: AtomicU64::new(0),
        }
    }

    fn observe(&self, v_ms: u64) {
        for (i, &b) in LATENCY_BUCKETS_MS.iter().enumerate() {
            if v_ms <= b {
                self.buckets[i].fetch_add(1, Ordering::Relaxed);
                break;
            }
        }
        self.sum.fetch_add(v_ms, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Labels bornés (énumérés — cardinalité maîtrisée)
// ─────────────────────────────────────────────────────────────────────────

/// Type de mission (`type` label).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MissionType {
    /// Mission A (1 LLM pré-mâché).
    A,
    /// Mission B (N LLMs + vote).
    B,
    /// Mission Speculative (drafter + verifier).
    Speculative,
}
impl MissionType {
    const ALL: &'static [MissionType] = &[Self::A, Self::B, Self::Speculative];
    fn as_label(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::Speculative => "Speculative",
        }
    }
}

/// Issue d'une mission (`outcome` label).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Outcome {
    /// Mission terminée avec succès (réponse produite).
    Success,
    /// Mission échouée (erreur remontée via `?`).
    Error,
}
impl Outcome {
    const ALL: &'static [Outcome] = &[Self::Success, Self::Error];
    fn as_label(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Error => "error",
        }
    }
}

/// Phase mesurée par l'histogramme de latence (`phase` label).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    /// Nano-robot analyzer (NER/intent/langue).
    Analyzer,
    /// Nano-robot compressor.
    Compressor,
    /// Nano-robot verifier (fact-check) / verifier LLM (Speculative).
    Verifier,
    /// Appel au gros LLM (drafter Speculative inclus).
    Llm,
}
impl Phase {
    const ALL: &'static [Phase] = &[Self::Analyzer, Self::Compressor, Self::Verifier, Self::Llm];
    fn as_label(self) -> &'static str {
        match self {
            Self::Analyzer => "analyzer",
            Self::Compressor => "compressor",
            Self::Verifier => "verifier",
            Self::Llm => "llm",
        }
    }
}

/// Nature de dégradation (`kind` label), dérivée de [`FailureMode`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailureKind {
    /// Un SLM est tombé (circuit ouvert) → robot skippé.
    SlmDown,
    /// Le LLM choisi a échoué → bascule fallback chain.
    LlmFallbackTriggered,
    /// Mission B : quorum atteint mais < N votes.
    VoteDegraded,
    /// Mission Speculative : drafter échoué → verifier-only.
    DrafterBypassed,
}
impl FailureKind {
    const ALL: &'static [FailureKind] = &[
        Self::SlmDown,
        Self::LlmFallbackTriggered,
        Self::VoteDegraded,
        Self::DrafterBypassed,
    ];
    fn as_label(self) -> &'static str {
        match self {
            Self::SlmDown => "SLM_DOWN",
            Self::LlmFallbackTriggered => "LLM_FALLBACK_TRIGGERED",
            Self::VoteDegraded => "VOTE_DEGRADED",
            Self::DrafterBypassed => "DRAFTER_BYPASSED",
        }
    }
}

/// Mode de savings (`mode` label). Seul `Speculative` a une source live
/// (`spec_savings_microcents`) ; `full`/`baseline` descopés (cf. doc module).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SavingsMode {
    /// Savings réels du bypass verifier (hot path Speculative).
    Speculative,
}
impl SavingsMode {
    const ALL: &'static [SavingsMode] = &[Self::Speculative];
    fn as_label(self) -> &'static str {
        match self {
            Self::Speculative => "speculative",
        }
    }
}

/// Verdict de fact-check (`verdict` label), dérivé de [`crate::request::FactCheck`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Verdict {
    /// Claim soutenu par les sources.
    Supported,
    /// Claim non soutenu.
    Unsupported,
    /// Claim contredit par les sources.
    Contradicted,
    /// Preuves insuffisantes pour trancher.
    InsufficientEvidence,
    /// Verdict inconnu / non mappé (borne la cardinalité).
    Other,
}
impl Verdict {
    const ALL: &'static [Verdict] = &[
        Self::Supported,
        Self::Unsupported,
        Self::Contradicted,
        Self::InsufficientEvidence,
        Self::Other,
    ];
    fn as_label(self) -> &'static str {
        match self {
            Self::Supported => "SUPPORTED",
            Self::Unsupported => "UNSUPPORTED",
            Self::Contradicted => "CONTRADICTED",
            Self::InsufficientEvidence => "INSUFFICIENT_EVIDENCE",
            Self::Other => "OTHER",
        }
    }
    /// Mappe une string verdict (bornée) ; inconnu → `Other` (jamais de label
    /// non borné).
    fn parse(s: &str) -> Self {
        match s {
            "SUPPORTED" => Self::Supported,
            "UNSUPPORTED" => Self::Unsupported,
            "CONTRADICTED" => Self::Contradicted,
            "INSUFFICIENT_EVIDENCE" => Self::InsufficientEvidence,
            _ => Self::Other,
        }
    }
}

/// Aplatit un [`FailureMode`] en kinds atomiques (Composite déplié).
fn failure_kinds_of(mode: &FailureMode) -> Vec<FailureKind> {
    match mode {
        FailureMode::None => vec![],
        FailureMode::SlmDown(_) => vec![FailureKind::SlmDown],
        FailureMode::LlmFallbackTriggered { .. } => vec![FailureKind::LlmFallbackTriggered],
        FailureMode::VoteDegraded { .. } => vec![FailureKind::VoteDegraded],
        FailureMode::DrafterBypassed(_) => vec![FailureKind::DrafterBypassed],
        FailureMode::Composite(modes) => modes.iter().flat_map(failure_kinds_of).collect(),
    }
}

/// Extrait les latences par phase (>0 seulement) depuis `ChewingMetrics`
/// (variante Mission A/B : analyzer/compressor/verifier/llm).
fn phase_latencies_full(cm: &ChewingMetrics) -> Vec<(Phase, u64)> {
    [
        (Phase::Analyzer, cm.analyzer_latency_ms),
        (Phase::Compressor, cm.compressor_latency_ms),
        (Phase::Verifier, cm.verifier_latency_ms),
        (Phase::Llm, cm.llm_latency_ms),
    ]
    .into_iter()
    .filter(|&(_, ms)| ms > 0)
    .collect()
}

// ─────────────────────────────────────────────────────────────────────────
// Vue normalisée d'une mission (découple le registre des types `request.rs`)
// ─────────────────────────────────────────────────────────────────────────

/// Vue normalisée enregistrée par le guard. Construite depuis chaque réponse
/// via `from_mission_a/b/speculative`, ou `error(ty)` pour l'issue d'échec.
#[derive(Debug, Clone)]
pub struct MissionRecord {
    /// Type de mission.
    pub ty: MissionType,
    /// Issue.
    pub outcome: Outcome,
    /// Provider facturé (label `provider`).
    pub provider: String,
    /// Coût constaté (µ¢).
    pub cost_microcents: u64,
    /// Latences par phase (>0).
    pub phase_latencies: Vec<(Phase, u64)>,
    /// Savings (mode, µ¢) si applicable.
    pub savings: Option<(SavingsMode, u64)>,
    /// Verdicts produits.
    pub verdicts: Vec<Verdict>,
    /// Dégradations observées.
    pub failure_kinds: Vec<FailureKind>,
}

impl MissionRecord {
    /// Issue d'échec : compte seulement `missions_total{outcome="error"}`.
    #[must_use]
    pub fn error(ty: MissionType) -> Self {
        Self {
            ty,
            outcome: Outcome::Error,
            provider: String::new(),
            cost_microcents: 0,
            phase_latencies: vec![],
            savings: None,
            verdicts: vec![],
            failure_kinds: vec![],
        }
    }

    /// Vue d'une Mission A réussie.
    #[must_use]
    pub fn from_mission_a(r: &MissionAResponse) -> Self {
        let (phase_latencies, failure_kinds) = match &r.chewing_metrics {
            Some(cm) => (phase_latencies_full(cm), failure_kinds_of(&cm.failure_mode)),
            None => (vec![], vec![]),
        };
        Self {
            ty: MissionType::A,
            outcome: Outcome::Success,
            provider: r.llm_used.clone(),
            cost_microcents: r.cost_microcents,
            phase_latencies,
            savings: None, // mode "full" descopé (pas de coût-savings live)
            verdicts: r.verdicts.iter().map(|v| Verdict::parse(&v.verdict)).collect(),
            failure_kinds,
        }
    }

    /// Vue d'une Mission B réussie. `cost` = agrégat fan-out, étiqueté par le
    /// gagnant du vote (approximation documentée, architecture §4.5).
    #[must_use]
    pub fn from_mission_b(r: &MissionBResponse) -> Self {
        let (phase_latencies, failure_kinds) = match &r.chewing_metrics {
            Some(cm) => (phase_latencies_full(cm), failure_kinds_of(&cm.failure_mode)),
            None => (vec![], vec![]),
        };
        Self {
            ty: MissionType::B,
            outcome: Outcome::Success,
            provider: r.winner_llm.clone(),
            cost_microcents: r.total_cost_microcents,
            phase_latencies,
            savings: None,
            verdicts: r.verdicts.iter().map(|v| Verdict::parse(&v.verdict)).collect(),
            failure_kinds,
        }
    }

    /// Vue d'une mission Speculative réussie. Phases : drafter→`llm`,
    /// verifier→`verifier`. Savings réel quand le verifier a été skippé.
    #[must_use]
    pub fn from_speculative(r: &SpeculativeResponse) -> Self {
        let mut phase_latencies = vec![];
        let mut failure_kinds = vec![];
        let mut savings = None;
        let mut provider = String::new();
        if let Some(cm) = &r.chewing_metrics {
            if cm.spec_drafter_latency_ms > 0 {
                phase_latencies.push((Phase::Llm, cm.spec_drafter_latency_ms));
            }
            if cm.spec_verifier_latency_ms > 0 {
                phase_latencies.push((Phase::Verifier, cm.spec_verifier_latency_ms));
            }
            failure_kinds = failure_kinds_of(&cm.failure_mode);
            provider = cm.selected_llm.clone();
            if r.verifier_skipped && cm.spec_savings_microcents > 0 {
                savings = Some((SavingsMode::Speculative, cm.spec_savings_microcents));
            }
        }
        Self {
            ty: MissionType::Speculative,
            outcome: Outcome::Success,
            provider,
            cost_microcents: r.cost_microcents,
            phase_latencies,
            savings,
            verdicts: vec![], // Speculative n'a pas de verdicts
            failure_kinds,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Registre
// ─────────────────────────────────────────────────────────────────────────

/// Registre central de métriques. Clés à labels bornés pré-allouées
/// (lookup O(1), zéro allocation au hot path). `provider` (≈ borné mais
/// dynamique) via `DashMap`.
#[derive(Debug)]
pub struct MetricsRegistry {
    missions: HashMap<(MissionType, Outcome), Counter>,
    cost_microcents: DashMap<String, Counter>,
    latency: HashMap<Phase, Histogram>,
    failure_mode: HashMap<FailureKind, Counter>,
    savings_microcents: HashMap<SavingsMode, Counter>,
    verdicts: HashMap<Verdict, Counter>,
}

impl Default for MetricsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl MetricsRegistry {
    /// Construit un registre avec toutes les clés bornées pré-allouées.
    #[must_use]
    pub fn new() -> Self {
        let mut missions = HashMap::new();
        for &t in MissionType::ALL {
            for &o in Outcome::ALL {
                missions.insert((t, o), Counter::default());
            }
        }
        let mut latency = HashMap::new();
        for &p in Phase::ALL {
            latency.insert(p, Histogram::new());
        }
        let mut failure_mode = HashMap::new();
        for &k in FailureKind::ALL {
            failure_mode.insert(k, Counter::default());
        }
        let mut savings_microcents = HashMap::new();
        for &m in SavingsMode::ALL {
            savings_microcents.insert(m, Counter::default());
        }
        let mut verdicts = HashMap::new();
        for &v in Verdict::ALL {
            verdicts.insert(v, Counter::default());
        }
        Self {
            missions,
            cost_microcents: DashMap::new(),
            latency,
            failure_mode,
            savings_microcents,
            verdicts,
        }
    }

    /// Hook lossless : enregistre une mission (appelé par [`MissionMetricGuard`]).
    /// L'issue est toujours comptée ; les détails seulement en succès.
    pub fn record_mission(&self, rec: &MissionRecord) {
        if let Some(c) = self.missions.get(&(rec.ty, rec.outcome)) {
            c.add(1);
        }
        if rec.outcome != Outcome::Success {
            return;
        }
        if rec.cost_microcents > 0 && !rec.provider.is_empty() {
            self.cost_microcents
                .entry(rec.provider.clone())
                .or_default()
                .add(rec.cost_microcents);
        }
        for (phase, ms) in &rec.phase_latencies {
            if let Some(h) = self.latency.get(phase) {
                h.observe(*ms);
            }
        }
        if let Some((mode, mc)) = rec.savings {
            if let Some(c) = self.savings_microcents.get(&mode) {
                c.add(mc);
            }
        }
        for v in &rec.verdicts {
            if let Some(c) = self.verdicts.get(v) {
                c.add(1);
            }
        }
        for k in &rec.failure_kinds {
            if let Some(c) = self.failure_mode.get(k) {
                c.add(1);
            }
        }
    }

    /// Accès lecture d'un compteur missions (tests).
    #[must_use]
    pub fn missions_count(&self, ty: MissionType, outcome: Outcome) -> u64 {
        self.missions.get(&(ty, outcome)).map_or(0, Counter::get)
    }

    /// Accès lecture du coût cumulé d'un provider (tests).
    #[must_use]
    pub fn cost_microcents_of(&self, provider: &str) -> u64 {
        self.cost_microcents.get(provider).map_or(0, |c| c.get())
    }

    /// Accès lecture des savings cumulés d'un mode (tests / introspection).
    #[must_use]
    pub fn savings_microcents_of(&self, mode: SavingsMode) -> u64 {
        self.savings_microcents.get(&mode).map_or(0, Counter::get)
    }

    /// Accès lecture du nombre de verdicts d'un type (tests / introspection).
    #[must_use]
    pub fn verdicts_count(&self, verdict: Verdict) -> u64 {
        self.verdicts.get(&verdict).map_or(0, Counter::get)
    }

    /// Accès lecture du nombre de dégradations d'un kind (tests / introspection).
    #[must_use]
    pub fn failure_mode_count(&self, kind: FailureKind) -> u64 {
        self.failure_mode.get(&kind).map_or(0, Counter::get)
    }

    /// Rendu au format d'exposition Prometheus (`text/plain; version=0.0.4`).
    ///
    /// `circuits` = snapshot scrape-time `(robot, état)` fourni par le serveur
    /// (les circuits changent d'état de façon asynchrone → gauge lu au scrape).
    #[must_use]
    pub fn render_prometheus(&self, circuits: &[(String, SlmCircuitState)]) -> String {
        let mut out = String::with_capacity(4096);

        // ── efe_missions_total ──
        let _ = writeln!(out, "# HELP efe_missions_total Total missions by type and outcome.");
        let _ = writeln!(out, "# TYPE efe_missions_total counter");
        for &t in MissionType::ALL {
            for &o in Outcome::ALL {
                let v = self.missions_count(t, o);
                let _ = writeln!(
                    out,
                    "efe_missions_total{{type=\"{}\",outcome=\"{}\"}} {v}",
                    t.as_label(),
                    o.as_label()
                );
            }
        }

        // ── efe_cost_microcents_total ──
        let _ = writeln!(
            out,
            "# HELP efe_cost_microcents_total Cumulative cost (microcents) by billed provider \
             (Mission B = fan-out aggregate labeled by vote winner)."
        );
        let _ = writeln!(out, "# TYPE efe_cost_microcents_total counter");
        let mut providers: Vec<(String, u64)> = self
            .cost_microcents
            .iter()
            .map(|e| (e.key().clone(), e.value().get()))
            .collect();
        providers.sort_by(|a, b| a.0.cmp(&b.0)); // ordre stable
        for (provider, v) in providers {
            let _ = writeln!(
                out,
                "efe_cost_microcents_total{{provider=\"{}\"}} {v}",
                escape_label(&provider)
            );
        }

        // ── efe_latency_ms (histogram) ──
        let _ = writeln!(out, "# HELP efe_latency_ms Per-phase latency distribution (ms).");
        let _ = writeln!(out, "# TYPE efe_latency_ms histogram");
        for &p in Phase::ALL {
            let h = &self.latency[&p];
            let mut cum = 0u64;
            for (i, &b) in LATENCY_BUCKETS_MS.iter().enumerate() {
                cum += h.buckets[i].load(Ordering::Relaxed);
                let _ = writeln!(
                    out,
                    "efe_latency_ms_bucket{{phase=\"{}\",le=\"{b}\"}} {cum}",
                    p.as_label()
                );
            }
            let count = h.count.load(Ordering::Relaxed);
            let _ = writeln!(
                out,
                "efe_latency_ms_bucket{{phase=\"{}\",le=\"+Inf\"}} {count}",
                p.as_label()
            );
            let _ = writeln!(
                out,
                "efe_latency_ms_sum{{phase=\"{}\"}} {}",
                p.as_label(),
                h.sum.load(Ordering::Relaxed)
            );
            let _ = writeln!(out, "efe_latency_ms_count{{phase=\"{}\"}} {count}", p.as_label());
        }

        // ── efe_failure_mode_total ──
        let _ = writeln!(out, "# HELP efe_failure_mode_total Graceful-degradation events by kind.");
        let _ = writeln!(out, "# TYPE efe_failure_mode_total counter");
        for &k in FailureKind::ALL {
            let v = self.failure_mode.get(&k).map_or(0, Counter::get);
            let _ = writeln!(out, "efe_failure_mode_total{{kind=\"{}\"}} {v}", k.as_label());
        }

        // ── efe_savings_microcents_total ──
        let _ = writeln!(
            out,
            "# HELP efe_savings_microcents_total Cumulative savings (microcents) by mode."
        );
        let _ = writeln!(out, "# TYPE efe_savings_microcents_total counter");
        for &m in SavingsMode::ALL {
            let v = self.savings_microcents.get(&m).map_or(0, Counter::get);
            let _ = writeln!(out, "efe_savings_microcents_total{{mode=\"{}\"}} {v}", m.as_label());
        }

        // ── efe_verdicts_total ──
        let _ = writeln!(out, "# HELP efe_verdicts_total Fact-check verdicts by type.");
        let _ = writeln!(out, "# TYPE efe_verdicts_total counter");
        for &vd in Verdict::ALL {
            let v = self.verdicts.get(&vd).map_or(0, Counter::get);
            let _ = writeln!(out, "efe_verdicts_total{{verdict=\"{}\"}} {v}", vd.as_label());
        }

        // ── efe_slm_circuit_state (gauge, scrape-time) ──
        let _ = writeln!(out, "# HELP efe_slm_circuit_state Current SLM circuit state (1=active).");
        let _ = writeln!(out, "# TYPE efe_slm_circuit_state gauge");
        for (robot, state) in circuits {
            for st in ["closed", "open", "half_open"] {
                let active = u8::from(circuit_state_label(*state) == st);
                let _ = writeln!(
                    out,
                    "efe_slm_circuit_state{{robot=\"{}\",state=\"{st}\"}} {active}",
                    escape_label(robot)
                );
            }
        }

        out
    }
}

/// Label canonique d'un état de circuit.
fn circuit_state_label(state: SlmCircuitState) -> &'static str {
    match state {
        SlmCircuitState::Closed => "closed",
        SlmCircuitState::Open => "open",
        SlmCircuitState::HalfOpen => "half_open",
    }
}

/// Échappe une valeur de label Prometheus (`\`, `"`, newline).
fn escape_label(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────
// Guard RAII (capture succès + erreur)
// ─────────────────────────────────────────────────────────────────────────

/// Garde RAII posée en tête de workflow. Capture **l'erreur** via `Drop`
/// (mission `?`-retournée) et **le succès** via [`record_success`]. Inerte si
/// `reg` est `None` (télémétrie off → overhead nul).
///
/// [`record_success`]: MissionMetricGuard::record_success
pub struct MissionMetricGuard {
    reg: Option<Arc<MetricsRegistry>>,
    ty: MissionType,
    armed: bool,
}

impl MissionMetricGuard {
    /// Arme la garde. Tant qu'elle n'est pas désarmée (`record_success`), un
    /// `Drop` enregistre `outcome="error"`.
    #[must_use]
    pub fn arm(reg: Option<Arc<MetricsRegistry>>, ty: MissionType) -> Self {
        Self { reg, ty, armed: true }
    }

    /// Désarme + enregistre la mission réussie (vue détaillée).
    pub fn record_success(&mut self, rec: &MissionRecord) {
        self.armed = false;
        if let Some(r) = &self.reg {
            r.record_mission(rec);
        }
    }
}

impl Drop for MissionMetricGuard {
    fn drop(&mut self) {
        if self.armed {
            if let Some(r) = &self.reg {
                r.record_mission(&MissionRecord::error(self.ty));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_and_histogram_basics() {
        let c = Counter::default();
        c.add(3);
        c.add(4);
        assert_eq!(c.get(), 7);

        let h = Histogram::new();
        h.observe(7); // bucket le=10
        h.observe(120); // bucket le=250
        h.observe(20000); // > 10000 → seulement +Inf (count), aucun bucket fini
        assert_eq!(h.count.load(Ordering::Relaxed), 3);
        assert_eq!(h.sum.load(Ordering::Relaxed), 7 + 120 + 20000);
        // bucket le=10 contient 1 (le 7).
        let idx10 = LATENCY_BUCKETS_MS.iter().position(|&b| b == 10).unwrap();
        assert_eq!(h.buckets[idx10].load(Ordering::Relaxed), 1);
    }

    #[test]
    fn metrics_registry_increments_counter() {
        let reg = MetricsRegistry::new();
        let rec = MissionRecord {
            ty: MissionType::A,
            outcome: Outcome::Success,
            provider: "claude".into(),
            cost_microcents: 500,
            phase_latencies: vec![(Phase::Llm, 120)],
            savings: None,
            verdicts: vec![Verdict::Supported],
            failure_kinds: vec![],
        };
        reg.record_mission(&rec);
        reg.record_mission(&rec);
        assert_eq!(reg.missions_count(MissionType::A, Outcome::Success), 2);
        assert_eq!(reg.cost_microcents_of("claude"), 1000);
    }

    #[test]
    fn metrics_registry_records_savings_verdicts_failures() {
        // Couvre les chemins savings + verdicts + failure_mode (sinon non
        // testés : tous les autres tests passent `savings: None`).
        let reg = MetricsRegistry::new();
        let rec = MissionRecord {
            ty: MissionType::Speculative,
            outcome: Outcome::Success,
            provider: "gemini".into(),
            cost_microcents: 3,
            phase_latencies: vec![(Phase::Llm, 30), (Phase::Verifier, 200)],
            savings: Some((SavingsMode::Speculative, 5_000)),
            verdicts: vec![Verdict::Supported, Verdict::Contradicted, Verdict::Supported],
            failure_kinds: vec![FailureKind::DrafterBypassed],
        };
        reg.record_mission(&rec);
        reg.record_mission(&rec);

        assert_eq!(reg.savings_microcents_of(SavingsMode::Speculative), 10_000);
        assert_eq!(reg.verdicts_count(Verdict::Supported), 4);
        assert_eq!(reg.verdicts_count(Verdict::Contradicted), 2);
        assert_eq!(reg.verdicts_count(Verdict::Unsupported), 0);
        assert_eq!(reg.failure_mode_count(FailureKind::DrafterBypassed), 2);
        assert_eq!(reg.failure_mode_count(FailureKind::SlmDown), 0);

        // Rendu : les lignes savings/verdicts apparaissent avec les bonnes valeurs.
        let txt = reg.render_prometheus(&[]);
        assert!(txt.contains("efe_savings_microcents_total{mode=\"speculative\"} 10000"));
        assert!(txt.contains("efe_verdicts_total{verdict=\"SUPPORTED\"} 4"));
        assert!(txt.contains("efe_failure_mode_total{kind=\"DRAFTER_BYPASSED\"} 2"));
    }

    #[test]
    fn error_outcome_skips_detail_metrics() {
        // Une issue d'échec ne doit incrémenter QUE le compteur missions
        // (pas de coût/savings/verdicts fantômes).
        let reg = MetricsRegistry::new();
        reg.record_mission(&MissionRecord {
            // outcome=Error mais champs détaillés non vides → doivent être ignorés.
            cost_microcents: 999,
            savings: Some((SavingsMode::Speculative, 999)),
            verdicts: vec![Verdict::Supported],
            ..MissionRecord::error(MissionType::A)
        });
        assert_eq!(reg.missions_count(MissionType::A, Outcome::Error), 1);
        assert_eq!(reg.cost_microcents_of(""), 0);
        assert_eq!(reg.savings_microcents_of(SavingsMode::Speculative), 0);
        assert_eq!(reg.verdicts_count(Verdict::Supported), 0);
    }

    #[test]
    fn metrics_registry_counts_error_outcome_via_guard_drop() {
        let reg = Arc::new(MetricsRegistry::new());
        {
            // Garde armée droppée sans record_success → outcome=error.
            let _g = MissionMetricGuard::arm(Some(reg.clone()), MissionType::B);
        }
        assert_eq!(reg.missions_count(MissionType::B, Outcome::Error), 1);
        assert_eq!(reg.missions_count(MissionType::B, Outcome::Success), 0);
    }

    #[test]
    fn guard_record_success_disarms_no_double_count() {
        let reg = Arc::new(MetricsRegistry::new());
        {
            let mut g = MissionMetricGuard::arm(Some(reg.clone()), MissionType::A);
            let rec = MissionRecord::error(MissionType::A); // outcome ignoré : on force success
            let success = MissionRecord {
                outcome: Outcome::Success,
                ..rec
            };
            g.record_success(&success);
        } // Drop : ne doit PAS recompter une erreur (désarmé).
        assert_eq!(reg.missions_count(MissionType::A, Outcome::Success), 1);
        assert_eq!(reg.missions_count(MissionType::A, Outcome::Error), 0);
    }

    #[test]
    fn guard_none_registry_is_inert() {
        // arm(None) → aucun panic, aucun enregistrement.
        let reg = Arc::new(MetricsRegistry::new());
        {
            let _g = MissionMetricGuard::arm(None, MissionType::A);
        }
        // Registre indépendant reste à 0.
        assert_eq!(reg.missions_count(MissionType::A, Outcome::Error), 0);
    }

    #[test]
    fn render_prometheus_is_valid_format() {
        let reg = MetricsRegistry::new();
        reg.record_mission(&MissionRecord {
            ty: MissionType::A,
            outcome: Outcome::Success,
            provider: "claude_sonnet".into(),
            cost_microcents: 558_000,
            phase_latencies: vec![(Phase::Llm, 480)],
            savings: None,
            verdicts: vec![Verdict::Supported, Verdict::Unsupported],
            failure_kinds: vec![FailureKind::LlmFallbackTriggered],
        });
        let circuits = vec![("analyzer".to_string(), SlmCircuitState::Open)];
        let txt = reg.render_prometheus(&circuits);

        assert!(txt.contains("# TYPE efe_missions_total counter"));
        assert!(txt.contains("efe_missions_total{type=\"A\",outcome=\"success\"} 1"));
        assert!(txt.contains("efe_cost_microcents_total{provider=\"claude_sonnet\"} 558000"));
        assert!(txt.contains("# TYPE efe_latency_ms histogram"));
        assert!(txt.contains("efe_latency_ms_bucket{phase=\"llm\",le=\"500\"} 1"));
        assert!(txt.contains("efe_latency_ms_bucket{phase=\"llm\",le=\"+Inf\"} 1"));
        assert!(txt.contains("efe_latency_ms_count{phase=\"llm\"} 1"));
        assert!(txt.contains("efe_failure_mode_total{kind=\"LLM_FALLBACK_TRIGGERED\"} 1"));
        assert!(txt.contains("efe_verdicts_total{verdict=\"SUPPORTED\"} 1"));
        assert!(txt.contains("efe_slm_circuit_state{robot=\"analyzer\",state=\"open\"} 1"));
        assert!(txt.contains("efe_slm_circuit_state{robot=\"analyzer\",state=\"closed\"} 0"));
    }

    #[test]
    fn render_excludes_tenant_id_label() {
        // Anti §4.3 : aucun label tenant ne doit apparaître, jamais.
        let reg = MetricsRegistry::new();
        reg.record_mission(&MissionRecord {
            ty: MissionType::A,
            outcome: Outcome::Success,
            provider: "claude".into(),
            cost_microcents: 1,
            phase_latencies: vec![],
            savings: None,
            verdicts: vec![],
            failure_kinds: vec![],
        });
        let txt = reg.render_prometheus(&[]);
        assert!(!txt.to_lowercase().contains("tenant"), "aucun label tenant autorisé");
    }

    #[test]
    fn failure_kinds_flattens_composite() {
        let mode = FailureMode::Composite(vec![
            FailureMode::SlmDown("analyzer".into()),
            FailureMode::DrafterBypassed("503".into()),
        ]);
        let kinds = failure_kinds_of(&mode);
        assert_eq!(kinds, vec![FailureKind::SlmDown, FailureKind::DrafterBypassed]);
    }

    #[test]
    fn verdict_parse_unknown_is_other() {
        assert_eq!(Verdict::parse("SUPPORTED"), Verdict::Supported);
        assert_eq!(Verdict::parse("WAT"), Verdict::Other);
    }
}
