// SPDX-License-Identifier: UNLICENSED
//! **Le Terrain — le harnais de FALSIFICATION** (claim H-B, après l'arc HB.1–HB.4).
//!
//! L'arc H-B a montré qu'aucune agrégation déployable ne récupère l'écart sur l'arithmétique. La VRAIE
//! question n'est pas « où est-ce que je gagne ? » (du cherry-pick) mais : **l'agrégation récupère-t-elle
//! un vrai écart K=1 sur QUELQUE type de tâche ?** — avec l'issue par défaut attendue « **non** ». Ce
//! module est l'instrument qui tranche, sans tricher (cf. `docs/reports/EFE_Archi_Terrain_Frontiere`).
//!
//! Trois gardes, gelés AVANT toute issue :
//! - **L'écran de gap** ([`gap_screen`]) — désarme le confondant **plafond** : « essaim = gros parce que
//!   tout est trivial » n'est PAS une récupération. Une cellule n'est ouverte que si le petit-seul (K=1)
//!   est &lt; 90 % ET en écart ≥ 5 pts du gros. C'est la substitution H-A à K=1.
//! - **La récupération en conditions multiples** ([`run_family`]) — non-infériorité d'un **mécanisme
//!   déployable pré-engagé** à un **K\* gelé** vs gros@1, + **puissance ≥ 70 %** (sinon Indéterminé) + pas
//!   un **artefact de fabrication** (si M synthétise) + **survie de Holm**.
//! - **Le contrôle d'erreur de famille** — Holm-Bonferroni inter-cellules, réutilise [`crate::substitution::GapMap`]
//!   **verbatim** (le même que la carte H-A). Les cellules échec/plafond restent au **dénominateur**.
//!
//! Module **pur** : aucune inférence, déterministe, re-prouvable. Le driver (un exemple) produit les
//! [`SampleSet`] par vraie inférence et appelle [`run_family`] ; ici, c'est de l'analyse seule.

use serde::Serialize;

use crate::aggregation::{curve, paired_correctness, symmetry_test, DeployableVerifier, Mechanism, ReasoningWeigher, SampleSet, SymmetryReport};
use crate::substitution::{achieved_power, discordance, non_inferiority, required_n, wilson_ci, GapCell, GapMap, Verdict};

// ─────────────────────────────────────────────────────────────────────────────
//  Constantes PRÉ-ENREGISTRÉES (gelées avant toute issue sur les cellules — I-TERRAIN-PREREG)
// ─────────────────────────────────────────────────────────────────────────────

/// Au-dessus : le petit-seul suffit déjà → **plafond**, pas d'écart à récupérer.
pub const SMALL_ALONE_MAX: f64 = 0.90;
/// Écart minimal `big − small_alone` pour qu'il y ait quelque chose à combler.
pub const GAP_MIN: f64 = 0.05;
/// Sous ce seuil de puissance, le verdict est **Indéterminé**, jamais « récupération ».
pub const POWER_MIN: f64 = 0.70;
/// Au-dessus, une « récupération » par SYNTHÈSE est étiquetée artefact (la médiane invente des réponses).
pub const FAB_MAX: f64 = 0.20;

/// Un mécanisme qui peut **fabriquer** une réponse absente des K (clause d'artefact, §4.4). SetVote est
/// un vote par élément (sélection loyale), PAS une synthèse.
fn is_synthesis(m: Mechanism) -> bool {
    matches!(m, Mechanism::Median | Mechanism::TrimmedMeanNumeric | Mechanism::AuditWeightedMedian)
}

// ─────────────────────────────────────────────────────────────────────────────
//  L'écran de gap (la substitution H-A à K=1, avec les seuils gelés)
// ─────────────────────────────────────────────────────────────────────────────

/// Le verdict de l'écran de gap d'une cellule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenVerdict {
    /// Écart réel à combler (small_alone &lt; 0,90 ET gap ≥ 0,05) → on ouvre le verdict de récupération.
    Valid,
    /// Le petit-seul ≥ 0,90 → plafond ; ni récupération ni victoire (escalade).
    Ceiling,
    /// gap &lt; 0,05 → rien à combler ; déclassée.
    NoGap,
}

/// L'écran de gap (caractérisation hors-ligne, utilise le label — comme H-A) : le petit-SEUL (K=1) vs le
/// gros (K=1), seuils gelés appliqués.
#[derive(Debug, Clone, Serialize)]
pub struct GapScreen {
    pub small_alone: f64,
    pub big: f64,
    pub gap: f64,
    pub n: usize,
    pub power: f64,
    pub n_required: usize,
    pub verdict: ScreenVerdict,
}

/// L'écran de gap d'une cellule : substitution à **K=1** (small_alone = SC@1 = première réponse), seuils
/// pré-enregistrés. `small`/`big` sont les jeux d'échantillons appariés par `case_id`. Point-estimés +
/// puissance (aucun bootstrap : c'est la caractérisation hors-ligne H-A à K=1).
pub fn gap_screen(small: &[SampleSet], big: &[SampleSet], delta: f64, alpha: f64, beta: f64) -> GapScreen {
    let pairs = paired_correctness(small, big, Mechanism::SelfConsistency, 1, None, None);
    let n = pairs.len();
    let den = n.max(1) as f64;
    let small_alone = pairs.iter().filter(|p| p.small_correct[0]).count() as f64 / den;
    let big = pairs.iter().filter(|p| p.big_correct[0]).count() as f64 / den;
    let gap = big - small_alone;
    let d = discordance(&pairs);
    let (p_b, p_c) = (d.big_better as f64 / den, d.small_better as f64 / den);
    let power = achieved_power(n, delta, alpha, p_b, p_c);
    let n_required = required_n(delta, alpha, beta, p_b, p_c);
    let verdict = if small_alone >= SMALL_ALONE_MAX {
        ScreenVerdict::Ceiling
    } else if gap < GAP_MIN {
        ScreenVerdict::NoGap
    } else {
        ScreenVerdict::Valid
    };
    GapScreen { small_alone, big, gap, n, power, n_required, verdict }
}

// ─────────────────────────────────────────────────────────────────────────────
//  La cellule, son résultat, et le verdict de famille
// ─────────────────────────────────────────────────────────────────────────────

/// Une **cellule** du terrain : ses échantillons (petit/gros appariés), son mécanisme déployable
/// **pré-engagé** M, et les seams (filtre/pondérateur) que M consomme éventuellement.
pub struct Cell<'a> {
    pub name: String,
    pub small: &'a [SampleSet],
    pub big: &'a [SampleSet],
    /// Le mécanisme déployable **pré-engagé** (SC pour scalaire, SetVote pour ensemble, …).
    pub mechanism: Mechanism,
    pub verifier: Option<&'a dyn DeployableVerifier>,
    pub weigher: Option<&'a dyn ReasoningWeigher>,
}

/// Le résultat complet d'une cellule.
#[derive(Debug, Clone, Serialize)]
pub struct CellResult {
    pub name: String,
    pub mechanism: String,
    pub k_star: usize,
    pub screen: GapScreen,
    /// La GapCell de récupération (M@K\* vs gros@1) ; `holm_passes` posé par la famille.
    pub recovery: GapCell,
    pub recovery_acc: f64,
    /// Si M synthétise : fraction d'items où il fabrique (sortie ∉ candidats). `0` sinon.
    pub fabrication_frac: f64,
    /// Diversité déployable (fraction distincte moyenne @K\*) — le prédicteur, valable scalaire ET ensemble.
    pub distinct_frac: f64,
    /// Variance numérique moyenne @K\* (NaN pour les ensembles).
    pub dispersion_var: f64,
    /// Diagnostic label-using (asymétrie/biais) — explique, ne décide pas (I-TERRAIN-LABELFENCE).
    pub symmetry: SymmetryReport,
    /// Verdict FINAL : écran Valid ∧ Holm ∧ puissance ≥ 0,70 ∧ ¬artefact-de-fabrication.
    pub recovers: bool,
}

/// Le verdict de **famille** : « X des N cellules pré-enregistrées récupèrent », sous Holm.
#[derive(Debug, Clone, Serialize)]
pub struct FamilyVerdict {
    pub cells: Vec<CellResult>,
    /// Cellules qui **récupèrent** (les 4 conditions).
    pub n_recover: usize,
    /// Cellules « **réglées** » (puissance ≥ 0,70 ET verdict ≠ Indéterminé) — la famille n'est robuste
    /// que si ≥ 2 le sont (sinon « exploratoire »).
    pub n_tuned: usize,
    pub robust: bool,
    pub n_cells: usize,
    pub alpha: f64,
}

impl FamilyVerdict {
    /// Le titre, écrit d'avance : « X des N récupèrent » (+ note de robustesse).
    pub fn headline(&self) -> String {
        let robust = if self.robust { "robuste" } else { "EXPLORATOIRE (< 2 cellules réglées)" };
        format!(
            "{}/{} cellule(s) récupèrent (Holm α={:.2}, {} réglées → {})",
            self.n_recover, self.n_cells, self.alpha, self.n_tuned, robust
        )
    }
}

/// Construit la **GapCell de récupération** d'une cellule : non-infériorité de `M@k_star` vs gros@1,
/// par items appariés, avec IC (Wilson, un item = une obs ⇒ binomial-sur-items exact), puissance et n requis.
fn recovery_cell(c: &Cell, k_star: usize, delta: f64, alpha: f64, beta: f64, boot: usize, seed: u64) -> GapCell {
    let pairs = paired_correctness(c.small, c.big, c.mechanism, k_star, c.verifier, c.weigher);
    let n = pairs.len();
    let den = n.max(1) as f64;
    let small_succ = pairs.iter().filter(|p| p.small_correct[0]).count();
    let big_succ = pairs.iter().filter(|p| p.big_correct[0]).count();
    let nin = non_inferiority(&pairs, delta, alpha, boot, seed);
    let d = discordance(&pairs);
    let (p_b, p_c) = (d.big_better as f64 / den, d.small_better as f64 / den);
    GapCell {
        small_model: format!("{:?}", c.mechanism),
        task: c.name.clone(),
        small_acc: small_succ as f64 / den,
        small_ci: wilson_ci(small_succ as f64, n, alpha),
        big_acc: big_succ as f64 / den,
        big_ci: wilson_ci(big_succ as f64, n, alpha),
        raw_passes: nin.verdict.substitutes(),
        nin: nin.clone(),
        nin_present: nin,
        power: achieved_power(n, delta, alpha, p_b, p_c),
        n_required: required_n(delta, alpha, beta, p_b, p_c),
        holm_passes: false, // posé par GapMap::build
    }
}

/// **Le verdict de famille** : par cellule, l'écran de gap + la récupération@K\* ; Holm-Bonferroni
/// inter-cellules (via [`GapMap::build`]) ; puis le compte « X des N récupèrent » avec la règle de
/// robustesse (≥ 2 cellules réglées). La famille est **fermée** (les cellules échec/plafond comptent au
/// dénominateur — jamais retirées). Déterministe (graines fixes) → re-prouvable.
#[allow(clippy::too_many_arguments)]
pub fn run_family(cells: &[Cell], k_star: usize, delta: f64, alpha: f64, beta: f64, boot: usize, seed: u64) -> FamilyVerdict {
    // 1) GapCells de récupération (M@K\* vs gros@1), une par cellule — graine décorrélée par index.
    let recov: Vec<GapCell> = cells.iter().enumerate().map(|(i, c)| recovery_cell(c, k_star, delta, alpha, beta, boot, seed ^ (0x9E37 * (i as u64 + 1)))).collect();
    // 2) Holm-Bonferroni inter-cellules (réutilise GapMap::build verbatim) → pose holm_passes.
    let map = GapMap::build(recov, alpha);
    // 3) Par cellule : écran + diagnostics + verdict final de récupération.
    let mut results = Vec::with_capacity(cells.len());
    let (mut n_recover, mut n_tuned) = (0usize, 0usize);
    for (c, recov) in cells.iter().zip(map.cells.into_iter()) {
        let screen = gap_screen(c.small, c.big, delta, alpha, beta);
        let cur = curve(c.small, &[k_star], c.verifier, c.weigher);
        let p = &cur[0];
        let fab = if p.fabrication_frac.is_nan() { 0.0 } else { p.fabrication_frac };
        let symmetry = symmetry_test(c.small);

        let powered = recov.power >= POWER_MIN;
        let tuned = powered && recov.nin.verdict != Verdict::Indetermine;
        let not_fab_artifact = !is_synthesis(c.mechanism) || fab <= FAB_MAX;
        let recovers = screen.verdict == ScreenVerdict::Valid && recov.holm_passes && powered && not_fab_artifact;
        if recovers {
            n_recover += 1;
        }
        if tuned {
            n_tuned += 1;
        }
        results.push(CellResult {
            name: c.name.clone(),
            mechanism: format!("{:?}", c.mechanism),
            k_star,
            recovery_acc: recov.small_acc,
            screen,
            recovery: recov,
            fabrication_frac: fab,
            distinct_frac: p.mean_distinct_frac,
            dispersion_var: p.dispersion_var,
            symmetry,
            recovers,
        });
    }
    let n_cells = cells.len();
    FamilyVerdict { cells: results, n_recover, n_tuned, robust: n_tuned >= 2, n_cells, alpha }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scorer::ScorerKind;
    use serde_json::json;

    /// Item numérique : `k` réponses (entiers), label, scorer Numeric exact.
    fn num(case: &str, ans: &[i64], label: i64) -> SampleSet {
        SampleSet::new(case, json!({}), ans.iter().map(|&a| json!(a)).collect(), json!(label), ScorerKind::Numeric { tolerance: 0.0 })
    }

    // ── l'écran de gap : Valid / Ceiling / NoGap ──

    #[test]
    fn gap_screen_classifies_three_regimes() {
        // VALIDE : small_alone = 0.5 (10/20 justes à K=1), big = 1.0 → gap 0.5 ≥ 0.05, 0.5 < 0.90.
        let small: Vec<SampleSet> = (0..20).map(|i| num(&format!("c{i:02}"), &[if i < 10 { 42 } else { 7 }], 42)).collect();
        let big: Vec<SampleSet> = (0..20).map(|i| num(&format!("c{i:02}"), &[42], 42)).collect();
        let sc = gap_screen(&small, &big, 0.05, 0.05, 0.2);
        assert_eq!(sc.verdict, ScreenVerdict::Valid, "{sc:?}");
        assert!((sc.small_alone - 0.5).abs() < 1e-9 && (sc.big - 1.0).abs() < 1e-9);

        // PLAFOND : small_alone = 0.95 (19/20).
        let small_hi: Vec<SampleSet> = (0..20).map(|i| num(&format!("c{i:02}"), &[if i < 19 { 42 } else { 7 }], 42)).collect();
        assert_eq!(gap_screen(&small_hi, &big, 0.05, 0.05, 0.2).verdict, ScreenVerdict::Ceiling);

        // PAS DE GAP : small_alone = 0.80, big = 0.82 → gap 0.02 < 0.05.
        let small_ng: Vec<SampleSet> = (0..50).map(|i| num(&format!("c{i:02}"), &[if i < 40 { 42 } else { 7 }], 42)).collect();
        let big_ng: Vec<SampleSet> = (0..50).map(|i| num(&format!("c{i:02}"), &[if i < 41 { 42 } else { 7 }], 42)).collect();
        assert_eq!(gap_screen(&small_ng, &big_ng, 0.05, 0.05, 0.2).verdict, ScreenVerdict::NoGap);
    }

    // ── le verdict de famille : récupération vs contrôle, Holm, robustesse ──

    #[test]
    fn family_separates_recovery_from_control() {
        // Cellule A (RÉCUPÈRE) : à K=1 small = 0.5 (gap réel), mais SC@3 = 1.0 = gros → non-inférieur.
        //   10 items [42,42,42] (juste à K1 et K3), 10 items [7,42,42] (faux à K1, juste à K3 par majorité).
        let a_small: Vec<SampleSet> = (0..20)
            .map(|i| if i < 10 { num(&format!("a{i:02}"), &[42, 42, 42], 42) } else { num(&format!("a{i:02}"), &[7, 42, 42], 42) })
            .collect();
        let a_big: Vec<SampleSet> = (0..20).map(|i| num(&format!("a{i:02}"), &[42], 42)).collect();
        // Cellule B (CONTRÔLE, ne récupère pas) : small 0.5 à K1 ET K3 (jamais comblé).
        let b_small: Vec<SampleSet> = (0..20)
            .map(|i| if i < 10 { num(&format!("b{i:02}"), &[42, 42, 42], 42) } else { num(&format!("b{i:02}"), &[7, 7, 7], 42) })
            .collect();
        let b_big: Vec<SampleSet> = (0..20).map(|i| num(&format!("b{i:02}"), &[42], 42)).collect();

        let cells = vec![
            Cell { name: "A".into(), small: &a_small, big: &a_big, mechanism: Mechanism::SelfConsistency, verifier: None, weigher: None },
            Cell { name: "B".into(), small: &b_small, big: &b_big, mechanism: Mechanism::SelfConsistency, verifier: None, weigher: None },
        ];
        let fam = run_family(&cells, 3, 0.05, 0.05, 0.2, 1000, 7);
        assert_eq!(fam.n_cells, 2);
        assert!(fam.cells[0].recovers, "A récupère : {:?}", fam.cells[0].recovery.nin.verdict);
        assert!(!fam.cells[1].recovers, "B ne récupère pas (toujours inférieur)");
        assert_eq!(fam.n_recover, 1, "exactement 1 des 2 récupère");
        assert!(fam.cells[0].screen.verdict == ScreenVerdict::Valid && fam.cells[1].screen.verdict == ScreenVerdict::Valid, "les deux ont un gap réel");
        // Déterminisme.
        let fam2 = run_family(&cells, 3, 0.05, 0.05, 0.2, 1000, 7);
        assert_eq!(fam.n_recover, fam2.n_recover);
        assert_eq!(fam.cells[0].recovery.nin.p_value, fam2.cells[0].recovery.nin.p_value);
    }

    #[test]
    fn ceiling_cell_does_not_recover_even_if_non_inferior() {
        // Cellule plafond : small-seul déjà à 1.0 = gros. Non-inférieure trivialement, MAIS l'écran la
        // déclasse (Ceiling) → NE récupère PAS (le confondant plafond est désarmé).
        let small: Vec<SampleSet> = (0..20).map(|i| num(&format!("c{i:02}"), &[42, 42, 42], 42)).collect();
        let big: Vec<SampleSet> = (0..20).map(|i| num(&format!("c{i:02}"), &[42], 42)).collect();
        let cells = vec![Cell { name: "triv".into(), small: &small, big: &big, mechanism: Mechanism::SelfConsistency, verifier: None, weigher: None }];
        let fam = run_family(&cells, 3, 0.05, 0.05, 0.2, 500, 5);
        assert_eq!(fam.cells[0].screen.verdict, ScreenVerdict::Ceiling, "small-seul ≥ 0.90");
        assert!(!fam.cells[0].recovers, "plafond ⇒ pas une récupération, même si non-inférieur");
        assert_eq!(fam.n_recover, 0);
    }

    #[test]
    fn set_cell_recovers_via_set_vote() {
        // Cellule ENSEMBLE : à K=1 small échoue (chaque candidat oublie un élément → small_alone bas),
        // mais le VOTE PAR ÉLÉMENT reconstruit l'ensemble complet → récupération sur la cellule-ensemble.
        let mk = |case: &str, c0: &[&str], c1: &[&str], c2: &[&str], label: &[&str]| {
            SampleSet::new(
                case,
                json!({}),
                vec![json!(c0.iter().collect::<Vec<_>>()), json!(c1.iter().collect::<Vec<_>>()), json!(c2.iter().collect::<Vec<_>>())],
                json!(label.iter().collect::<Vec<_>>()),
                ScorerKind::Set,
            )
        };
        // 20 items, chacun : 3 candidats oubliant chacun un élément distinct de {a,b,c} → K1 faux, SetVote {a,b,c} juste.
        let small: Vec<SampleSet> = (0..20).map(|i| mk(&format!("s{i:02}"), &["a", "b"], &["b", "c"], &["a", "c"], &["a", "b", "c"])).collect();
        let big: Vec<SampleSet> = (0..20)
            .map(|i| SampleSet::new(format!("s{i:02}"), json!({}), vec![json!(["a", "b", "c"])], json!(["a", "b", "c"]), ScorerKind::Set))
            .collect();
        let cells = vec![Cell { name: "T-EXTRACT".into(), small: &small, big: &big, mechanism: Mechanism::SetVote, verifier: None, weigher: None }];
        let fam = run_family(&cells, 3, 0.05, 0.05, 0.2, 1000, 11);
        assert_eq!(fam.cells[0].screen.verdict, ScreenVerdict::Valid, "K=1 échoue → écart réel : {:?}", fam.cells[0].screen);
        assert!(fam.cells[0].recovers, "le vote par élément récupère l'ensemble complet → non-infériorité");
    }
}
