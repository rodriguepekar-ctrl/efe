// SPDX-License-Identifier: UNLICENSED
//! **H-A — Substitution : un petit modèle vaut-il un gros ?** (couche d'analyse pure sur B1.)
//!
//! Ce module ne fait AUCUNE inférence : il prend les justesses **par item** que B1 produit déjà
//! ([`ConfigReport::records`] → `RunRecord { case_id, correct, present }`) pour le petit et le gros
//! modèle sur le **même** dataset, et tranche la **non-infériorité appariée** — la seule méthode qui
//! résiste à un sceptique (cf. l'architecture H-A).
//!
//! Les trois erreurs évitées par construction :
//! - **supériorité ≠ non-infériorité** : on n'établit pas « le petit est meilleur » mais « le petit
//!   n'est pas pire de plus d'une marge δ » — borne de confiance SUPÉRIEURE de l'écart < δ ([`Verdict`]).
//! - **« non significatif » ≠ « égal »** : sous-puissance ⇒ verdict [`Verdict::Indetermine`], jamais
//!   « non-inférieur » par défaut (I-HA-POWER, [`achieved_power`] / [`required_n`]).
//! - **non apparié = puissance gaspillée** : on compare les MÊMES items (la variance vient des
//!   discordants), via un **bootstrap apparié au niveau item** (cluster) — déterministe (graine
//!   fixe) donc le verdict statistique est lui-même **re-prouvable** (I-HA-REPROVE).
//!
//! Le livrable est une [`GapMap`] {modèle × tâche} avec correction de multiplicité (Holm-Bonferroni,
//! I-HA-MULTIPLE) : où le petit substitue déjà, où l'écart résiduel est le budget de H-B, où c'est
//! hors de portée.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::report::ConfigReport;

// ─────────────────────────────────────────────────────────────────────────────
//  Erreurs
// ─────────────────────────────────────────────────────────────────────────────

/// Erreur d'appariement (I-HA-PAIRED : mêmes items des deux côtés).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubstitutionError {
    /// Aucun rapport fourni d'un côté.
    Empty(&'static str),
    /// Les graines d'un même modèle ne portent pas le même ensemble de `case_id` (ou doublons).
    SeedMismatch { side: &'static str, detail: String },
    /// Les ensembles de `case_id` du petit et du gros diffèrent (comparaison non appariée).
    CaseMismatch { detail: String },
}

impl std::fmt::Display for SubstitutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SubstitutionError::Empty(s) => write!(f, "aucun rapport pour le {s}"),
            SubstitutionError::SeedMismatch { side, detail } => write!(f, "graines incohérentes ({side}) : {detail}"),
            SubstitutionError::CaseMismatch { detail } => write!(f, "items non appariés petit/gros : {detail}"),
        }
    }
}
impl std::error::Error for SubstitutionError {}

// ─────────────────────────────────────────────────────────────────────────────
//  Appariement par item
// ─────────────────────────────────────────────────────────────────────────────

/// Une paire appariée : les justesses **par graine** des deux modèles sur le MÊME item. La justesse
/// de l'item (moyennée sur ses graines) est `*_acc()`; l'écart par-item est `diff()`.
#[derive(Debug, Clone)]
pub struct PairedItem {
    pub case_id: String,
    /// Justesse du petit, une valeur par graine (∈ {false,true} par run).
    pub small_correct: Vec<bool>,
    /// Justesse du gros, une valeur par graine.
    pub big_correct: Vec<bool>,
    /// Présence (réponse non vide) du petit, par graine.
    pub small_present: Vec<bool>,
    /// Présence du gros, par graine.
    pub big_present: Vec<bool>,
}

impl PairedItem {
    /// Exactitude moyenne du petit sur cet item (sur ses graines).
    pub fn small_acc(&self) -> f64 {
        mean_bool(&self.small_correct)
    }
    /// Exactitude moyenne du gros sur cet item.
    pub fn big_acc(&self) -> f64 {
        mean_bool(&self.big_correct)
    }
    /// Écart par-item Δ_item = exactitude(gros) − exactitude(petit) ∈ [−1, 1].
    pub fn diff(&self) -> f64 {
        self.big_acc() - self.small_acc()
    }
    /// Mono-graine des deux côtés (condition d'un McNemar EXACT).
    fn single_seed(&self) -> bool {
        self.small_correct.len() == 1 && self.big_correct.len() == 1
    }
}

fn mean_bool(v: &[bool]) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.iter().filter(|&&b| b).count() as f64 / v.len() as f64
}

/// `case_id → (corrects par graine, présents par graine)` pour un côté.
type SideMap = BTreeMap<String, (Vec<bool>, Vec<bool>)>;

/// Collecte un côté (toutes ses graines) en `case_id → (corrects, présents)` (une entrée par graine).
/// Échoue si les graines ne portent pas exactement le même ensemble d'items (anti-désalignement).
fn collect_side(reports: &[ConfigReport], side: &'static str) -> Result<SideMap, SubstitutionError> {
    if reports.is_empty() {
        return Err(SubstitutionError::Empty(side));
    }
    let mut map: SideMap = BTreeMap::new();
    for rec in &reports[0].records {
        // Doublon de case_id dans un même rapport → ambigu.
        if map.insert(rec.case_id.clone(), (Vec::new(), Vec::new())).is_some() {
            return Err(SubstitutionError::SeedMismatch { side, detail: format!("case_id en double : {}", rec.case_id) });
        }
    }
    for (si, rep) in reports.iter().enumerate() {
        if rep.records.len() != map.len() {
            return Err(SubstitutionError::SeedMismatch {
                side,
                detail: format!("graine {si} a {} items, attendu {}", rep.records.len(), map.len()),
            });
        }
        for rec in &rep.records {
            match map.get_mut(&rec.case_id) {
                Some(e) => {
                    e.0.push(rec.correct);
                    e.1.push(rec.present);
                }
                None => {
                    return Err(SubstitutionError::SeedMismatch { side, detail: format!("case_id inconnu : {}", rec.case_id) })
                }
            }
        }
    }
    // Chaque item doit avoir reçu exactement une valeur par graine.
    for (id, (c, _)) in &map {
        if c.len() != reports.len() {
            return Err(SubstitutionError::SeedMismatch { side, detail: format!("item {id} : {} valeurs, attendu {}", c.len(), reports.len()) });
        }
    }
    Ok(map)
}

/// Apparie le petit et le gros **par `case_id`** (I-HA-PAIRED). Chaque slice est l'ensemble des
/// **graines** (répétitions) d'un modèle sur le MÊME dataset ; les graines sont moyennées par item.
/// Les ensembles d'items du petit et du gros doivent coïncider exactement.
pub fn pair_by_case(small: &[ConfigReport], big: &[ConfigReport]) -> Result<Vec<PairedItem>, SubstitutionError> {
    let sm = collect_side(small, "petit")?;
    let bg = collect_side(big, "gros")?;
    if sm.len() != bg.len() {
        return Err(SubstitutionError::CaseMismatch { detail: format!("{} items (petit) vs {} (gros)", sm.len(), bg.len()) });
    }
    let mut pairs = Vec::with_capacity(sm.len());
    for (id, (sc, sp)) in &sm {
        let (bc, bp) = bg
            .get(id)
            .ok_or_else(|| SubstitutionError::CaseMismatch { detail: format!("item {id} absent côté gros") })?;
        pairs.push(PairedItem {
            case_id: id.clone(),
            small_correct: sc.clone(),
            big_correct: bc.clone(),
            small_present: sp.clone(),
            big_present: bp.clone(),
        });
    }
    Ok(pairs)
}

/// Restreint chaque item aux graines où **les deux** modèles ont produit une réponse présente
/// (I-HA-PRESENT : la comparaison de **capacité** conditionnée au succès de **format**). Les items
/// sans aucune graine doublement-présente sont retirés.
pub fn condition_on_present(pairs: &[PairedItem]) -> Vec<PairedItem> {
    pairs
        .iter()
        .filter_map(|p| {
            let n = p.small_correct.len().min(p.big_correct.len());
            let mut sc = Vec::new();
            let mut bc = Vec::new();
            for i in 0..n {
                if *p.small_present.get(i).unwrap_or(&false) && *p.big_present.get(i).unwrap_or(&false) {
                    sc.push(p.small_correct[i]);
                    bc.push(p.big_correct[i]);
                }
            }
            if sc.is_empty() {
                None
            } else {
                Some(PairedItem {
                    case_id: p.case_id.clone(),
                    small_present: vec![true; sc.len()],
                    big_present: vec![true; sc.len()],
                    small_correct: sc,
                    big_correct: bc,
                })
            }
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
//  Structure de discordance (McNemar)
// ─────────────────────────────────────────────────────────────────────────────

/// La structure de concordance **au niveau item** : qui gagne sur chaque item (la granularité du
/// bootstrap cluster). `discordant = big_better + small_better` porte toute la variance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Discordance {
    pub big_better: usize,
    pub small_better: usize,
    pub tie: usize,
}
impl Discordance {
    pub fn discordant(&self) -> usize {
        self.big_better + self.small_better
    }
}

/// Compte la discordance au niveau item.
pub fn discordance(pairs: &[PairedItem]) -> Discordance {
    let mut d = Discordance { big_better: 0, small_better: 0, tie: 0 };
    for p in pairs {
        let diff = p.diff();
        if diff > 1e-12 {
            d.big_better += 1;
        } else if diff < -1e-12 {
            d.small_better += 1;
        } else {
            d.tie += 1;
        }
    }
    d
}

/// Table de McNemar **exacte** (mono-graine seulement) : a=tous deux corrects, b=gros correct &
/// petit faux, c=gros faux & petit correct, d=tous deux faux. `None` si une graine multiple rend la
/// table fractionnaire (le bootstrap reste l'autorité — voir [`non_inferiority`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct McNemar {
    pub a: usize,
    pub b: usize,
    pub c: usize,
    pub d: usize,
}
pub fn mcnemar(pairs: &[PairedItem]) -> Option<McNemar> {
    let mut m = McNemar { a: 0, b: 0, c: 0, d: 0 };
    for p in pairs {
        if !p.single_seed() {
            return None;
        }
        match (p.small_correct[0], p.big_correct[0]) {
            (true, true) => m.a += 1,
            (false, true) => m.b += 1,
            (true, false) => m.c += 1,
            (false, false) => m.d += 1,
        }
    }
    Some(m)
}

// ─────────────────────────────────────────────────────────────────────────────
//  Non-infériorité (le cœur)
// ─────────────────────────────────────────────────────────────────────────────

/// Le verdict de substitution sur une tâche, à marge δ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Le petit est CONFIDEMMENT meilleur (borne sup de Δ < 0).
    Superieur,
    /// Le petit n'est pas pire de plus de δ (borne sup de Δ < δ) — **substitue**.
    NonInferieur,
    /// L'intervalle chevauche δ : sous-puissance ou écart proche de δ → plus de données.
    Indetermine,
    /// Le petit est CONFIDEMMENT pire de plus de δ (borne inf de Δ > δ) — substitution réfutée.
    Inferieur,
}
impl Verdict {
    /// Le petit substitue-t-il (non-inférieur ou supérieur) ?
    pub fn substitutes(&self) -> bool {
        matches!(self, Verdict::Superieur | Verdict::NonInferieur)
    }
    pub fn label(&self) -> &'static str {
        match self {
            Verdict::Superieur => "supérieur",
            Verdict::NonInferieur => "non-inférieur",
            Verdict::Indetermine => "indéterminé",
            Verdict::Inferieur => "inférieur",
        }
    }
}

/// Le résultat complet d'un test de non-infériorité apparié.
#[derive(Debug, Clone, Serialize)]
pub struct NonInferiority {
    /// Marge d'indifférence δ (perte de qualité max tolérée pour appeler « substituable »).
    pub delta: f64,
    /// Niveau α (test une-face).
    pub alpha: f64,
    /// Écart ponctuel Δ̂ = exactitude(gros) − exactitude(petit).
    pub diff_point: f64,
    /// Borne de confiance INFÉRIEure (one-sided, niveau α) de Δ — détecte la supériorité.
    pub ci_lower: f64,
    /// Borne de confiance SUPÉRIEure (one-sided, niveau α) de Δ — décide la non-infériorité.
    pub ci_upper: f64,
    /// p-valeur bootstrap du test de non-infériorité (H0 : Δ ≥ δ) = fraction des Δ* ≥ δ.
    pub p_value: f64,
    /// Le verdict.
    pub verdict: Verdict,
    /// Nombre d'items appariés.
    pub n: usize,
    /// Items discordants (big_better + small_better) — le moteur de la variance.
    pub discordant: usize,
    /// Itérations de bootstrap.
    pub boot_iters: usize,
    /// Graine du bootstrap — l'intervalle est re-dérivable à l'identique (I-HA-REPROVE).
    pub boot_seed: u64,
}

/// Teste la non-infériorité du petit vs le gros, à marge δ, niveau α, par **bootstrap apparié
/// déterministe au niveau item** (cluster, **méthode du percentile**). Re-échantillonne les items
/// avec remise ; chaque item porte toutes ses graines (préserve la corrélation intra-item). Les
/// bornes sont les percentiles one-sided (α, 1−α) ; la p-valeur de non-infériorité (H0 : Δ ≥ δ) est
/// la fraction des ré-échantillons ≥ δ. La couverture du percentile-bootstrap a été vérifiée ≈ 1−α
/// par Monte-Carlo (revue adversariale). Verdict selon la position de l'intervalle vs 0 et δ.
pub fn non_inferiority(pairs: &[PairedItem], delta: f64, alpha: f64, boot_iters: usize, seed: u64) -> NonInferiority {
    let n = pairs.len();
    let disc = discordance(pairs).discordant();
    let diffs: Vec<f64> = pairs.iter().map(|p| p.diff()).collect();
    let diff_point = mean(&diffs);

    // Bootstrap cluster : ré-échantillonne les INDICES d'items avec remise.
    let mut rng = Rng::new(seed);
    let mut boots: Vec<f64> = Vec::with_capacity(boot_iters.max(1));
    if n == 0 {
        // Aucun item : tout est indéterminé, sans paniquer.
        return NonInferiority {
            delta, alpha, diff_point: 0.0, ci_lower: -1.0, ci_upper: 1.0, p_value: 1.0,
            verdict: Verdict::Indetermine, n: 0, discordant: 0, boot_iters, boot_seed: seed,
        };
    }
    for _ in 0..boot_iters.max(1) {
        let mut acc = 0.0;
        for _ in 0..n {
            acc += diffs[rng.below(n)];
        }
        boots.push(acc / n as f64);
    }
    boots.sort_by(|a, b| a.partial_cmp(b).unwrap());

    // Bornes one-sided au niveau α (quantiles α et 1−α de la distribution bootstrap).
    let ci_lower = quantile_sorted(&boots, alpha);
    let ci_upper = quantile_sorted(&boots, 1.0 - alpha);
    // p-valeur de non-infériorité : fraction des ré-échantillons encore ≥ δ (H0 : Δ ≥ δ).
    let p_value = boots.iter().filter(|&&x| x >= delta).count() as f64 / boots.len() as f64;

    let verdict = if ci_upper < 0.0 {
        Verdict::Superieur
    } else if ci_upper < delta {
        Verdict::NonInferieur
    } else if ci_lower > delta {
        Verdict::Inferieur
    } else {
        Verdict::Indetermine
    };

    NonInferiority { delta, alpha, diff_point, ci_lower, ci_upper, p_value, verdict, n, discordant: disc, boot_iters, boot_seed: seed }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Intervalle d'exactitude (Wilson) · puissance · taille requise
// ─────────────────────────────────────────────────────────────────────────────

/// Intervalle de confiance **de Wilson** (score) pour une proportion (exactitude d'un rapport).
/// Plus honnête que Wald aux bords. `correct` peut être fractionnaire (graines moyennées).
pub fn wilson_ci(correct: f64, n: usize, alpha: f64) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let z = probit(1.0 - alpha / 2.0); // two-sided
    let nn = n as f64;
    let p = (correct / nn).clamp(0.0, 1.0);
    let denom = 1.0 + z * z / nn;
    let center = (p + z * z / (2.0 * nn)) / denom;
    let margin = (z / denom) * (p * (1.0 - p) / nn + z * z / (4.0 * nn * nn)).sqrt();
    ((center - margin).max(0.0), (center + margin).min(1.0))
}

/// Taille d'échantillon requise pour conclure la non-infériorité appariée à marge δ, niveau α,
/// puissance 1−β, sous une discordance attendue (`p_b`, `p_c` = proportions discordantes attendues).
/// `n ≈ (z_α+z_β)²·[ψ − (p_b−p_c)²]/(δ − Δ₀)²`, ψ=p_b+p_c, Δ₀=p_b−p_c (écart vrai supposé).
/// `usize::MAX` si l'écart vrai supposé atteint ou dépasse δ (non puissançable).
pub fn required_n(delta: f64, alpha: f64, beta: f64, p_b: f64, p_c: f64) -> usize {
    let psi = p_b + p_c;
    let delta0 = p_b - p_c;
    let gap = delta - delta0;
    if gap <= 0.0 {
        return usize::MAX;
    }
    let z = probit(1.0 - alpha) + probit(1.0 - beta); // une-face
    let n = z * z * (psi - delta0 * delta0) / (gap * gap);
    n.max(0.0).ceil() as usize
}

/// Puissance atteinte (probabilité de conclure non-inférieur quand c'est vrai) au `n` observé, sous
/// la discordance observée (`p_b`, `p_c`). Φ((δ−Δ₀)·√(n/(ψ−Δ₀²)) − z_{1−α}).
pub fn achieved_power(n: usize, delta: f64, alpha: f64, p_b: f64, p_c: f64) -> f64 {
    if n == 0 {
        return 0.0;
    }
    let psi = p_b + p_c;
    let delta0 = p_b - p_c;
    let var = psi - delta0 * delta0;
    if var <= 0.0 {
        // Aucune discordance : tout concordant → non-infériorité triviale (si Δ₀ < δ).
        return if delta0 < delta { 1.0 } else { 0.0 };
    }
    let z_alpha = probit(1.0 - alpha);
    let arg = (delta - delta0) * (n as f64 / var).sqrt() - z_alpha;
    normal_cdf(arg)
}

// ─────────────────────────────────────────────────────────────────────────────
//  Carte des écarts {modèle × tâche} + multiplicité (Holm-Bonferroni)
// ─────────────────────────────────────────────────────────────────────────────

/// L'analyse d'une cellule (un petit modèle sur une classe de tâches), à une marge δ.
#[derive(Debug, Clone, Serialize)]
pub struct GapCell {
    pub small_model: String,
    pub task: String,
    /// Exactitude du petit (moyenne des justesses par-item) + IC bootstrap cluster (niveau item).
    pub small_acc: f64,
    pub small_ci: (f64, f64),
    /// Exactitude du gros + IC bootstrap cluster (niveau item).
    pub big_acc: f64,
    pub big_ci: (f64, f64),
    /// Le test de non-infériorité (justesse brute).
    pub nin: NonInferiority,
    /// Idem, conditionné à la présence des deux (capacité, I-HA-PRESENT).
    pub nin_present: NonInferiority,
    /// Puissance atteinte au n observé (I-HA-POWER).
    pub power: f64,
    /// n requis pour conclure à cette δ sous la discordance observée.
    pub n_required: usize,
    /// Substitue au niveau brut (verdict.substitutes()), AVANT correction de multiplicité.
    pub raw_passes: bool,
    /// Substitue APRÈS Holm-Bonferroni sur la famille (I-HA-MULTIPLE). Posé par [`GapMap::build`].
    pub holm_passes: bool,
}

/// Construit une cellule depuis les rapports (graines) du petit et du gros sur une tâche.
#[allow(clippy::too_many_arguments)]
pub fn analyze_cell(
    small_model: impl Into<String>,
    task: impl Into<String>,
    small: &[ConfigReport],
    big: &[ConfigReport],
    delta: f64,
    alpha: f64,
    beta: f64,
    boot_iters: usize,
    seed: u64,
) -> Result<GapCell, SubstitutionError> {
    let pairs = pair_by_case(small, big)?;
    let present = condition_on_present(&pairs);

    let n_items = pairs.len();
    let small_accs: Vec<f64> = pairs.iter().map(|p| p.small_acc()).collect();
    let big_accs: Vec<f64> = pairs.iter().map(|p| p.big_acc()).collect();

    // IC d'exactitude par côté = bootstrap CLUSTER au niveau item (PAS un binomial par-run, qui
    // sous-estimerait le n du côté le plus répété et ignorerait la corrélation intra-item). Chaque
    // côté est ré-échantillonné sur SES propres justesses par-item → correct quel que soit le nombre
    // de graines de chaque modèle, et déterministe (cohérent avec `non_inferiority`).
    let small_acc = mean(&small_accs);
    let big_acc = mean(&big_accs);
    let small_ci = cluster_acc_ci(&small_accs, alpha, boot_iters, seed ^ 0x0000_ACC1);
    let big_ci = cluster_acc_ci(&big_accs, alpha, boot_iters, seed ^ 0x0000_ACC2);

    let nin = non_inferiority(&pairs, delta, alpha, boot_iters, seed);
    let nin_present = non_inferiority(&present, delta, alpha, boot_iters, seed ^ 0x5051_5253);

    // Discordance observée → p_b, p_c (proportions au niveau item) pour puissance / n requis.
    let d = discordance(&pairs);
    let denom = n_items.max(1) as f64;
    let (p_b, p_c) = (d.big_better as f64 / denom, d.small_better as f64 / denom);
    let power = achieved_power(n_items, delta, alpha, p_b, p_c);
    let n_required = required_n(delta, alpha, beta, p_b, p_c);

    Ok(GapCell {
        small_model: small_model.into(),
        task: task.into(),
        small_acc,
        small_ci,
        big_acc,
        big_ci,
        raw_passes: nin.verdict.substitutes(),
        nin,
        nin_present,
        power,
        n_required,
        holm_passes: false, // posé par GapMap::build
    })
}

/// La carte des écarts : la grille {modèle × tâche}, avec multiplicité corrigée.
#[derive(Debug, Clone, Serialize)]
pub struct GapMap {
    pub cells: Vec<GapCell>,
    pub alpha: f64,
}

impl GapMap {
    /// Assemble la carte et applique **Holm-Bonferroni** (step-down) à la famille des tests de
    /// non-infériorité (I-HA-MULTIPLE) : empêche de déclarer une fausse substitution par hasard sur
    /// l'une des nombreuses cellules. `holm_passes` n'est vrai que si la cellule substitue au brut
    /// ET franchit le seuil de Holm.
    pub fn build(mut cells: Vec<GapCell>, alpha: f64) -> Self {
        let m = cells.len();
        if m > 0 {
            // p-valeurs (de non-infériorité) triées croissantes, avec index d'origine.
            let mut order: Vec<usize> = (0..m).collect();
            order.sort_by(|&i, &j| cells[i].nin.p_value.partial_cmp(&cells[j].nin.p_value).unwrap());
            // Step-down : rejeter tant que p_(i) ≤ α/(m−i) ; au premier échec, stop (rien après).
            let mut still_rejecting = true;
            for (rank, &idx) in order.iter().enumerate() {
                let threshold = alpha / (m - rank) as f64;
                let rejected = still_rejecting && cells[idx].nin.p_value <= threshold;
                if !rejected {
                    still_rejecting = false;
                }
                // holm_passes exige AUSSI que la cellule substitue au niveau brut (cohérence verdict).
                cells[idx].holm_passes = rejected && cells[idx].raw_passes;
            }
        }
        GapMap { cells, alpha }
    }

    /// La thèse (substitution) tient-elle sur AU MOINS une cellule, après correction de multiplicité ?
    pub fn any_substitution(&self) -> bool {
        self.cells.iter().any(|c| c.holm_passes)
    }

    /// Rendu texte lisible (une ligne par cellule).
    pub fn table(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "{:<14} {:<10} {:>8} {:>8} {:>8} {:>12} {:>7} {:>8}\n",
            "modèle", "tâche", "petit%", "gros%", "Δ", "verdict", "puiss", "Holm"
        ));
        out.push_str(&"-".repeat(82));
        out.push('\n');
        for c in &self.cells {
            out.push_str(&format!(
                "{:<14} {:<10} {:>7.1}% {:>7.1}% {:>+8.1} {:>12} {:>6.0}% {:>8}\n",
                truncate(&c.small_model, 14),
                truncate(&c.task, 10),
                c.small_acc * 100.0,
                c.big_acc * 100.0,
                c.nin.diff_point * 100.0,
                c.nin.verdict.label(),
                c.power * 100.0,
                if c.holm_passes { "SUBST." } else { "—" },
            ));
        }
        out
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n.saturating_sub(1)).collect::<String>() + "…"
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Primitives statistiques (déterministes, sans dépendance)
// ─────────────────────────────────────────────────────────────────────────────

fn mean(v: &[f64]) -> f64 {
    if v.is_empty() {
        0.0
    } else {
        v.iter().sum::<f64>() / v.len() as f64
    }
}

/// IC d'exactitude d'un côté par **bootstrap cluster** au niveau item (two-sided 1−α, percentile),
/// déterministe. Ré-échantillonne les items (chacun porte sa justesse moyennée sur ses graines) →
/// respecte la corrélation intra-item et tout nombre de graines. Vide ⇒ `(0,1)` (non informatif).
fn cluster_acc_ci(per_item: &[f64], alpha: f64, iters: usize, seed: u64) -> (f64, f64) {
    if per_item.is_empty() {
        return (0.0, 1.0);
    }
    let n = per_item.len();
    let mut rng = Rng::new(seed);
    let mut boots = Vec::with_capacity(iters.max(1));
    for _ in 0..iters.max(1) {
        let mut acc = 0.0;
        for _ in 0..n {
            acc += per_item[rng.below(n)];
        }
        boots.push(acc / n as f64);
    }
    boots.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (quantile_sorted(&boots, alpha / 2.0), quantile_sorted(&boots, 1.0 - alpha / 2.0))
}

/// Quantile d'un tableau **déjà trié** (interpolation au plus proche rang).
fn quantile_sorted(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let q = q.clamp(0.0, 1.0);
    let idx = (q * (sorted.len() - 1) as f64).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// PRNG **déterministe** SplitMix64 (aucune entropie système → re-prouvabilité du bootstrap).
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        // Évite l'état 0 dégénéré.
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Entier uniforme dans [0, n) (biais modulo négligeable pour nos tailles).
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

/// Fonction d'erreur (Abramowitz & Stegun 7.1.26, |erreur| ≤ 1.5e-7).
fn erf(x: f64) -> f64 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let y = 1.0
        - (((((1.061_405_429 * t - 1.453_152_027) * t) + 1.421_413_741) * t - 0.284_496_736) * t + 0.254_829_592)
            * t
            * (-x * x).exp();
    sign * y
}

/// CDF normale standard Φ(x).
pub fn normal_cdf(x: f64) -> f64 {
    0.5 * (1.0 + erf(x / std::f64::consts::SQRT_2))
}

/// Quantile normal standard (probit / inverse CDF), algorithme d'Acklam (|erreur| ≈ 1e-9).
pub fn probit(p: f64) -> f64 {
    if p <= 0.0 {
        return f64::NEG_INFINITY;
    }
    if p >= 1.0 {
        return f64::INFINITY;
    }
    const A: [f64; 6] = [
        -3.969_683_028_665_376e1, 2.209_460_984_245_205e2, -2.759_285_104_469_687e2,
        1.383_577_518_672_69e2, -3.066_479_806_614_716e1, 2.506_628_277_459_239e0,
    ];
    const B: [f64; 5] = [
        -5.447_609_879_822_406e1, 1.615_858_368_580_409e2, -1.556_989_798_598_866e2,
        6.680_131_188_771_972e1, -1.328_068_155_288_572e1,
    ];
    const C: [f64; 6] = [
        -7.784_894_002_430_293e-3, -3.223_964_580_411_365e-1, -2.400_758_277_161_838e0,
        -2.549_732_539_343_734e0, 4.374_664_141_464_968e0, 2.938_163_982_698_783e0,
    ];
    const D: [f64; 4] = [
        7.784_695_709_041_462e-3, 3.224_671_290_700_398e-1, 2.445_134_137_142_996e0, 3.754_408_661_907_416e0,
    ];
    const PLOW: f64 = 0.024_25;
    const PHIGH: f64 = 1.0 - PLOW;
    if p < PLOW {
        let q = (-2.0 * p.ln()).sqrt();
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= PHIGH {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::report::ConfigReport;
    use crate::runner::RunRecord;
    use serde_json::json;

    // ── primitives statistiques ──

    #[test]
    fn probit_matches_known_quantiles() {
        assert!((probit(0.5)).abs() < 1e-6);
        assert!((probit(0.975) - 1.959_964).abs() < 1e-4, "got {}", probit(0.975));
        assert!((probit(0.95) - 1.644_854).abs() < 1e-4);
        assert!((probit(0.99) - 2.326_348).abs() < 1e-4);
        assert!((probit(0.025) + 1.959_964).abs() < 1e-4, "symétrie");
    }

    #[test]
    fn normal_cdf_matches_known() {
        assert!((normal_cdf(0.0) - 0.5).abs() < 1e-6);
        assert!((normal_cdf(1.959_964) - 0.975).abs() < 1e-3);
        assert!((normal_cdf(-1.959_964) - 0.025).abs() < 1e-3);
    }

    #[test]
    fn wilson_contains_p_and_handles_edges() {
        let (lo, hi) = wilson_ci(50.0, 100, 0.05);
        assert!(lo < 0.5 && hi > 0.5, "encadre p=0.5 : ({lo},{hi})");
        assert!((lo - 0.404).abs() < 0.01 && (hi - 0.596).abs() < 0.01, "Wilson 50/100 ≈ (0.404,0.596)");
        assert_eq!(wilson_ci(0.0, 0, 0.05), (0.0, 1.0), "n=0 → (0,1)");
        let (lo, _hi) = wilson_ci(100.0, 100, 0.05);
        assert!(lo > 0.95 && lo < 1.0, "p=1 borné, pas dégénéré : {lo}");
    }

    // ── helpers de construction de paires ──

    fn item(case: &str, small: bool, big: bool) -> PairedItem {
        PairedItem {
            case_id: case.into(),
            small_correct: vec![small],
            big_correct: vec![big],
            small_present: vec![true],
            big_present: vec![true],
        }
    }
    /// n items dont `b` où le gros gagne (small faux/big vrai), `c` où le petit gagne, reste concordant-correct.
    fn synth(n: usize, b: usize, c: usize) -> Vec<PairedItem> {
        let mut v = Vec::new();
        for i in 0..n {
            let (s, g) = if i < b {
                (false, true) // gros gagne
            } else if i < b + c {
                (true, false) // petit gagne
            } else {
                (true, true) // concordant correct
            };
            v.push(item(&format!("c{i:04}"), s, g));
        }
        v
    }

    // ── non-infériorité : les quatre verdicts ──

    #[test]
    fn perfect_substitute_is_non_inferior() {
        // Tous concordants corrects → Δ≡0 → borne sup 0 < δ → non-inférieur (mais pas supérieur).
        let pairs = synth(100, 0, 0);
        let r = non_inferiority(&pairs, 0.05, 0.05, 2000, 42);
        assert_eq!(r.verdict, Verdict::NonInferieur, "{r:?}");
        assert!(r.verdict.substitutes());
        assert!((r.diff_point).abs() < 1e-9);
    }

    #[test]
    fn clearly_worse_is_inferior() {
        // 30% des items : gros gagne → Δ≈0.30, δ=0.05 → borne inf > δ → inférieur.
        let pairs = synth(100, 30, 0);
        let r = non_inferiority(&pairs, 0.05, 0.05, 2000, 42);
        assert_eq!(r.verdict, Verdict::Inferieur, "{r:?}");
        assert!(!r.verdict.substitutes());
        assert!(r.ci_lower > 0.05, "borne inf {} > δ", r.ci_lower);
    }

    #[test]
    fn clearly_better_is_superior() {
        // 20% des items : petit gagne → Δ≈−0.20 → borne sup < 0 → supérieur.
        let pairs = synth(100, 0, 20);
        let r = non_inferiority(&pairs, 0.05, 0.05, 2000, 42);
        assert_eq!(r.verdict, Verdict::Superieur, "{r:?}");
        assert!(r.verdict.substitutes());
        assert!(r.ci_upper < 0.0, "borne sup {} < 0", r.ci_upper);
    }

    #[test]
    fn underpowered_is_indeterminate_not_equal() {
        // n=10, 1 item où le gros gagne → Δ=0.1, δ=0.05 : l'IC chevauche δ → INDÉTERMINÉ.
        // C'est l'anti-erreur cardinale : ne pas confondre « pas assez de données » avec « égal ».
        let pairs = synth(10, 1, 0);
        let r = non_inferiority(&pairs, 0.05, 0.05, 2000, 42);
        assert_eq!(r.verdict, Verdict::Indetermine, "sous-puissance → indéterminé, pas 'égal' : {r:?}");
    }

    #[test]
    fn bootstrap_is_deterministic() {
        let pairs = synth(80, 5, 3);
        let a = non_inferiority(&pairs, 0.05, 0.05, 1500, 7);
        let b = non_inferiority(&pairs, 0.05, 0.05, 1500, 7);
        assert_eq!(a.ci_upper, b.ci_upper, "même graine → même borne (I-HA-REPROVE)");
        assert_eq!(a.p_value, b.p_value);
        let c = non_inferiority(&pairs, 0.05, 0.05, 1500, 8);
        // Graine différente : très probablement une borne différente (pas garanti, mais attendu ici).
        assert!(a.ci_upper != c.ci_upper || a.ci_lower != c.ci_lower, "graine ≠ → ré-échantillonnage ≠");
    }

    #[test]
    fn empty_pairs_are_indeterminate_not_panic() {
        let r = non_inferiority(&[], 0.05, 0.05, 100, 1);
        assert_eq!(r.verdict, Verdict::Indetermine);
        assert_eq!(r.n, 0);
    }

    #[test]
    fn delta_sweep_strengthens_from_lenient_to_strict() {
        // Δ vrai ≈ 0.03 : substitue à δ=0.05 (lâche), mais indéterminé/inférieur à δ=0.01 (strict).
        let pairs = synth(300, 9, 0); // Δ ≈ 0.03
        let lax = non_inferiority(&pairs, 0.05, 0.05, 2000, 11);
        let strict = non_inferiority(&pairs, 0.01, 0.05, 2000, 11);
        assert!(lax.verdict.substitutes(), "à δ=5pts, substitue : {:?}", lax.verdict);
        assert!(!strict.verdict.substitutes(), "à δ=1pt, ne substitue plus : {:?}", strict.verdict);
    }

    // ── McNemar & discordance ──

    #[test]
    fn mcnemar_counts_and_discordance() {
        let pairs = synth(100, 30, 5); // b=30, c=5, a=65, d=0
        let m = mcnemar(&pairs).expect("mono-graine");
        assert_eq!((m.a, m.b, m.c, m.d), (65, 30, 5, 0));
        let d = discordance(&pairs);
        assert_eq!((d.big_better, d.small_better, d.tie), (30, 5, 65));
        assert_eq!(d.discordant(), 35);
    }

    #[test]
    fn mcnemar_none_when_multi_seed() {
        let p = PairedItem {
            case_id: "x".into(),
            small_correct: vec![true, false],
            big_correct: vec![true, true],
            small_present: vec![true, true],
            big_present: vec![true, true],
        };
        assert!(mcnemar(&[p]).is_none(), "graine multiple → pas de McNemar exact");
    }

    // ── puissance & taille requise ──

    #[test]
    fn required_n_grows_as_margin_shrinks() {
        let n5 = required_n(0.05, 0.05, 0.20, 0.10, 0.10); // δ=5pts
        let n1 = required_n(0.01, 0.05, 0.20, 0.10, 0.10); // δ=1pt
        assert!(n1 > n5, "δ plus strict → n bien plus grand : n1={n1} n5={n5}");
        assert!(n5 > 0 && n5 < 100_000);
    }

    #[test]
    fn required_n_infinite_when_true_diff_exceeds_margin() {
        // Écart vrai supposé p_b−p_c = 0.10 ≥ δ=0.05 → non puissançable.
        assert_eq!(required_n(0.05, 0.05, 0.20, 0.15, 0.05), usize::MAX);
    }

    #[test]
    fn achieved_power_increases_with_n() {
        let p_lo = achieved_power(100, 0.05, 0.05, 0.10, 0.10);
        let p_hi = achieved_power(2000, 0.05, 0.05, 0.10, 0.10);
        assert!(p_hi > p_lo, "plus d'items → plus de puissance : {p_lo} → {p_hi}");
        assert!((0.0..=1.0).contains(&p_hi));
    }

    // ── Holm-Bonferroni ──

    #[test]
    fn holm_is_stricter_than_raw() {
        // Quatre cellules « limites » : au brut elles passent, mais Holm en demote.
        let mk = |name: &str, p: f64, passes: bool| {
            let nin = NonInferiority {
                delta: 0.05, alpha: 0.05, diff_point: 0.0, ci_lower: -0.1, ci_upper: 0.04, p_value: p,
                verdict: if passes { Verdict::NonInferieur } else { Verdict::Indetermine },
                n: 100, discordant: 10, boot_iters: 1, boot_seed: 0,
            };
            GapCell {
                small_model: name.into(), task: "t".into(), small_acc: 0.8, small_ci: (0.7, 0.9),
                big_acc: 0.8, big_ci: (0.7, 0.9), raw_passes: passes,
                nin_present: nin.clone(), nin, power: 0.9, n_required: 100, holm_passes: false,
            }
        };
        // p-valeurs : 0.004, 0.02, 0.03, 0.045 ; m=4. Seuils Holm : 0.0125, 0.0167, 0.025, 0.05.
        let cells = vec![mk("a", 0.004, true), mk("b", 0.02, true), mk("c", 0.03, true), mk("d", 0.045, true)];
        let map = GapMap::build(cells, 0.05);
        let pass: Vec<bool> = map.cells.iter().map(|c| c.holm_passes).collect();
        // a : 0.004 ≤ 0.0125 ✓ ; b : 0.02 ≤ 0.0167 ✗ → stop → b,c,d rejetés.
        assert_eq!(pass, vec![true, false, false, false], "Holm step-down : seul 'a' survit");
        assert!(map.any_substitution());
    }

    // ── appariement depuis des ConfigReport ──

    fn rec(case: &str, correct: bool, present: bool) -> RunRecord {
        RunRecord {
            case_id: case.into(), correct, present, agents_used: 1, n17_calls: 1, tokens: 10,
            tokens_estimated: true, elapsed_ms: 0, spent_cents: 0, verified: true,
            provenance_root: Some("aa".into()), n17_root: Some("bb".into()),
            attestation: Some("R2-LOCAL-TEMOIN".into()), winner: json!({"value":"x"}),
        }
    }
    fn report(cfg: &str, items: &[(&str, bool, bool)]) -> ConfigReport {
        ConfigReport::aggregate(cfg.into(), items.iter().map(|&(c, ok, pr)| rec(c, ok, pr)).collect())
    }

    #[test]
    fn pair_by_case_aligns_and_averages_seeds() {
        // Petit : 2 graines ; gros : 1 graine. Items {a,b}.
        let small = vec![
            report("s", &[("a", true, true), ("b", false, true)]),
            report("s", &[("a", false, true), ("b", false, true)]),
        ];
        let big = vec![report("g", &[("a", true, true), ("b", true, true)])];
        let pairs = pair_by_case(&small, &big).expect("apparié");
        assert_eq!(pairs.len(), 2);
        let a = pairs.iter().find(|p| p.case_id == "a").unwrap();
        assert!((a.small_acc() - 0.5).abs() < 1e-9, "petit sur a : (1+0)/2 = 0.5");
        assert!((a.big_acc() - 1.0).abs() < 1e-9);
        assert!((a.diff() - 0.5).abs() < 1e-9, "Δ_a = 1.0 − 0.5");
    }

    #[test]
    fn pair_by_case_rejects_mismatched_items() {
        let small = vec![report("s", &[("a", true, true)])];
        let big = vec![report("g", &[("b", true, true)])]; // item différent
        assert!(matches!(pair_by_case(&small, &big), Err(SubstitutionError::CaseMismatch { .. })));
    }

    #[test]
    fn pair_by_case_rejects_seed_disagreement() {
        let small = vec![
            report("s", &[("a", true, true), ("b", true, true)]),
            report("s", &[("a", true, true)]), // graine 2 a moins d'items
        ];
        let big = vec![report("g", &[("a", true, true), ("b", true, true)])];
        assert!(matches!(pair_by_case(&small, &big), Err(SubstitutionError::SeedMismatch { .. })));
    }

    #[test]
    fn condition_on_present_drops_format_failures() {
        // Item 'b' : le petit n'a pas répondu (absent) → retiré de la comparaison de CAPACITÉ.
        let pairs = vec![
            PairedItem { case_id: "a".into(), small_correct: vec![true], big_correct: vec![true], small_present: vec![true], big_present: vec![true] },
            PairedItem { case_id: "b".into(), small_correct: vec![false], big_correct: vec![true], small_present: vec![false], big_present: vec![true] },
        ];
        let cap = condition_on_present(&pairs);
        assert_eq!(cap.len(), 1, "seul 'a' (les deux présents) survit");
        assert_eq!(cap[0].case_id, "a");
    }

    // ── la cellule complète & la carte ──

    #[test]
    fn cluster_acc_ci_collapses_on_constant_and_contains_mean() {
        // Tout pareil → l'IC s'effondre sur la valeur ; varié → encadre la moyenne ; vide → (0,1).
        assert_eq!(cluster_acc_ci(&[1.0, 1.0, 1.0], 0.05, 500, 1), (1.0, 1.0));
        let (lo, hi) = cluster_acc_ci(&[1.0, 0.0, 1.0, 0.0, 1.0, 0.0], 0.05, 2000, 3);
        assert!(lo <= 0.5 && 0.5 <= hi && lo >= 0.0 && hi <= 1.0, "encadre 0.5 dans [0,1] : ({lo},{hi})");
        assert_eq!(cluster_acc_ci(&[], 0.05, 10, 1), (0.0, 1.0));
    }

    #[test]
    fn accuracy_ci_uses_cluster_bootstrap_not_min_seed_count() {
        // RÉGRESSION (revue adversariale) : nb de graines DIFFÉRENT (petit=1, gros=3). L'ancien code
        // calculait l'IC des deux à n = n_items × min(graines) → largeur fausse pour le côté répété.
        // Le bootstrap cluster par côté donne des IC valides, déterministes, indépendants de min().
        let items: Vec<(String, bool, bool)> = (0..100).map(|i| (format!("c{i:03}"), i % 2 == 0, true)).collect();
        let refs: Vec<(&str, bool, bool)> = items.iter().map(|(c, a, p)| (c.as_str(), *a, *p)).collect();
        let small = vec![report("s", &refs)];
        let big = vec![report("g", &refs), report("g", &refs), report("g", &refs)]; // 3 graines
        let cell = analyze_cell("s", "T", &small, &big, 0.05, 0.05, 0.2, 1500, 5).expect("cellule");
        // IC valides, encadrant le point, dans [0,1] — des deux côtés malgré les graines asymétriques.
        assert!(cell.small_ci.0 <= cell.small_acc && cell.small_acc <= cell.small_ci.1, "petit : {:?}", cell.small_ci);
        assert!(cell.big_ci.0 <= cell.big_acc && cell.big_acc <= cell.big_ci.1, "gros : {:?}", cell.big_ci);
        assert!(cell.small_ci.0 >= 0.0 && cell.small_ci.1 <= 1.0 && cell.big_ci.0 >= 0.0 && cell.big_ci.1 <= 1.0);
        // Déterminisme (I-HA-REPROVE) : même graine → mêmes IC.
        let cell2 = analyze_cell("s", "T", &small, &big, 0.05, 0.05, 0.2, 1500, 5).expect("cellule");
        assert_eq!((cell.small_ci, cell.big_ci), (cell2.small_ci, cell2.big_ci));
    }

    #[test]
    fn analyze_cell_and_gap_map_end_to_end() {
        // Petit substitue (mêmes réponses que le gros), n suffisant.
        let items: Vec<(String, bool, bool)> = (0..120).map(|i| (format!("c{i:03}"), true, true)).collect();
        let refs: Vec<(&str, bool, bool)> = items.iter().map(|(c, a, p)| (c.as_str(), *a, *p)).collect();
        let small = vec![report("small-3b", &refs)];
        let big = vec![report("big-12b", &refs)];
        let cell = analyze_cell("small-3b", "T-MATH", &small, &big, 0.05, 0.05, 0.2, 1000, 99).expect("cellule");
        assert!(cell.raw_passes, "petit = gros → substitue");
        assert_eq!(cell.nin.verdict, Verdict::NonInferieur);
        assert!((cell.small_acc - 1.0).abs() < 1e-9 && (cell.big_acc - 1.0).abs() < 1e-9);
        let map = GapMap::build(vec![cell], 0.05);
        assert!(map.any_substitution(), "la thèse (substitution) tient sur cette cellule");
        assert!(map.table().contains("T-MATH"));
    }
}
