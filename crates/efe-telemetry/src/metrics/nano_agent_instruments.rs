//! Sprint **D34 (Φ-E)** — Nano-agent template Prometheus instruments.
//!
//! Cf. `docs/EFE_TELEMETRY_EXTENSION_ARCHITECTURE.md` § 10.3.
//!
//! 4 instruments couvrant la diversité de la population D33
//! (8 templates GA : `NaSem`, `NaLog`, `NaCrit`, `NaQual`,
//! `NaCausal`, `NaTemp`, `NaSafety`, `NaConsist`) :
//!
//! | # | Métrique                                       | Type      | Labels                      |
//! |--:|------------------------------------------------|-----------|-----------------------------|
//! | 1 | `efe_nano_agent_population_total`              | Counter   | template_code, tenant       |
//! | 2 | `efe_nano_agent_consensus_weight`              | Gauge     | template_code               |
//! | 3 | `efe_nano_agent_safety_triggered_total`        | Counter   | template_code, tenant       |
//! | 4 | `efe_nano_agent_kernel_routed_total`           | Counter   | kernel_id, template_code    |
//!
//! Cardinalité projetée (architecture § 14.1) :
//!   * `template_code` ∈ 8 valeurs D33 GA set
//!   * `tenant` ∈ 200 max
//!   * `kernel_id` ∈ 8 valeurs D33 (`Semantic`, `Logical`,
//!     `Diversifying`, `Quality`, `Causal`, `Temporal`, `Safety`,
//!     `Consistency`)
//!   → 8 × 200 + 8 + 8 × 200 + 8 × 8 = 3 272 séries (sous le
//!     budget D13 50 000 séries ✅).
//!
//! ## Pattern eager-registration
//!
//! Au boot, `register_all_nano_agent()` appelle l'accessor de
//! chaque instrument (force `OnceLock::get_or_init`) ET pré-peuple
//! la gauge `efe_nano_agent_consensus_weight` avec les valeurs
//! déclarées par les 8 implems du trait `NanoAgentTemplate`. Sans
//! ce pré-peuplement, la gauge afficherait "no data" jusqu'à la
//! première application du poids par le détecteur de consensus —
//! mauvais signal opérateur.
//!
//! ## Wiring status (post-M5 audit T-A3)
//!
//! Les instruments 1, 3, 4 dépendent de **wirings non-encore livrés
//! par contrainte d'architecture** : leurs sites d'émission canoniques
//! vivent dans `efe-core::genesis::spawner` / `agents::templates::NaSafety`
//! / `fusion::MvpCpuFusion`, mais **`efe-core` ne peut pas dépendre de
//! `efe-telemetry`** (cela créerait un cycle puisque
//! `efe-telemetry` dépend déjà de `efe-core`).
//!
//! Trois options pour débloquer ce wiring (toutes hors-scope du
//! sprint M5 D34) :
//!
//!   1. **DT-T-D34-WIRE-1** — Étendre le trait `TelemetryCollector`
//!      (M11 framework, `efe-core::swarm::strategies`) avec des
//!      méthodes `on_template_spawn(...)`, `on_safety_triggered(...)`,
//!      `on_kernel_routed(...)`. L'implem `TracingTelemetryCollector`
//!      d'`efe-telemetry` les wire vers les instruments. Refactor
//!      léger, préserve la direction des dépendances.
//!   2. **DT-T-D34-EVENT-BUS-1** — Créer un canal `RegistryEvent`-style
//!      (cf. `efe-core::alignment::events`) que la genesis spawner
//!      pousse, et qu'un sink `efe-telemetry::events::TemplateSink`
//!      consomme. Pattern déjà éprouvé par M11.
//!   3. **DT-T-D34-CALLBACK-1** — Injecter une `Arc<dyn Fn(...)>`
//!      callback dans `genesis::spawner::Spawner::new(callback)` que
//!      `efe-server::runtime::Server::build` configure pour bumper
//!      les counters. Plus invasif sur l'API publique de spawner.
//!
//! En attendant, ces 3 instruments restent à 0 en production. Les
//! dashboards d34_templates panel "Population by Template",
//! "NaSafety Triggers", "Kernel Routing Heatmap" afficheront "no
//! data". **Cela est intentionnel et safe-by-default** : pas de
//! fausse donnée, pas d'alerte intempestive.
//!
//! L'instrument 2 (`consensus_weight`) est wiré au boot par
//! `register_all_nano_agent()` ; il reflète immédiatement les
//! valeurs déclarées par le trait sans attendre une exécution
//! swarm.
//!
//! ## Audit fix T-A1 (Mode-2/Mode-3 connexes)
//!
//! Les instruments Mode-2 (`mode2_instruments`) et Mode-3
//! (`mode3_instruments`) ne souffrent PAS du même blocage car
//! leurs sites d'émission canoniques vivent dans
//! `efe-server::services::mode2/mod.rs` et
//! `efe-server::services::mode3/mod.rs` — `efe-server` peut
//! dépendre d'`efe-telemetry` sans cycle. Ces wirings ont été
//! livrés dans le M5 audit pass 1 (T-A1).

use std::sync::OnceLock;

use prometheus::{GaugeVec, IntCounterVec, Opts};

use super::registry::global_registry;

/// Internal once-per-call macro identical to D17/D18/D29.
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
// 1. Counter — population spawned per (template, tenant)
// ====================================================================

once_metric!(
    /// `efe_nano_agent_population_total{template_code, tenant}` —
    /// count of nano-agents spawned per template per tenant.
    ///
    /// Émis par `efe-core::genesis::spawner::on_spawn(template,
    /// tenant)` à chaque appel `SwarmController::execute`.
    /// **Wiring** : `DT-T-D34-WIRE-1` post-livraison D34.
    ///
    /// **Use case** : repérer les tenants qui sur-représentent un
    /// template (e.g. quelqu'un qui force `NaQual=80%` rendrait la
    /// pipeline déséquilibrée).
    nano_agent_population_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_nano_agent_population_total",
            "Total nano-agents spawned per template per tenant (D34 § 10.3 #1).",
        ),
        &["template_code", "tenant"],
    )
    .expect("valid counter opts")
);

// ====================================================================
// 2. Gauge — consensus weight per template
// ====================================================================

once_metric!(
    /// `efe_nano_agent_consensus_weight{template_code}` — poids
    /// du template dans le `MultiCriteriaDetector`.
    ///
    /// **Pré-peuplement** : `register_all_nano_agent()` set les 8
    /// valeurs au boot depuis le trait
    /// `efe-core::agents::templates::NanoAgentTemplate::consensus_weight()`
    /// — sans ça la gauge afficherait "no data" jusqu'à la première
    /// exécution swarm.
    ///
    /// **Use case** : panel Grafana "Consensus Weights Table" qui
    /// affiche les 8 valeurs et alerte si elles divergent du contrat
    /// (NaSafety doit toujours être 2.0, etc.).
    nano_agent_consensus_weight,
    GaugeVec,
    GaugeVec::new(
        Opts::new(
            "efe_nano_agent_consensus_weight",
            "Per-template consensus weight applied by the multi-criteria detector (D34 § 10.3 #2).",
        ),
        &["template_code"],
    )
    .expect("valid gauge opts")
);

// ====================================================================
// 3. Counter — NaSafety refusal triggers
// ====================================================================

once_metric!(
    /// `efe_nano_agent_safety_triggered_total{template_code, tenant}`
    /// — count of times `NaSafety` produced a refusal vote that
    /// outweighed positive votes.
    ///
    /// `template_code` est toujours `"na-safety"` mais on garde le
    /// label pour symétrie + future extension (autre safety
    /// templates ?).
    ///
    /// **Wiring** : `DT-T-D34-SAFETY-1` post-livraison D34.
    ///
    /// **SLO produit C-11** : NaSafety triggers ≥ 0.5 % du trafic
    /// (mesuré via `rate(efe_nano_agent_safety_triggered_total[5m])
    /// / rate(efe_grpc_requests_total{method=~".*Execute.*"}[5m])`).
    nano_agent_safety_triggered_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_nano_agent_safety_triggered_total",
            "Total NaSafety refusal triggers per template per tenant (D34 § 10.3 #3).",
        ),
        &["template_code", "tenant"],
    )
    .expect("valid counter opts")
);

// ====================================================================
// 4. Counter — fusion kernel routing per template
// ====================================================================

once_metric!(
    /// `efe_nano_agent_kernel_routed_total{kernel_id, template_code}`
    /// — count of fusion-kernel routing decisions per template.
    ///
    /// **Wiring** : `DT-C-D33-FUSION-KERNEL-1` (M1 audit C-A5
    /// documented). `MvpCpuFusion::dispatch_kernel` lit
    /// `template.fusion_kernel()` et incrémente cet instrument.
    ///
    /// **Use case** : heatmap kernel_id × template_code pour
    /// vérifier que le routing est conforme aux déclarations du
    /// trait (NaSem → Semantic, NaSafety → Safety, etc.).
    nano_agent_kernel_routed_total,
    IntCounterVec,
    IntCounterVec::new(
        Opts::new(
            "efe_nano_agent_kernel_routed_total",
            "Total fusion-kernel routing decisions per template (D34 § 10.3 #4).",
        ),
        &["kernel_id", "template_code"],
    )
    .expect("valid counter opts")
);

// ====================================================================
// Eager registration helper
// ====================================================================

/// Force-register every nano-agent instrument with the global
/// registry. **MUST** be called from `Server::build` before the
/// first gRPC accept.
///
/// **Sprint D34 init step** — pre-populate the
/// `efe_nano_agent_consensus_weight` gauge with the 8 values
/// declared by the `NanoAgentTemplate` trait. Without this, the
/// "Consensus Weights Table" panel in `d34_templates.json`
/// shows "no data" until the first swarm execution applies any
/// weight — misleading for operators triaging weight
/// configuration issues.
pub fn register_all_nano_agent() {
    let _ = nano_agent_population_total();
    let weight_gauge = nano_agent_consensus_weight();
    let _ = nano_agent_safety_triggered_total();
    let _ = nano_agent_kernel_routed_total();

    // **Sprint D34** — pre-populate the consensus_weight gauge with
    // the canonical values declared by the trait. We import the
    // 8 template impls + the trait, iterate, and `set` each weight.
    // The tracing call documents which weights are wired so an
    // operator grepping JSON logs sees them at boot.
    use efe_core::agents::templates::{ga_templates, NanoAgentTemplate};
    for t in ga_templates() {
        let template_code: &str = t.code().as_str();
        weight_gauge
            .with_label_values(&[template_code])
            .set(f64::from(t.consensus_weight()));
        tracing::debug!(
            target: "efe.metrics.nano_agent",
            template = %template_code,
            weight = t.consensus_weight(),
            "consensus_weight gauge pre-populated"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prometheus::core::Collector;

    /// **Audit fix T-A4** — process-wide mutex serialising the
    /// tests that mutate then read the `nano_agent_consensus_weight`
    /// gauge. Cargo runs tests in parallel by default ; without
    /// this gate, two tests both calling `register_all_nano_agent`
    /// then immediately reading specific label values race on the
    /// internal Prometheus mutex and produce occasional false
    /// negatives (1/3 runs). Prefer this over `--test-threads=1`
    /// so other tests can still run in parallel.
    static GAUGE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn register_all_nano_agent_idempotent() {
        let _guard = GAUGE_LOCK.lock().expect("gauge lock poisoned");
        register_all_nano_agent();
        register_all_nano_agent();
    }

    #[test]
    fn population_total_label_arity_is_two() {
        assert_eq!(
            nano_agent_population_total().desc()[0]
                .variable_labels
                .len(),
            2
        );
    }

    #[test]
    fn consensus_weight_label_arity_is_one() {
        assert_eq!(
            nano_agent_consensus_weight().desc()[0]
                .variable_labels
                .len(),
            1
        );
    }

    #[test]
    fn safety_triggered_label_arity_is_two() {
        assert_eq!(
            nano_agent_safety_triggered_total().desc()[0]
                .variable_labels
                .len(),
            2
        );
    }

    #[test]
    fn kernel_routed_label_arity_is_two() {
        assert_eq!(
            nano_agent_kernel_routed_total().desc()[0]
                .variable_labels
                .len(),
            2
        );
    }

    #[test]
    fn consensus_weight_pre_populated_for_all_8_templates() {
        // **Audit anti-regression** — `register_all_nano_agent()`
        // must populate the gauge for every GA template ; otherwise
        // the d34_templates dashboard panel shows "no data" for
        // templates that haven't been used yet.
        let _guard = GAUGE_LOCK.lock().expect("gauge lock poisoned");
        register_all_nano_agent();
        let gauge = nano_agent_consensus_weight();

        for code in [
            "na-sem", "na-log", "na-crit", "na-qual",
            "na-causal", "na-temp", "na-safety", "na-consist",
        ] {
            // `with_label_values` returns the existing series ;
            // we read its current value via `get`. After
            // `register_all_nano_agent`, every series should be
            // > 0 (or = 1.0 for non-overridden templates).
            let v = gauge.with_label_values(&[code]).get();
            assert!(
                v > 0.0,
                "consensus_weight for {code} not pre-populated (got {v})",
            );
        }
    }

    #[test]
    fn na_safety_consensus_weight_is_2_0() {
        // Institutional invariant : NaSafety pulls 2.0 weight.
        let _guard = GAUGE_LOCK.lock().expect("gauge lock poisoned");
        register_all_nano_agent();
        let gauge = nano_agent_consensus_weight();
        let v = gauge.with_label_values(&["na-safety"]).get();
        assert!(
            (v - 2.0).abs() < 1e-6,
            "NaSafety weight = {v} ; expected 2.0"
        );
    }
}
