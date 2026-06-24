// SPDX-License-Identifier: UNLICENSED
//! **HB.1 — l'agrégation : la quantité vérifiée paie-t-elle ?** (couche d'analyse pure sur B1.)
//!
//! Étant donné, par item, **K réponses extraites** (des SEULS agents générateurs — I-HB-GENONLY ;
//! le driver les récupère par rôle, jamais de `llm_records` brut qui mêle des juges-LLM en Mode C),
//! ce module calcule, en fonction de K, la justesse de plusieurs **mécanismes** et la confronte à une
//! référence (le gros modèle), avec la rigueur que la revue adversariale a imposée :
//!
//! - **Mécanismes DÉPLOYABLES** (sans label) : self-consistency (mode), vote **pondéré**, **vérifié**
//!   (mode parmi les passants d'un vérificateur-fonction), et **synthèse** (médiane numérique — qui
//!   peut DÉPASSER pass@K en fabriquant une réponse absente des K).
//! - **Plafond oracle pass@K** (utilise le label) : plafond de la **sélection-parmi-K** seulement
//!   (I-HB-CEILING) ; **monotone par construction** → ne sert qu'à réfuter et à chiffrer l'écart.
//! - **Verdict sans pêche au ∃K** : non-infériorité (réutilise [`crate::substitution`]) à un **K\*
//!   pré-spécifié** (le genou), OU sur le **sup-K via un bootstrap COUPLÉ** (recalcule la courbe par
//!   réplicat → erreur de famille contrôlée). Tie du mode = **abstention = échec** (conservateur).
//! - **Coût** en jetons **entrée/sortie séparés**, sous bornes **cache OFF/ON** ; la domination de
//!   DÉPLOIEMENT (GPU-secondes) reste **suspendue à B2** (I-HB-COST) — jamais conclue ici.
//!
//! Module **pur** : aucune inférence. Déterministe (bootstrap à graine scellée → re-prouvable).

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::{json, Value};

use crate::scorer::ScorerKind;
use crate::substitution::{non_inferiority, NonInferiority, PairedItem};

// ─────────────────────────────────────────────────────────────────────────────
//  Données : un jeu de K échantillons par item
// ─────────────────────────────────────────────────────────────────────────────

/// Les K réponses extraites d'un item, sa vérité-terrain, et la comptabilité de jetons par appel.
#[derive(Debug, Clone)]
pub struct SampleSet {
    pub case_id: String,
    /// L'énoncé (pour un vérificateur-fonction type plug-back ; ignoré par les vérificateurs de sanité).
    pub task: Value,
    /// K réponses EXTRAITES (rôle générateur uniquement). Une réponse absente/illisible est tolérée
    /// (elle ne votera pas) mais compte dans la dégénérescence.
    pub answers: Vec<Value>,
    pub label: Value,
    pub scorer: ScorerKind,
    /// Poids par réponse (confiance/probabilité) pour le vote pondéré ; `None` ⇒ uniforme (= SC).
    pub weights: Option<Vec<f64>>,
    /// Jetons d'entrée par appel (le prompt — répété K× sans cache, partagé avec cache).
    pub in_tokens: Vec<u32>,
    /// Jetons de sortie par appel (le raisonnement + la réponse).
    pub out_tokens: Vec<u32>,
}

impl SampleSet {
    /// Constructeur minimal (réponses + label + scorer) ; poids uniformes, jetons à zéro.
    pub fn new(case_id: impl Into<String>, task: Value, answers: Vec<Value>, label: Value, scorer: ScorerKind) -> Self {
        let n = answers.len();
        Self { case_id: case_id.into(), task, answers, label, scorer, weights: None, in_tokens: vec![0; n], out_tokens: vec![0; n] }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Vérificateurs déployables (sans label)
// ─────────────────────────────────────────────────────────────────────────────

/// Un vérificateur **déployable** : il juge une réponse SANS la vérité-terrain (≠ l'oracle).
pub trait DeployableVerifier: Send + Sync {
    fn accepts(&self, task: &Value, answer: &Value) -> bool;
    fn name(&self) -> &str;
}

/// Sanité de plage : la réponse est un **entier positif** plausible (filtre négatifs, décimaux,
/// magnitudes absurdes). Vérificateur-fonction bon marché, déployable, pour l'arithmétique de comptage.
pub struct PositiveInteger {
    pub max: f64,
}
impl Default for PositiveInteger {
    fn default() -> Self {
        Self { max: 1e7 }
    }
}
impl DeployableVerifier for PositiveInteger {
    fn accepts(&self, _task: &Value, answer: &Value) -> bool {
        parse_num(answer).map(|n| n > 0.0 && n.fract() == 0.0 && n <= self.max).unwrap_or(false)
    }
    fn name(&self) -> &str {
        "positive_integer"
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Les mécanismes (sur les k premières réponses)
// ─────────────────────────────────────────────────────────────────────────────

/// Le mécanisme d'agrégation testé (chacun bien défini PAR ITEM).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mechanism {
    /// Vote majoritaire sur la réponse extraite (self-consistency nue).
    SelfConsistency,
    /// Vote pondéré par les poids (confiance) ; = SC si poids absents.
    Weighted,
    /// Mode parmi les réponses passant un vérificateur-fonction (repli SC si aucune ne passe).
    Verified,
    /// Synthèse : médiane numérique (peut dépasser pass@K).
    Median,
}

/// Le **mode** (réponse la plus fréquente) par clé canonique du scorer, pondéré ou non. `None` =
/// **pas de majorité claire** (égalité) → abstention (I-HB-ANSWER, conservateur).
fn mode(answers: &[Value], scorer: &ScorerKind, weights: Option<&[f64]>) -> Option<Value> {
    let mut tally: BTreeMap<String, (f64, usize)> = BTreeMap::new();
    for (i, a) in answers.iter().enumerate() {
        if let Some(key) = scorer.canonical_key(a) {
            let w = weights.and_then(|ws| ws.get(i)).copied().unwrap_or(1.0);
            let e = tally.entry(key).or_insert((0.0, i)); // garde l'index du PREMIER représentant
            e.0 += w;
        }
    }
    if tally.is_empty() {
        return None;
    }
    let max_w = tally.values().map(|(w, _)| *w).fold(f64::MIN, f64::max);
    let winners: Vec<usize> = tally.values().filter(|(w, _)| (*w - max_w).abs() < 1e-9).map(|(_, i)| *i).collect();
    if winners.len() == 1 {
        Some(answers[winners[0]].clone())
    } else {
        None // égalité → abstention = échec
    }
}

/// Médiane numérique des réponses (synthèse). `None` si aucune n'est numérique.
fn median_numeric(answers: &[Value]) -> Option<Value> {
    let mut nums: Vec<f64> = answers.iter().filter_map(parse_num).collect();
    if nums.is_empty() {
        return None;
    }
    nums.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let m = nums.len();
    let med = if m % 2 == 1 { nums[m / 2] } else { (nums[m / 2 - 1] + nums[m / 2]) / 2.0 };
    Some(json!(med))
}

/// La réponse agrégée d'un mécanisme sur les `k` premières réponses (None = abstention).
fn aggregate(s: &SampleSet, k: usize, mech: Mechanism, ver: Option<&dyn DeployableVerifier>) -> Option<Value> {
    let kk = k.min(s.answers.len());
    let ans = &s.answers[..kk];
    match mech {
        Mechanism::SelfConsistency => mode(ans, &s.scorer, None),
        Mechanism::Weighted => {
            let w = s.weights.as_deref().map(|ws| &ws[..kk.min(ws.len())]);
            mode(ans, &s.scorer, w)
        }
        Mechanism::Verified => {
            let ver = match ver {
                Some(v) => v,
                None => return mode(ans, &s.scorer, None), // pas de vérificateur → SC nu
            };
            let passing: Vec<Value> = ans.iter().filter(|a| ver.accepts(&s.task, a)).cloned().collect();
            if passing.is_empty() {
                mode(ans, &s.scorer, None) // repli SC si aucune ne passe
            } else {
                mode(&passing, &s.scorer, None)
            }
        }
        Mechanism::Median => median_numeric(ans),
    }
}

/// Une réponse est CORRECTE ssi sa **clé canonique** égale celle du label — la MÊME équivalence que
/// le mode (cohérence « même clé ⇒ même verdict » ; sinon une majorité correcte mais mal-formée — un
/// blanc en trop sous scorer Exact — serait notée fausse). Pour Numeric (tolérance 0), Set, Lenient :
/// coïncide avec le scoring de B1 ; pour Exact, tolère la forme — ce que canonical_key encode déjà.
fn answer_correct(answer: &Value, label_key: Option<&str>, scorer: &ScorerKind) -> bool {
    match (scorer.canonical_key(answer), label_key) {
        (Some(k), Some(lk)) => k == lk,
        _ => false,
    }
}

/// Le mécanisme est-il CORRECT sur cet item à k (abstention/illisible ⇒ faux) ?
fn mech_correct(s: &SampleSet, k: usize, mech: Mechanism, ver: Option<&dyn DeployableVerifier>) -> bool {
    let lk = s.scorer.canonical_key(&s.label);
    aggregate(s, k, mech, ver).map(|a| answer_correct(&a, lk.as_deref(), &s.scorer)).unwrap_or(false)
}

/// pass@k : la bonne réponse est-elle PRÉSENTE parmi les k premières (utilise le label — plafond) ?
fn pass_at_k(s: &SampleSet, k: usize) -> bool {
    let lk = s.scorer.canonical_key(&s.label);
    let kk = k.min(s.answers.len());
    s.answers[..kk].iter().any(|a| answer_correct(a, lk.as_deref(), &s.scorer))
}

// ─────────────────────────────────────────────────────────────────────────────
//  La courbe justesse × K
// ─────────────────────────────────────────────────────────────────────────────

/// Un point de la courbe : justesse de chaque mécanisme à K, + télémétrie de diversité et coût.
#[derive(Debug, Clone, Serialize)]
pub struct CurvePoint {
    pub k: usize,
    pub self_consistency: f64,
    pub weighted: f64,
    pub verified: f64,
    /// Justesse de la médiane ; `NaN` si la tâche n'est pas numérique (médiane inapplicable).
    pub median: f64,
    /// **Plafond** (monotone par construction — ne sert qu'à réfuter / chiffrer l'écart).
    pub pass_at_k: f64,
    /// Max des mécanismes DÉPLOYABLES applicables (pour le genou).
    pub best_deployable: f64,
    /// Coût-jetons d'ENTRÉE moyen, borne **cache OFF** (prompt répété k×).
    pub mean_in_cache_off: f64,
    /// Coût-jetons d'ENTRÉE moyen, borne **cache ON** (prompt compté 1×).
    pub mean_in_cache_on: f64,
    /// Coût-jetons de SORTIE moyen (indépendant du cache).
    pub mean_out: f64,
    /// Diversité : fraction moyenne de réponses DISTINCTES (clés) parmi les k.
    pub mean_distinct_frac: f64,
    /// Dégénérescence : fraction moyenne de réponses absentes/illisibles parmi les k.
    pub mean_degenerate_frac: f64,
}

/// Calcule la courbe pour les largeurs `ks`. `verifier` alimente le mécanisme `Verified` (sinon SC nu).
pub fn curve(samples: &[SampleSet], ks: &[usize], verifier: Option<&dyn DeployableVerifier>) -> Vec<CurvePoint> {
    let n = samples.len().max(1) as f64;
    let numeric = samples.iter().all(|s| matches!(s.scorer, ScorerKind::Numeric { .. })) && !samples.is_empty();
    ks.iter()
        .map(|&k| {
            let (mut sc, mut wt, mut vf, mut md, mut ps) = (0usize, 0usize, 0usize, 0usize, 0usize);
            let (mut in_off, mut in_on, mut out, mut distinct, mut degen) = (0.0, 0.0, 0.0, 0.0, 0.0);
            for s in samples {
                // Borné aux réponses RÉELLES (sans `.max(1)` qui ferait paniquer un item à 0 réponse —
                // état légitime : K appels tous illisibles, ou K=0).
                let kk = k.min(s.answers.len());
                if mech_correct(s, k, Mechanism::SelfConsistency, None) {
                    sc += 1;
                }
                if mech_correct(s, k, Mechanism::Weighted, None) {
                    wt += 1;
                }
                if mech_correct(s, k, Mechanism::Verified, verifier) {
                    vf += 1;
                }
                if numeric && mech_correct(s, k, Mechanism::Median, None) {
                    md += 1;
                }
                if pass_at_k(s, k) {
                    ps += 1;
                }
                // Coût (bornes de cache).
                in_off += s.in_tokens[..kk.min(s.in_tokens.len())].iter().map(|&t| t as f64).sum::<f64>();
                in_on += s.in_tokens.first().map(|&t| t as f64).unwrap_or(0.0);
                out += s.out_tokens[..kk.min(s.out_tokens.len())].iter().map(|&t| t as f64).sum::<f64>();
                // Diversité (un item à 0 réponse compte comme 100 % dégénéré).
                if kk == 0 {
                    degen += 1.0;
                } else {
                    let keys: std::collections::BTreeSet<String> =
                        s.answers[..kk].iter().filter_map(|a| s.scorer.canonical_key(a)).collect();
                    distinct += keys.len() as f64 / kk as f64;
                    let deg = s.answers[..kk].iter().filter(|a| s.scorer.canonical_key(a).is_none()).count();
                    degen += deg as f64 / kk as f64;
                }
            }
            let median = if numeric { md as f64 / n } else { f64::NAN };
            let mut best = (sc.max(wt).max(vf)) as f64 / n;
            if numeric {
                best = best.max(median);
            }
            CurvePoint {
                k,
                self_consistency: sc as f64 / n,
                weighted: wt as f64 / n,
                verified: vf as f64 / n,
                median,
                pass_at_k: ps as f64 / n,
                best_deployable: best,
                mean_in_cache_off: in_off / n,
                mean_in_cache_on: in_on / n,
                mean_out: out / n,
                mean_distinct_frac: distinct / n,
                mean_degenerate_frac: degen / n,
            }
        })
        .collect()
}

/// Le **genou** : premier K où le gain marginal du meilleur déployable tombe sous `tau`. À défaut, le
/// dernier K. Règle a priori (pré-spécifiée) qui évite la multiplicité (un seul K\* est testé).
pub fn knee(curve: &[CurvePoint], tau: f64) -> usize {
    for w in curve.windows(2) {
        if w[1].best_deployable - w[0].best_deployable < tau {
            return w[0].k;
        }
    }
    curve.last().map(|p| p.k).unwrap_or(1)
}

// ─────────────────────────────────────────────────────────────────────────────
//  Coût (entrée/sortie séparés, bornes de cache)
// ─────────────────────────────────────────────────────────────────────────────

/// Prix par 1000 jetons, **entrée et sortie séparés** (I-HB-COST). Un jeton de petit modèle ≪ gros.
#[derive(Debug, Clone, Copy)]
pub struct IoPrice {
    pub in_per_1k: f64,
    pub out_per_1k: f64,
}
impl IoPrice {
    pub fn cost(&self, in_tokens: f64, out_tokens: f64) -> f64 {
        in_tokens / 1000.0 * self.in_per_1k + out_tokens / 1000.0 * self.out_per_1k
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Le verdict (sans pêche au ∃K)
// ─────────────────────────────────────────────────────────────────────────────

/// Verdict de H-B à un K\* **pré-spécifié** (aucune multiplicité) — la procédure recommandée.
#[derive(Debug, Clone, Serialize)]
pub struct HbVerdict {
    pub mechanism: Mechanism,
    pub k_star: usize,
    /// Justesse du mécanisme déployable @K\*.
    pub small_acc: f64,
    /// Justesse de la référence (gros, K=1, CoT) — pré-mesurée et scellée.
    pub big_acc: f64,
    /// Non-infériorité appariée du déployable vs le gros (réutilise `substitution`).
    pub nin: NonInferiority,
    /// Coût-jetons domine à la borne **cache OFF** (défavorable à l'essaim) ?
    pub cost_dominates_cache_off: bool,
    /// Coût-jetons domine à la borne **cache ON** (favorable) ?
    pub cost_dominates_cache_on: bool,
    /// Coût-jetons du déployable @K\* (cache OFF) et du gros, pour transparence.
    pub small_cost_off: f64,
    pub big_cost: f64,
    /// Rappel : la domination de DÉPLOIEMENT (GPU-secondes) n'est PAS conclue ici.
    pub deployment: &'static str,
}

/// Apparie deux jeux de samples par `case_id` ; renvoie, pour chaque item commun, (small@k* correct,
/// big@1 correct) — le gros est K=1 (sa « mécanique » est la justesse brute de son unique réponse).
fn pair_correct(
    small: &[SampleSet],
    big: &[SampleSet],
    mech: Mechanism,
    k_star: usize,
    ver: Option<&dyn DeployableVerifier>,
) -> Vec<PairedItem> {
    let big_by: BTreeMap<&str, &SampleSet> = big.iter().map(|s| (s.case_id.as_str(), s)).collect();
    small
        .iter()
        .filter_map(|s| {
            let b = big_by.get(s.case_id.as_str())?;
            let sc = mech_correct(s, k_star, mech, ver);
            // Gros : justesse brute de sa première réponse (K=1, CoT), même équivalence canonique.
            let blk = b.scorer.canonical_key(&b.label);
            let bc = b.answers.first().map(|a| answer_correct(a, blk.as_deref(), &b.scorer)).unwrap_or(false);
            Some(PairedItem {
                case_id: s.case_id.clone(),
                small_correct: vec![sc],
                big_correct: vec![bc],
                small_present: vec![true],
                big_present: vec![true],
            })
        })
        .collect()
}

/// **Le verdict à K\* unique** (pré-spécifié, pas de multiplicité). Non-infériorité du `mechanism`
/// déployable @`k_star` vs la référence du gros, + domination coût-jetons aux deux bornes de cache.
/// La domination de déploiement (GPU-s) reste suspendue à B2.
#[allow(clippy::too_many_arguments)]
pub fn verdict_at_k(
    small: &[SampleSet],
    big: &[SampleSet],
    mechanism: Mechanism,
    k_star: usize,
    verifier: Option<&dyn DeployableVerifier>,
    delta: f64,
    alpha: f64,
    boot_iters: usize,
    seed: u64,
    price_small: IoPrice,
    price_big: IoPrice,
) -> HbVerdict {
    let pairs = pair_correct(small, big, mechanism, k_star, verifier);
    let nin = non_inferiority(&pairs, delta, alpha, boot_iters, seed);
    // Justesse sur les items APPARIÉS (pas tout `small`, qui peut avoir des items absents du gros).
    let np = pairs.len().max(1) as f64;
    let small_acc = pairs.iter().filter(|p| p.small_correct[0]).count() as f64 / np;
    let big_acc = pairs.iter().filter(|p| p.big_correct[0]).count() as f64 / np;

    // Coût moyen par item : déployable @K\* (cache OFF, défavorable) vs gros (K=1).
    let ns = small.len().max(1) as f64;
    let nb = big.len().max(1) as f64;
    let (mut s_in_off, mut s_out, mut b_in, mut b_out) = (0.0, 0.0, 0.0, 0.0);
    for s in small {
        let kk = k_star.min(s.answers.len());
        s_in_off += s.in_tokens[..kk.min(s.in_tokens.len())].iter().map(|&t| t as f64).sum::<f64>();
        s_out += s.out_tokens[..kk.min(s.out_tokens.len())].iter().map(|&t| t as f64).sum::<f64>();
    }
    for b in big {
        b_in += b.in_tokens.first().map(|&t| t as f64).unwrap_or(0.0);
        b_out += b.out_tokens.first().map(|&t| t as f64).unwrap_or(0.0);
    }
    let small_cost_off = price_small.cost(s_in_off / ns, s_out / ns);
    let s_in_on: f64 = small.iter().map(|s| s.in_tokens.first().map(|&t| t as f64).unwrap_or(0.0)).sum::<f64>() / ns;
    let small_cost_on = price_small.cost(s_in_on, s_out / ns);
    let big_cost = price_big.cost(b_in / nb, b_out / nb);

    HbVerdict {
        mechanism,
        k_star,
        small_acc,
        big_acc,
        nin,
        cost_dominates_cache_off: small_cost_off < big_cost,
        cost_dominates_cache_on: small_cost_on < big_cost,
        small_cost_off,
        big_cost,
        deployment: "GPU-secondes à confirmer (B2) — domination coût-jetons seulement",
    }
}

/// Verdict du **sup-K via bootstrap COUPLÉ** (erreur de famille contrôlée) : « ∃K, le déployable est
/// non-inférieur au gros ». Un unique ré-échantillonnage d'items recalcule TOUTE la courbe par
/// réplicat (respecte la covariance inter-K), et l'on borne `sup_K [mech@K − (big − δ)]`.
#[derive(Debug, Clone, Serialize)]
pub struct SupVerdict {
    pub ks: Vec<usize>,
    /// Statistique observée `sup_K [mech@K − (big − δ)]` (positive ⇒ un K franchit dans les données).
    pub sup_stat: f64,
    /// Borne de confiance INFÉRIEure (1−α) de la statistique sup (bootstrap couplé).
    pub sup_lower: f64,
    /// K atteignant le sup observé.
    pub argmax_k: usize,
    /// On peut affirmer (au niveau α) qu'au moins un K est non-inférieur (borne inf &gt; 0).
    pub non_inferior_some_k: bool,
    pub boot_seed: u64,
}

/// Bootstrap couplé sur les items. `mech_correct_matrix[i][j]` = small mécanisme@ks[j] correct sur
/// l'item i ; `big_correct[i]` = gros correct sur l'item i (apparié).
#[allow(clippy::too_many_arguments)]
pub fn verdict_sup_k(
    small: &[SampleSet],
    big: &[SampleSet],
    mechanism: Mechanism,
    ks: &[usize],
    verifier: Option<&dyn DeployableVerifier>,
    delta: f64,
    alpha: f64,
    boot_iters: usize,
    seed: u64,
) -> SupVerdict {
    // Pré-calcule la matrice item × K (small) et le vecteur big, appariés par case_id.
    let big_by: BTreeMap<&str, &SampleSet> = big.iter().map(|s| (s.case_id.as_str(), s)).collect();
    let mut mat: Vec<Vec<bool>> = Vec::new();
    let mut bigv: Vec<bool> = Vec::new();
    for s in small {
        if let Some(b) = big_by.get(s.case_id.as_str()) {
            mat.push(ks.iter().map(|&k| mech_correct(s, k, mechanism, verifier)).collect());
            let blk = b.scorer.canonical_key(&b.label);
            bigv.push(b.answers.first().map(|a| answer_correct(a, blk.as_deref(), &b.scorer)).unwrap_or(false));
        }
    }
    let n = mat.len();

    // Statistiques PAR K sur un ré-échantillon d'items : stat_j = sc@K_j − (big − δ).
    let stats_of = |idx: &[usize]| -> Vec<f64> {
        if idx.is_empty() {
            return vec![f64::MIN; ks.len()];
        }
        let m = idx.len() as f64;
        let big_acc = idx.iter().filter(|&&i| bigv[i]).count() as f64 / m;
        (0..ks.len())
            .map(|j| idx.iter().filter(|&&i| mat[i][j]).count() as f64 / m - (big_acc - delta))
            .collect()
    };

    // Point-estimé observé et son argmax (le K qui franchit le plus).
    let all: Vec<usize> = (0..n).collect();
    let stat_obs = stats_of(&all);
    let (sup_stat, argmax_k) = stat_obs
        .iter()
        .enumerate()
        .fold((f64::MIN, *ks.first().unwrap_or(&1)), |(b, bk), (j, &s)| if s > b { (s, ks[j]) } else { (b, bk) });

    // **Romano-Wolf / max-t** : on borne le SUP en re-centrant CHAQUE composante par son point-estimé
    // AVANT de prendre le max — sinon le max est positivement biaisé et le percentile nu gonfle le
    // taux de faux positifs (revue adversariale : 0,15–0,36 au lieu de α). On collecte le max des
    // déviations centrées par réplicat, et `sup_lower = sup_stat − quantile(maxdev, 1−α)`.
    let mut rng = Rng::new(seed);
    let mut maxdevs: Vec<f64> = Vec::with_capacity(boot_iters.max(1));
    if n > 0 {
        for _ in 0..boot_iters.max(1) {
            let idx: Vec<usize> = (0..n).map(|_| rng.below(n)).collect();
            let st = stats_of(&idx);
            let maxdev = st.iter().zip(&stat_obs).map(|(s, o)| s - o).fold(f64::MIN, f64::max);
            maxdevs.push(maxdev);
        }
        maxdevs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    }
    let sup_lower = if maxdevs.is_empty() { f64::MIN } else { sup_stat - quantile(&maxdevs, 1.0 - alpha) };
    SupVerdict { ks: ks.to_vec(), sup_stat, sup_lower, argmax_k, non_inferior_some_k: sup_lower > 0.0, boot_seed: seed }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Primitives (déterministes, sans dépendance)
// ─────────────────────────────────────────────────────────────────────────────

/// Extrait un f64 d'une réponse (nombre JSON, ou dernier nombre d'une chaîne). **Pèle d'abord
/// l'enveloppe `{value|text}`** — les sorties réelles des générateurs sont `{value: "…CoT… 142"}`,
/// pas des nombres nus (sans ça la médiane lit `None` partout → 0 %, le bug attrapé par la mesure HB.2).
fn parse_num(v: &Value) -> Option<f64> {
    let v = v.get("value").or_else(|| v.get("text")).unwrap_or(v);
    if let Some(f) = v.as_f64() {
        return Some(f);
    }
    let s = v.as_str()?;
    if let Ok(f) = s.trim().parse::<f64>() {
        return Some(f);
    }
    let mut last = None;
    for tok in s.split_whitespace() {
        let d: String = tok.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect();
        let c = d.trim_matches('.');
        if !c.is_empty() {
            if let Ok(f) = c.parse::<f64>() {
                last = Some(f);
            }
        }
    }
    last
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = (q.clamp(0.0, 1.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// PRNG déterministe SplitMix64 (re-prouvabilité du bootstrap).
struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num_sample(case: &str, answers: &[i64], label: i64) -> SampleSet {
        SampleSet::new(case, json!({ "text": "p" }), answers.iter().map(|&a| json!(a)).collect(), json!(label), ScorerKind::Numeric { tolerance: 0.0 })
    }

    // ── mode / self-consistency ──

    #[test]
    fn sc_picks_the_majority_answer() {
        // {142,142,138} → mode 142 = label → correct.
        let s = num_sample("a", &[142, 142, 138], 142);
        assert!(mech_correct(&s, 3, Mechanism::SelfConsistency, None));
        // {138,142,144} (pas de majorité, 1 chacun) → abstention → faux MÊME si 142 présent.
        let s2 = num_sample("b", &[138, 142, 144], 142);
        assert!(!mech_correct(&s2, 3, Mechanism::SelfConsistency, None), "égalité → abstention = échec");
        // mais pass@3 voit 142 présent.
        assert!(pass_at_k(&s2, 3), "pass@k : la bonne réponse est présente");
    }

    #[test]
    fn sc_amplifies_correlated_error() {
        // Modèle biaisé : majorité unanimement FAUSSE → SC amplifie l'erreur (le poison).
        let s = num_sample("a", &[100, 100, 100, 142], 142);
        assert!(!mech_correct(&s, 4, Mechanism::SelfConsistency, None), "mode=100 faux");
        assert!(pass_at_k(&s, 4), "mais 142 est présent → pass@4 vrai");
    }

    #[test]
    fn k1_is_raw_accuracy_equals_pass_at_1() {
        let correct = num_sample("a", &[142], 142);
        assert!(mech_correct(&correct, 1, Mechanism::SelfConsistency, None));
        assert!(pass_at_k(&correct, 1));
        let wrong = num_sample("b", &[7], 142);
        assert!(!mech_correct(&wrong, 1, Mechanism::SelfConsistency, None));
        assert!(!pass_at_k(&wrong, 1), "SC@1 = pass@1 = justesse brute");
    }

    // ── plafond pass@K : monotone, et la SÉLECTION est bornée par lui ──

    #[test]
    fn pass_at_k_is_monotone_and_bounds_selection() {
        let s = num_sample("a", &[7, 8, 142, 9], 142); // correct apparaît à k=3
        assert!(!pass_at_k(&s, 2));
        assert!(pass_at_k(&s, 3));
        assert!(pass_at_k(&s, 4), "monotone : reste vrai");
        // La SÉLECTION (SC) ≤ pass@K : le mode ∈ {réponses tirées}.
        for k in 1..=4 {
            let sc = mech_correct(&s, k, Mechanism::SelfConsistency, None);
            let ps = pass_at_k(&s, k);
            assert!(!sc || ps, "SC correct ⇒ pass@k correct (mode ⊆ tirées) à k={k}");
        }
    }

    #[test]
    fn median_peels_value_wrapper_like_real_generator_output() {
        // Régression (mesure HB.2) : les réponses réelles sont {value: "…CoT… 142"}, pas des nombres
        // nus. parse_num doit peler l'enveloppe — sinon la médiane lit None partout → 0 %.
        let s = SampleSet::new(
            "a",
            json!({}),
            vec![json!({"value": "reasoning... = 141"}), json!({"value": "...so 143"})],
            json!(142),
            ScorerKind::Numeric { tolerance: 0.0 },
        );
        // médiane(141, 143) = 142 = label → correct, même sous enveloppe {value}.
        assert!(mech_correct(&s, 2, Mechanism::Median, None), "médiane sur {{value:…}} = 142 (enveloppe pelée)");
    }

    #[test]
    fn median_can_exceed_pass_at_k() {
        // Aucun échantillon n'est 142, mais la médiane de {140,142,144}... non — prenons {141,142,143}→142
        // avec label 142 : 142 n'est tiré par AUCUN (140,144 + un 143). Médiane({140,143,144})=143≠142.
        // Cas net : {141,143} médiane=142 = label, ABSENT des tirages → dépasse pass@k.
        let s = num_sample("a", &[141, 143], 142);
        assert!(!pass_at_k(&s, 2), "142 absent des tirages → pass@2 faux");
        assert!(mech_correct(&s, 2, Mechanism::Median, None), "médiane(141,143)=142 = label : la SYNTHÈSE dépasse pass@k");
    }

    // ── vérificateur déployable ──

    #[test]
    fn verified_sc_filters_implausible_answers() {
        // Réponses : -5 (négatif, rejeté), 3.5 (décimal, rejeté), 142, 142. Le vérificateur garde {142,142}.
        let s = SampleSet::new("a", json!({}), vec![json!(-5), json!(3.5), json!(142), json!(142)], json!(142), ScorerKind::Numeric { tolerance: 0.0 });
        let ver = PositiveInteger::default();
        assert!(mech_correct(&s, 4, Mechanism::Verified, Some(&ver)), "mode parmi les passants = 142");
        // Sans vérificateur : SC nu sur {−5,3.5,142,142} → 142 majoritaire aussi ici.
        assert!(mech_correct(&s, 4, Mechanism::SelfConsistency, None));
    }

    // ── vote pondéré ──

    #[test]
    fn weighted_vote_overrides_naive_majority() {
        // {142, 100, 100} : SC nu → 100 (majorité). Mais poids confiants sur 142 → vote pondéré = 142.
        let mut s = num_sample("a", &[142, 100, 100], 142);
        s.weights = Some(vec![5.0, 1.0, 1.0]);
        assert!(!mech_correct(&s, 3, Mechanism::SelfConsistency, None), "SC nu → 100 (faux)");
        assert!(mech_correct(&s, 3, Mechanism::Weighted, None), "pondéré → 142 (juste)");
    }

    // ── courbe & genou ──

    #[test]
    fn curve_and_knee() {
        // Deux items ; à k croissant, SC s'améliore puis plafonne.
        let samples = vec![
            num_sample("a", &[142, 142, 142, 142], 142),
            num_sample("b", &[7, 50, 50, 50], 50),
        ];
        let c = curve(&samples, &[1, 2, 4], None);
        assert_eq!(c.len(), 3);
        assert!(c[2].pass_at_k >= c[0].pass_at_k, "pass@k monotone le long de la courbe");
        assert!(c[0].best_deployable <= c[2].best_deployable + 1e-9);
        let k = knee(&c, 0.05);
        assert!([1, 2, 4].contains(&k));
    }

    #[test]
    fn cost_separates_input_output_and_cache_bounds() {
        let mut s = num_sample("a", &[142, 142, 142], 142);
        s.in_tokens = vec![100, 100, 100]; // même prompt répété
        s.out_tokens = vec![20, 22, 18];
        let c = curve(std::slice::from_ref(&s), &[3], None);
        assert!((c[0].mean_in_cache_off - 300.0).abs() < 1e-9, "cache OFF : entrée 3×100");
        assert!((c[0].mean_in_cache_on - 100.0).abs() < 1e-9, "cache ON : entrée 1×100");
        assert!((c[0].mean_out - 60.0).abs() < 1e-9, "sortie = 20+22+18");
    }

    // ── verdict K* (réutilise substitution) ──

    #[test]
    fn verdict_at_k_uses_non_inferiority() {
        // 100 items : le petit (SC@3) corrige tout ; le gros (K=1) aussi tout → non-inférieur.
        let small: Vec<SampleSet> = (0..100).map(|i| num_sample(&format!("c{i}"), &[42, 42, 42], 42)).collect();
        let big: Vec<SampleSet> = (0..100).map(|i| num_sample(&format!("c{i}"), &[42], 42)).collect();
        let price = IoPrice { in_per_1k: 0.1, out_per_1k: 0.1 };
        let v = verdict_at_k(&small, &big, Mechanism::SelfConsistency, 3, None, 0.05, 0.05, 1000, 7, price, IoPrice { in_per_1k: 1.2, out_per_1k: 1.2 });
        assert!((v.small_acc - 1.0).abs() < 1e-9 && (v.big_acc - 1.0).abs() < 1e-9);
        assert!(v.nin.verdict.substitutes(), "SC@3 non-inférieur au gros : {:?}", v.nin.verdict);
        assert_eq!(v.deployment, "GPU-secondes à confirmer (B2) — domination coût-jetons seulement");
    }

    // ── verdict sup-K (bootstrap couplé, anti-multiplicité) ──

    #[test]
    fn exact_consensus_with_whitespace_is_scored_correct() {
        // Régression (revue) : un consensus unanime correct mais avec un espace (scorer Exact) ne doit
        // PAS être noté faux — le mode score par CLÉ canonique, pas par la forme brute du représentant.
        let s = SampleSet::new("a", json!({}), vec![json!(" finance "), json!("finance")], json!("finance"), ScorerKind::Exact);
        assert!(mech_correct(&s, 2, Mechanism::SelfConsistency, None), "majorité 'finance' (espaces ignorés) = correct");
    }

    #[test]
    fn curve_does_not_panic_on_zero_answer_item() {
        // Régression (revue) : un item à 0 réponse (K appels tous illisibles / K=0) est légitime →
        // 100 % dégénéré, et ne doit pas faire paniquer la courbe.
        let empty = SampleSet::new("a", json!({}), vec![], json!(42), ScorerKind::Numeric { tolerance: 0.0 });
        let good = num_sample("b", &[42, 42], 42);
        let c = curve(&[empty, good], &[1, 2], None);
        assert_eq!(c.len(), 2);
        assert!(c[0].mean_degenerate_frac > 0.0, "l'item vide contribue à la dégénérescence");
        assert!((c[1].self_consistency - 0.5).abs() < 1e-9, "1 item correct sur 2 (l'autre vide → faux)");
    }

    #[test]
    fn sup_k_controls_false_positive_at_the_frontier() {
        // Le test qui manquait (revue HIGH) : sous H0 (sc@k = gros − δ pour TOUT K), AVEC variation
        // inter-K (items différents corrects selon k → exerce le biais du max), le recentrage
        // Romano-Wolf / max-t doit CONTRÔLER le taux de faux positifs (le percentile nu le gonflait
        // à 0,15–0,36). Construction : gros toujours juste (acc 1.0) ; petit à sc@1 = sc@3 = 0.5 ;
        // δ = 0.5 ⇒ stat@k ≡ 0 (frontière). On ne doit (presque) jamais déclarer « non-inférieur ».
        let big: Vec<SampleSet> = (0..100).map(|i| num_sample(&format!("c{i:03}"), &[42], 42)).collect();
        let mut small: Vec<SampleSet> = Vec::new();
        for i in 0..30 {
            small.push(num_sample(&format!("c{i:03}"), &[42, 42, 42], 42)); // toujours juste
        }
        for i in 30..60 {
            small.push(num_sample(&format!("c{i:03}"), &[7, 7, 7], 42)); // toujours faux
        }
        for i in 60..80 {
            small.push(num_sample(&format!("c{i:03}"), &[7, 42, 42], 42)); // juste à k≥3, faux à k=1
        }
        for i in 80..100 {
            small.push(num_sample(&format!("c{i:03}"), &[42, 7, 7], 42)); // juste à k=1, faux à k≥3
        }
        let seeds = 60u64;
        let fp = (0..seeds)
            .filter(|&seed| {
                verdict_sup_k(&small, &big, Mechanism::SelfConsistency, &[1, 3], None, 0.5, 0.05, 400, seed).non_inferior_some_k
            })
            .count();
        let rate = fp as f64 / seeds as f64;
        assert!(rate <= 0.20, "taux de faux positifs à la frontière contrôlé (max-t) : {rate} ({fp}/{seeds})");
    }

    #[test]
    fn sup_k_detects_clear_non_inferiority() {
        // Le petit rejoint clairement le gros à K élevé (gain net) → sup-stat positif, borne inf > 0.
        let small: Vec<SampleSet> = (0..120)
            .map(|i| {
                // À k=1 souvent faux (premier=7), mais majorité correcte à k≥3.
                num_sample(&format!("c{i}"), &[7, 42, 42, 42, 42], 42)
            })
            .collect();
        let big: Vec<SampleSet> = (0..120).map(|i| num_sample(&format!("c{i}"), &[42], 42)).collect();
        let v = verdict_sup_k(&small, &big, Mechanism::SelfConsistency, &[1, 3, 5], None, 0.05, 0.05, 1500, 11);
        assert!(v.sup_stat > 0.0, "un K franchit dans les données");
        assert!(v.argmax_k >= 3, "le sup est atteint à K≥3 (où SC=1.0)");
        assert!(v.non_inferior_some_k, "borne inf > 0 → ∃K non-inférieur (FWER contrôlé) : {v:?}");
        // Déterminisme.
        let v2 = verdict_sup_k(&small, &big, Mechanism::SelfConsistency, &[1, 3, 5], None, 0.05, 0.05, 1500, 11);
        assert_eq!(v.sup_lower, v2.sup_lower);
    }

    #[test]
    fn sup_k_stays_negative_when_small_never_catches_up() {
        // Le petit plafonne sous le gros à tout K → sup-stat ≤ 0, pas de franchissement.
        let small: Vec<SampleSet> = (0..120).map(|i| num_sample(&format!("c{i}"), &[7, 7, 7], 142)).collect();
        let big: Vec<SampleSet> = (0..120).map(|i| num_sample(&format!("c{i}"), &[142], 142)).collect();
        let v = verdict_sup_k(&small, &big, Mechanism::SelfConsistency, &[1, 3], None, 0.05, 0.05, 1000, 5);
        assert!(!v.non_inferior_some_k, "petit toujours faux → aucun K non-inférieur");
    }
}
