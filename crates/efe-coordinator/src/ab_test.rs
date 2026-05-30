// SPDX-License-Identifier: UNLICENSED
//! **R3.S8 — A/B test entre tables de scoring (bucketing déterministe).**
//!
//! La `TaskClassScoringTable` (calibration figée) décide vers quel LLM router
//! chaque `task_class`. Pour tester une **nouvelle** table en prod sans risque,
//! ce module route un pourcentage du trafic vers une table d'expérimentation et
//! permet de comparer empiriquement les deux bras (via `SavingsReport`).
//!
//! ## Design (cf. architecture R3.S8)
//!
//! - **Bucketing déterministe** : `blake3(swarm_id) % 100` — zéro dépendance
//!   `rand` (blake3 est déjà tiré par l'audit chain), distribution **uniforme**
//!   (vs `uuid_v7 % 100` qui biaiserait : les bits de poids fort de v7 sont
//!   temporels), **reproductible** (même `swarm_id` → même bras).
//! - **Précédence** : dans `run_mission_a`, l'A/B s'applique uniquement si la
//!   requête n'épingle pas de table → `request > A/B > tenant (R3.S5) > global`.
//! - **Config sérialisable vs runtime rapide** : [`ABTestConfig`] (tables
//!   *owned*, `Serialize`) est l'entrée user ; [`ABTestRuntime`] pré-emballe
//!   chaque table dans un `Arc` une fois → résolution O(1) par mission (pas de
//!   clone profond de HashMaps).
//! - **Mission A uniquement** : seule mission avec une table/router. B
//!   (fan-out) et Speculative (drafter/verifier fixés) n'utilisent pas la table.
//! - **Backward-compat** : `ab_test: None` ⇒ comportement R3.S7 inchangé.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use efe_nano_app_coordinator::types::TaskClassScoringTable;
use efe_nano_app_coordinator::TenantId;

/// Bras d'un A/B test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ABArm {
    /// Table de contrôle (v1, prod actuelle).
    Control,
    /// Table d'expérimentation (v2, candidate).
    Experiment,
}

impl ABArm {
    /// Label court (`"control"` / `"experiment"`), pour report / métriques.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Control => "control",
            Self::Experiment => "experiment",
        }
    }
}

/// Configuration A/B **sérialisable** (entrée user). Tables *owned*.
///
/// Convertie une fois en [`ABTestRuntime`] par
/// [`crate::CoordinatorBuilder::ab_test`] pour une résolution sans clone.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ABTestConfig {
    /// Table de contrôle (v1).
    pub control_table: TaskClassScoringTable,
    /// Table d'expérimentation (v2).
    pub experiment_table: TaskClassScoringTable,
    /// Fraction du trafic routée vers l'expérience ∈ [0,1] (clampée).
    pub experiment_traffic_ratio: f32,
    /// Si `Some`, seul ce tenant est dans la population A/B ; les autres
    /// retombent sur la résolution normale (R3.S5).
    #[serde(default)]
    pub tenant_filter: Option<TenantId>,
    /// Si `Some`, seules ces fixtures (harness de report) sont dans la
    /// population. En live (pas d'identité fixture), une mission n'est dans la
    /// population que si ce filtre est `None`.
    #[serde(default)]
    pub fixture_filter: Option<Vec<String>>,
}

/// État runtime : tables pré-emballées en `Arc` (zéro clone profond/mission).
#[derive(Debug, Clone)]
pub struct ABTestRuntime {
    control: Arc<TaskClassScoringTable>,
    experiment: Arc<TaskClassScoringTable>,
    ratio: f32,
    tenant_filter: Option<TenantId>,
    fixture_filter: Option<Vec<String>>,
}

impl ABTestRuntime {
    /// Construit le runtime depuis une config (Arc-wrap une fois ; ratio clampé).
    #[must_use]
    pub fn from_config(cfg: ABTestConfig) -> Self {
        Self {
            control: Arc::new(cfg.control_table),
            experiment: Arc::new(cfg.experiment_table),
            ratio: cfg.experiment_traffic_ratio.clamp(0.0, 1.0),
            tenant_filter: cfg.tenant_filter,
            fixture_filter: cfg.fixture_filter,
        }
    }

    /// Ratio d'expérience effectif (clampé). Pour introspection / tests.
    #[must_use]
    pub fn ratio(&self) -> f32 {
        self.ratio
    }

    /// Bucket déterministe ∈ [0, 99] depuis `swarm_id` (blake3, §4.2).
    ///
    /// `blake3` casse la structure temporelle d'un UUID v7 → distribution
    /// uniforme. Fonction **pure** : reproductible (même `swarm_id` → même
    /// bucket).
    #[must_use]
    pub fn bucket_of(swarm_id: Uuid) -> u64 {
        let h = blake3::hash(swarm_id.as_bytes());
        let b = h.as_bytes();
        let n = u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
        n % 100
    }

    /// Assigne un bras, ou `None` si la mission n'est **pas** dans la population
    /// (tenant/fixture hors filtre) — dans ce cas la résolution normale (R3.S5)
    /// reprend la main.
    ///
    /// - `tenant_filter = Some(t)` : seul `t` est expérimenté.
    /// - `fixture_filter = Some(list)` : un nom de fixture doit être fourni ET
    ///   présent dans `list`. En live, `fixture = None` ⇒ exclusion (on ne peut
    ///   pas confirmer l'appartenance).
    /// - `ratio = 0.0` ⇒ toujours `Control` ; `ratio = 1.0` ⇒ toujours
    ///   `Experiment`.
    #[must_use]
    pub fn assign_arm(
        &self,
        swarm_id: Uuid,
        tenant: TenantId,
        fixture: Option<&str>,
    ) -> Option<ABArm> {
        if let Some(ft) = self.tenant_filter {
            if ft != tenant {
                return None;
            }
        }
        if let Some(ff) = &self.fixture_filter {
            match fixture {
                Some(name) if ff.iter().any(|f| f == name) => {}
                _ => return None,
            }
        }
        let bucket = Self::bucket_of(swarm_id);
        Some(if (bucket as f32) < self.ratio * 100.0 {
            ABArm::Experiment
        } else {
            ABArm::Control
        })
    }

    /// Table (`Arc`) du bras — clone d'un compteur atomique, pas de clone
    /// profond.
    #[must_use]
    pub fn table_for(&self, arm: ABArm) -> Arc<TaskClassScoringTable> {
        match arm {
            ABArm::Control => self.control.clone(),
            ABArm::Experiment => self.experiment.clone(),
        }
    }

    /// Version de la table d'un bras (traçabilité report).
    #[must_use]
    pub fn version_of(&self, arm: ABArm) -> String {
        match arm {
            ABArm::Control => self.control.version.clone(),
            ABArm::Experiment => self.experiment.version.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Table minimale taguée par version (les poids importent peu pour les
    /// tests de bucketing).
    fn tagged_table(version: &str) -> TaskClassScoringTable {
        TaskClassScoringTable {
            version: version.to_string(),
            ..TaskClassScoringTable::default()
        }
    }

    fn config(ratio: f32) -> ABTestConfig {
        ABTestConfig {
            control_table: tagged_table("ctrl-v1"),
            experiment_table: tagged_table("exp-v2"),
            experiment_traffic_ratio: ratio,
            tenant_filter: None,
            fixture_filter: None,
        }
    }

    #[test]
    fn ab_test_config_serializes_stable() {
        let cfg = ABTestConfig {
            tenant_filter: Some(Uuid::nil()),
            fixture_filter: Some(vec!["01_factual".into()]),
            ..config(0.25)
        };
        let s = serde_json::to_string(&cfg).unwrap();
        let back: ABTestConfig = serde_json::from_str(&s).unwrap();
        assert!((back.experiment_traffic_ratio - 0.25).abs() < 1e-6);
        assert_eq!(back.tenant_filter, Some(Uuid::nil()));
        assert_eq!(back.fixture_filter.as_deref(), Some(&["01_factual".to_string()][..]));
        assert_eq!(back.control_table.version, "ctrl-v1");
        assert_eq!(back.experiment_table.version, "exp-v2");
    }

    #[test]
    fn ab_test_assigns_buckets_deterministically() {
        let rt = ABTestRuntime::from_config(config(0.5));
        let sid = Uuid::from_u128(42);
        let first = rt.assign_arm(sid, Uuid::nil(), None);
        // Même swarm_id → même bras, 1000 fois.
        for _ in 0..1_000 {
            assert_eq!(rt.assign_arm(sid, Uuid::nil(), None), first);
        }
        // bucket_of pur : reproductible.
        assert_eq!(ABTestRuntime::bucket_of(sid), ABTestRuntime::bucket_of(sid));
    }

    #[test]
    fn ab_test_bucket_distribution_matches_ratio() {
        let rt = ABTestRuntime::from_config(config(0.30));
        let n = 10_000u128;
        let exp = (0..n)
            .filter(|&i| rt.assign_arm(Uuid::from_u128(i), Uuid::nil(), None) == Some(ABArm::Experiment))
            .count();
        let freq = exp as f32 / n as f32;
        assert!(
            (0.27..=0.33).contains(&freq),
            "fréquence experiment {freq} hors [0.27, 0.33]"
        );
    }

    #[test]
    fn ab_test_traffic_ratio_zero_skips_experiment() {
        let rt = ABTestRuntime::from_config(config(0.0));
        for i in 0..500u128 {
            assert_eq!(
                rt.assign_arm(Uuid::from_u128(i), Uuid::nil(), None),
                Some(ABArm::Control),
                "ratio 0 → toujours Control"
            );
        }
    }

    #[test]
    fn ab_test_traffic_ratio_one_always_experiment() {
        let rt = ABTestRuntime::from_config(config(1.0));
        for i in 0..500u128 {
            assert_eq!(
                rt.assign_arm(Uuid::from_u128(i), Uuid::nil(), None),
                Some(ABArm::Experiment),
                "ratio 1 → toujours Experiment"
            );
        }
    }

    #[test]
    fn ab_test_with_tenant_filter() {
        let target = Uuid::from_u128(7);
        let other = Uuid::from_u128(8);
        let cfg = ABTestConfig {
            tenant_filter: Some(target),
            ..config(1.0)
        };
        let rt = ABTestRuntime::from_config(cfg);
        // Tenant ciblé → dans la population (ratio 1 → Experiment).
        assert_eq!(
            rt.assign_arm(Uuid::from_u128(1), target, None),
            Some(ABArm::Experiment)
        );
        // Tenant hors filtre → None (résolution R3.S5 reprend).
        assert_eq!(rt.assign_arm(Uuid::from_u128(1), other, None), None);
    }

    #[test]
    fn ab_test_fixture_filter_excludes_when_no_name() {
        let cfg = ABTestConfig {
            fixture_filter: Some(vec!["keep".into()]),
            ..config(1.0)
        };
        let rt = ABTestRuntime::from_config(cfg);
        // Nom présent dans le filtre → population.
        assert_eq!(
            rt.assign_arm(Uuid::from_u128(1), Uuid::nil(), Some("keep")),
            Some(ABArm::Experiment)
        );
        // Nom absent → exclu.
        assert_eq!(rt.assign_arm(Uuid::from_u128(1), Uuid::nil(), Some("other")), None);
        // Pas de nom (live) → exclu (appartenance non confirmable).
        assert_eq!(rt.assign_arm(Uuid::from_u128(1), Uuid::nil(), None), None);
    }

    #[test]
    fn ab_test_table_for_returns_arm_table() {
        let rt = ABTestRuntime::from_config(config(0.5));
        assert_eq!(rt.table_for(ABArm::Control).version, "ctrl-v1");
        assert_eq!(rt.table_for(ABArm::Experiment).version, "exp-v2");
        assert_eq!(rt.version_of(ABArm::Experiment), "exp-v2");
    }

    #[test]
    fn ratio_is_clamped() {
        assert!((ABTestRuntime::from_config(config(5.0)).ratio() - 1.0).abs() < 1e-6);
        assert!((ABTestRuntime::from_config(config(-3.0)).ratio() - 0.0).abs() < 1e-6);
    }
}
