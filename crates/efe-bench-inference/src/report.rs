// SPDX-License-Identifier: UNLICENSED
//! **B1.5 — la comparaison, le rapport, et le bundle re-prouvable.**
//!
//! [`ConfigReport`] agrège les [`RunRecord`] d'une configuration : exactitude, coût-jetons (estimé
//! vs mesuré), largeur moyenne, latence, fraction R0-vérifiée. [`Comparison`] pose plusieurs configs
//! sur la même grille — **même dataset, même scorer, même comptabilité** (I-BENCH-FAIR) — avec un
//! [`PriceModel`] externe (¢/1k jetons par config) pour passer du coût-jetons brut au coût comparable.
//! Le livrable n'est pas un nombre mais une **courbe** justesse × coût (balayage de K vs un gros
//! modèle de référence) ; la thèse tient si la courbe de l'essaim domine le point de référence.
//!
//! [`BenchBundle`] applique le moat au benchmark (**I-BENCH-REPROVE**) : chaque run mesuré est déjà
//! scellé + R0-re-prouvable ; le bundle lie le digest du dataset, les rapports et une **racine de
//! manifeste** unique. On ne peut ni **cueillir** (le compte par config doit égaler le dataset), ni
//! **retoucher** (la racine de manifeste change), ni **inventer** un run (il manque sa preuve). La
//! re-preuve atteste la **computation** (le run a bien produit cette réponse), pas la **correction**
//! du label (vérité externe — gouvernance du dataset, hors-portée).

use std::collections::BTreeMap;

use serde::Serialize;

use efe_swarm_sdk::verify_run_offline;

use crate::dataset::Dataset;
use crate::runner::{RunEvidence, RunRecord};

/// Le rapport agrégé d'une configuration sur le dataset.
#[derive(Debug, Clone, Serialize)]
pub struct ConfigReport {
    pub config: String,
    /// Étiquette du backend (`scripted` / `selfhost:model@endpoint`) — **contextualise la latence**
    /// (I-BENCH-PROXY) : `mean_elapsed_ms` n'a de sens que pour un self-host réel.
    pub backend: String,
    pub cases: usize,
    pub correct: usize,
    /// Exactitude = correct / cases ∈ [0, 1]. La mesure de tête d'affiche (I-BENCH-TRUTH).
    pub accuracy: f64,
    pub total_tokens: u64,
    pub mean_tokens: f64,
    /// `true` si **au moins un** run a un coût-jetons estimé (proxy). `false` ⇒ tout est mesuré
    /// (un vrai vLLM a rapporté l'usage). I-BENCH-PROXY.
    pub tokens_estimated: bool,
    pub mean_agents: f64,
    pub mean_elapsed_ms: f64,
    /// Nombre de runs R0-vérifiés. Doit égaler `cases` pour un banc propre.
    pub verified: usize,
    pub verified_pct: f64,
    pub records: Vec<RunRecord>,
}

impl ConfigReport {
    /// Agrège des mesures par-cas en un rapport de configuration.
    pub fn aggregate(config: String, records: Vec<RunRecord>) -> Self {
        let cases = records.len();
        let correct = records.iter().filter(|r| r.correct).count();
        let total_tokens: u64 = records.iter().map(|r| r.tokens).sum();
        let verified = records.iter().filter(|r| r.verified).count();
        let total_agents: usize = records.iter().map(|r| r.agents_used).sum();
        let total_elapsed: u64 = records.iter().map(|r| r.elapsed_ms).sum();
        // Estimé si AUCUN run n'est intégralement mesuré (conservateur : un proxy quelque part ⇒
        // on ne prétend pas « mesuré » pour l'agrégat).
        let tokens_estimated = records.is_empty() || records.iter().any(|r| r.tokens_estimated);
        let denom = cases.max(1) as f64;
        Self {
            config,
            backend: String::new(), // posé par run_config_with_evidence (la source réelle).
            cases,
            correct,
            accuracy: correct as f64 / denom,
            total_tokens,
            mean_tokens: total_tokens as f64 / denom,
            tokens_estimated,
            mean_agents: total_agents as f64 / denom,
            mean_elapsed_ms: total_elapsed as f64 / denom,
            verified,
            verified_pct: verified as f64 / denom,
            records,
        }
    }
}

/// Le modèle de prix : ¢ par 1000 jetons, **par configuration**. Hypothèse économique **explicite**
/// (pas mesurée) — un jeton de petit modèle coûte moins qu'un jeton de gros modèle. Rendu visible et
/// rejouable : un lecteur peut refaire le calcul avec sa propre table.
#[derive(Debug, Clone, Default)]
pub struct PriceModel {
    pub cents_per_1k_tokens: BTreeMap<String, f64>,
}

impl PriceModel {
    pub fn new() -> Self {
        Self::default()
    }

    /// Déclare le prix d'une config (chaînable).
    pub fn with(mut self, config: impl Into<String>, cents_per_1k: f64) -> Self {
        self.cents_per_1k_tokens.insert(config.into(), cents_per_1k);
        self
    }

    /// Coût pondéré d'un rapport = jetons × prix/1k. `None` si la config n'a pas de prix déclaré.
    pub fn weighted_cost_cents(&self, report: &ConfigReport) -> Option<f64> {
        self.cents_per_1k_tokens
            .get(&report.config)
            .map(|p| report.total_tokens as f64 / 1000.0 * p)
    }
}

/// Plusieurs configurations posées sur la même grille — la thèse tranchée.
#[derive(Debug, Clone)]
pub struct Comparison {
    pub reports: Vec<ConfigReport>,
    pub price: Option<PriceModel>,
}

impl Comparison {
    pub fn new(reports: Vec<ConfigReport>) -> Self {
        Self { reports, price: None }
    }

    pub fn with_price(mut self, price: PriceModel) -> Self {
        self.price = Some(price);
        self
    }

    /// Une table texte lisible (logs / rapport). Le coût pondéré n'apparaît que si un prix est déclaré.
    pub fn table(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "{:<22} {:>9} {:>8} {:>11} {:>6} {:>9} {:>12}\n",
            "config", "exactitude", "agents", "jetons/moy", "est?", "R0 vérif", "coût pondéré"
        ));
        out.push_str(&"-".repeat(82));
        out.push('\n');
        for r in &self.reports {
            let weighted = self
                .price
                .as_ref()
                .and_then(|p| p.weighted_cost_cents(r))
                .map(|c| format!("{c:.3}¢"))
                .unwrap_or_else(|| "—".to_string());
            out.push_str(&format!(
                "{:<22} {:>8.1}% {:>8.1} {:>11.1} {:>6} {:>8.0}% {:>12}\n",
                truncate(&r.config, 22),
                r.accuracy * 100.0,
                r.mean_agents,
                r.mean_tokens,
                if r.tokens_estimated { "est" } else { "mes" },
                r.verified_pct * 100.0,
                weighted,
            ));
        }
        // I-BENCH-PROXY : la latence n'est pas tabulée ici car elle n'a de sens que sur self-host réel.
        if let Some(first) = self.reports.first() {
            out.push_str(&format!(
                "backend : {} · jetons {} · latence significative seulement en self-host réel\n",
                first.backend,
                if self.reports.iter().any(|r| r.tokens_estimated) { "estimés (proxy)" } else { "mesurés" },
            ));
        }
        out
    }

    /// **La thèse tranchée (archi §7).** Pour chaque config d'essaim, **domine-t-elle** la référence ?
    /// — exactitude **≥** celle de la référence ET coût pondéré **<** celui de la référence (Pareto).
    /// Nécessite un modèle de prix (le coût comparable). Renvoie `(config, domine)` par config non-réf ;
    /// vide si la référence est absente ou sans prix. La thèse tient si au moins une domine.
    pub fn dominates_baseline(&self, baseline: &str) -> Vec<(String, bool)> {
        let price = match &self.price {
            Some(p) => p,
            None => return Vec::new(),
        };
        let base = match self.reports.iter().find(|r| r.config == baseline) {
            Some(b) => b,
            None => return Vec::new(),
        };
        let base_cost = match price.weighted_cost_cents(base) {
            Some(c) => c,
            None => return Vec::new(),
        };
        self.reports
            .iter()
            .filter(|r| r.config != baseline)
            .map(|r| {
                let dominates = price
                    .weighted_cost_cents(r)
                    .map(|c| r.accuracy >= base.accuracy && c < base_cost)
                    .unwrap_or(false);
                (r.config.clone(), dominates)
            })
            .collect()
    }
}

/// Le verdict d'une **re-preuve hors-ligne indépendante** (archi §8.1) : on n'a lu AUCUN drapeau
/// `verified` stocké — chaque run a été re-dérivé depuis ses records bruts + la clé tenant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReproofVerdict {
    /// Nombre de runs (avec évidence) soumis à re-preuve.
    pub rows: usize,
    /// Combien ont re-prouvé intégralement (transaction : essaim + N17 + pont).
    pub fully_proven: usize,
    /// Tous les runs (≥ 1) re-prouvent.
    pub all_proven: bool,
    /// Le barreau d'attestation de computation **le plus faible** re-vérifié sur l'ensemble
    /// (0 = legacy … 4 = R1 déterministe ; R2-loopback = 3).
    pub weakest_rank: Option<u8>,
}

/// **Re-prouve l'évidence d'un bundle, indépendamment et hors-ligne** (I-BENCH-REPROVE réalisé).
///
/// Pour chaque [`RunEvidence`], re-exécute `verify_run_offline(records, n17, leaves, root, key)` — la
/// re-preuve cryptographique COMPLÈTE de la transaction, sans réseau, sans re-inférence, **sans faire
/// confiance** à un quelconque drapeau enregistré. Une clé erronée ⇒ aucune re-preuve (c'est une vraie
/// vérification HMAC, pas un booléen). Atteste la **computation** (le run a bien produit cette
/// réponse, avec cette provenance) ; la **correction du label** reste une confiance externe.
pub fn reprove(evidence: &[RunEvidence], key: &[u8]) -> ReproofVerdict {
    let mut fully_proven = 0usize;
    let mut weakest: Option<u8> = None;
    for ev in evidence {
        let verdict = verify_run_offline(&ev.swarm_records, &ev.llm_records, &ev.n17_leaves, ev.n17_root, key);
        if verdict.fully_proven {
            fully_proven += 1;
        }
        if let Some(rank) = verdict.held_strength.weakest_rank {
            weakest = Some(weakest.map_or(rank, |w| w.min(rank)));
        }
    }
    ReproofVerdict {
        rows: evidence.len(),
        fully_proven,
        all_proven: !evidence.is_empty() && fully_proven == evidence.len(),
        weakest_rank: weakest,
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n.saturating_sub(1)).collect::<String>() + "…"
    }
}

/// Le faisceau re-prouvable : digest du dataset + rapports + racine de manifeste (I-BENCH-REPROVE).
#[derive(Debug, Clone, Serialize)]
pub struct BenchBundle {
    /// Empreinte blake3 du dataset (labels liés).
    pub dataset_digest: String,
    /// Nombre de cas du dataset (référence de comptage anti-cueillette).
    pub cases: usize,
    /// Les rapports scellés (un par configuration).
    pub reports: Vec<ConfigReport>,
    /// Racine de manifeste : blake3 liant le digest + (config, case_id, correct, racines) trié.
    /// Retoucher une réponse, déposer une ligne, ou inventer un run change cette racine.
    pub manifest_root: String,
}

/// Le verdict de re-vérification d'un bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleVerdict {
    /// Tous les runs sont R0-vérifiés.
    pub all_verified: bool,
    /// Chaque config a exactement `cases` runs (aucune cueillette / dépose).
    pub counts_consistent: bool,
    /// Chaque run porte ses racines (provenance + N17).
    pub roots_present: bool,
    /// La racine de manifeste recalculée correspond (aucune retouche).
    pub manifest_root_matches: bool,
    /// Conjonction de tout ce qui précède.
    pub ok: bool,
}

impl BenchBundle {
    /// Scelle un bundle depuis un dataset et ses rapports.
    pub fn seal(dataset: &Dataset, reports: Vec<ConfigReport>) -> Self {
        let dataset_digest = dataset.digest();
        let manifest_root = Self::compute_manifest_root(&dataset_digest, &reports);
        Self { dataset_digest, cases: dataset.len(), reports, manifest_root }
    }

    /// La racine de manifeste : déterministe (tri par config puis par case_id), liant l'identité du
    /// dataset et, pour chaque run, sa justesse et ses racines de preuve.
    fn compute_manifest_root(dataset_digest: &str, reports: &[ConfigReport]) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(dataset_digest.as_bytes());
        let mut reps: Vec<&ConfigReport> = reports.iter().collect();
        reps.sort_by(|a, b| a.config.cmp(&b.config));
        for r in reps {
            hasher.update(r.config.as_bytes());
            let mut recs: Vec<&RunRecord> = r.records.iter().collect();
            recs.sort_by(|a, b| a.case_id.cmp(&b.case_id));
            for rec in recs {
                hasher.update(rec.case_id.as_bytes());
                hasher.update(&[rec.correct as u8]);
                hasher.update(rec.provenance_root.as_deref().unwrap_or("").as_bytes());
                hasher.update(rec.n17_root.as_deref().unwrap_or("").as_bytes());
            }
        }
        hasher.finalize().to_hex().to_string()
    }

    /// Re-vérifie l'intégrité du bundle. Atteste la **computation** (runs vérifiés, comptage, racines,
    /// manifeste intact) ; la **correction des labels** est une confiance externe (gouvernance du
    /// dataset), hors-portée.
    pub fn verify(&self) -> BundleVerdict {
        let all_verified = self.reports.iter().all(|r| r.records.iter().all(|x| x.verified));
        let counts_consistent = self.reports.iter().all(|r| r.records.len() == self.cases);
        let roots_present = self
            .reports
            .iter()
            .all(|r| r.records.iter().all(|x| x.provenance_root.is_some() && x.n17_root.is_some()));
        let manifest_root_matches =
            Self::compute_manifest_root(&self.dataset_digest, &self.reports) == self.manifest_root;
        let ok = all_verified && counts_consistent && roots_present && manifest_root_matches;
        BundleVerdict { all_verified, counts_consistent, roots_present, manifest_root_matches, ok }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn rec(case_id: &str, correct: bool, tokens: u64, agents: usize, verified: bool) -> RunRecord {
        RunRecord {
            case_id: case_id.to_string(),
            correct,
            agents_used: agents,
            n17_calls: agents,
            tokens,
            tokens_estimated: true,
            elapsed_ms: 0,
            spent_cents: 0,
            verified,
            provenance_root: Some("aa".to_string()),
            n17_root: Some("bb".to_string()),
            attestation: Some("R2-LOCAL-TEMOIN".to_string()),
            winner: json!({ "value": "x" }),
        }
    }

    #[test]
    fn aggregate_computes_accuracy_and_cost() {
        let report = ConfigReport::aggregate(
            "consensus".to_string(),
            vec![rec("a", true, 100, 4, true), rec("b", true, 120, 4, true), rec("c", false, 110, 4, true)],
        );
        assert_eq!(report.cases, 3);
        assert_eq!(report.correct, 2);
        assert!((report.accuracy - 2.0 / 3.0).abs() < 1e-9, "exactitude = 2/3");
        assert_eq!(report.total_tokens, 330);
        assert!((report.mean_agents - 4.0).abs() < 1e-9);
        assert!((report.verified_pct - 1.0).abs() < 1e-9);
        assert!(report.tokens_estimated);
    }

    #[test]
    fn price_model_weights_tokens() {
        let report = ConfigReport::aggregate("small".to_string(), vec![rec("a", true, 2000, 8, true)]);
        let price = PriceModel::new().with("small", 0.5); // 0.5¢ / 1k jetons
        assert_eq!(price.weighted_cost_cents(&report), Some(1.0)); // 2000/1000 * 0.5
        assert_eq!(price.weighted_cost_cents(&ConfigReport::aggregate("absent".into(), vec![])), None);
    }

    #[test]
    fn comparison_table_lists_every_config() {
        let a = ConfigReport::aggregate("swarm-k8".to_string(), vec![rec("a", true, 800, 8, true)]);
        let b = ConfigReport::aggregate("baseline-k1".to_string(), vec![rec("a", true, 200, 1, true)]);
        let cmp = Comparison::new(vec![a, b]).with_price(PriceModel::new().with("swarm-k8", 0.2).with("baseline-k1", 2.0));
        let table = cmp.table();
        assert!(table.contains("swarm-k8"));
        assert!(table.contains("baseline-k1"));
        assert!(table.contains("exactitude"));
    }

    #[test]
    fn dominates_baseline_is_accuracy_ge_and_cost_lt() {
        // L'essaim : aussi juste (2/3) que la référence, mais moins de jetons sous un prix favorable.
        let swarm = ConfigReport::aggregate("swarm-k8".to_string(), vec![rec("a", true, 800, 8, true), rec("b", false, 800, 8, true), rec("c", true, 800, 8, true)]);
        // La référence : même justesse (2/3), bien plus de jetons (gros modèle).
        let base = ConfigReport::aggregate("baseline-k1".to_string(), vec![rec("a", true, 5000, 1, true), rec("b", false, 5000, 1, true), rec("c", true, 5000, 1, true)]);
        // Prix : petit modèle 0.2¢/1k, gros 4¢/1k → essaim 2400×0.2/1k vs réf 15000×4/1k.
        let price = PriceModel::new().with("swarm-k8", 0.2).with("baseline-k1", 4.0);
        let cmp = Comparison::new(vec![swarm, base]).with_price(price);
        let verdict = cmp.dominates_baseline("baseline-k1");
        assert_eq!(verdict, vec![("swarm-k8".to_string(), true)], "≥ justesse ET coût pondéré moindre → domine");
        // Sans prix → pas de verdict de coût (vide, honnête).
        assert!(Comparison::new(vec![]).dominates_baseline("baseline-k1").is_empty());
    }

    #[test]
    fn reprove_on_empty_evidence_is_not_proven() {
        // Re-prouver « rien » n'est pas une réussite (all_proven exige ≥ 1 run).
        let v = crate::report::reprove(&[], b"key");
        assert_eq!(v.rows, 0);
        assert!(!v.all_proven);
    }

    #[test]
    fn bundle_verify_detects_tampering_and_cherry_picking() {
        let ds = Dataset::new(vec![
            crate::dataset::BenchCase::new("a", json!({"text":"x"}), json!("f"), crate::scorer::ScorerKind::Exact),
            crate::dataset::BenchCase::new("b", json!({"text":"y"}), json!("s"), crate::scorer::ScorerKind::Exact),
        ]);
        let report = ConfigReport::aggregate("c".to_string(), vec![rec("a", true, 100, 4, true), rec("b", true, 100, 4, true)]);
        let bundle = BenchBundle::seal(&ds, vec![report]);
        let v = bundle.verify();
        assert!(v.ok, "bundle sain : {v:?}");
        assert!(v.all_verified && v.counts_consistent && v.roots_present && v.manifest_root_matches);

        // Cueillette : on dépose une ligne → comptage incohérent (2 cas, 1 run).
        let cherry = ConfigReport::aggregate("c".to_string(), vec![rec("a", true, 100, 4, true)]);
        let bad = BenchBundle::seal(&ds, vec![cherry]);
        assert!(!bad.verify().counts_consistent, "dépose détectée");
        assert!(!bad.verify().ok);

        // Retouche : on altère la racine de manifeste a posteriori → ne correspond plus.
        let mut tampered = bundle.clone();
        tampered.manifest_root = "0".repeat(64);
        assert!(!tampered.verify().manifest_root_matches, "retouche détectée");
        assert!(!tampered.verify().ok);

        // Run non vérifié → bundle non OK (honnête).
        let unverified = ConfigReport::aggregate("c".to_string(), vec![rec("a", true, 100, 4, true), rec("b", true, 100, 4, false)]);
        let nv = BenchBundle::seal(&ds, vec![unverified]);
        assert!(!nv.verify().all_verified);
        assert!(!nv.verify().ok);
    }
}
