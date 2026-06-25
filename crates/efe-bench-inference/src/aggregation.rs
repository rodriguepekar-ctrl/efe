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

/// Un **pondérateur de raisonnement** (HB.3) : il attribue à UN candidat un **poids de vote** dans
/// `[w_min, w_max]`, à partir du SEUL texte de l'énoncé et du candidat — **jamais** le label, **jamais**
/// le `SampleSet`. La fuite du label est ainsi *inexprimable au type* (le garde-fou d'honnêteté n°1).
///
/// Pourquoi un pondérateur et pas un [`DeployableVerifier`] (filtre booléen) : un filtre ne peut **pas**
/// déplacer le mode en régime (b) — les réponses majoritaires fausses passent le filtre de sanité comme
/// les bonnes (c'est l'échec plat de `Verified` à 94 %). Seule une **re-pondération** du vote par un
/// signal de qualité par candidat peut faire gagner une minorité bien raisonnée. L'implémentation de
/// référence est [`crate::reexec::ReExecAudit`] (re-exécution de l'arithmétique écrite par le candidat).
pub trait ReasoningWeigher: Send + Sync {
    /// Poids de vote du candidat `answer` pour la tâche `task` (∈ `[w_min, w_max]`). Sans label.
    fn weight(&self, task: &Value, answer: &Value) -> f64;
    /// Borne inférieure de poids (un candidat ne tombe jamais à 0 — il vote, faiblement).
    fn w_min(&self) -> f64;
    /// Borne supérieure de poids.
    fn w_max(&self) -> f64;
    fn name(&self) -> &str;
}

// ─────────────────────────────────────────────────────────────────────────────
//  Les mécanismes (sur les k premières réponses)
// ─────────────────────────────────────────────────────────────────────────────

/// **HB.4 — constante PRÉ-ENREGISTRÉE** : fraction tronquée à CHAQUE queue de la moyenne tronquée. Gelée
/// AVANT toute mesure (la régler sur le jeu scellé serait pass@K fuyant par l'expérimentateur, I-HB4-PREREG).
pub(crate) const TRIM_FRAC: f64 = 0.25;

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
    /// **HB.3** — vote pondéré par l'**audit de re-exécution** (la qualité arithmétique de chaque
    /// candidat), sous **garde de non-infériorité** : ne dévie de la self-consistency que si l'appui
    /// audité de la minorité dépasse un seuil pré-enregistré (sinon rend le mode SC, à l'octet). C'est
    /// une **sélection** (≤ pass@K), pas une synthèse. Cf. [`crate::reexec`].
    ReExecWeighted,
    /// **HB.4** — moyenne **tronquée** des réponses numériques (synthèse robuste : fabrique une valeur
    /// absente des K à K pair). Plafond = l'**enveloppe** des candidats (`within_range`), pas pass@K.
    TrimmedMeanNumeric,
    /// **HB.4 — la fusion HB.3×HB.4** : médiane **pondérée par l'audit de re-exécution**. À traversée
    /// nette → un candidat (sélection) ; à frontière exacte de masse → médiane des deux (synthèse). Sans
    /// pondérateur → `median_numeric` nu. Re-pondère l'ORDRE statistique (continûment), là où RXW
    /// re-pondérait le mode sous un garde à seuil.
    AuditWeightedMedian,
    /// **HB.T (le Terrain)** — vote par **ÉLÉMENT** pour les réponses-ENSEMBLES : inclure un élément ssi
    /// il apparaît dans ≥ la moitié des K candidats. L'opérateur **loyal** pour la structure-ensemble (le
    /// mode-d'ensemble-exact est tout-ou-rien → mauvais outil) ; agrège élément par élément comme le mode
    /// agrège le scalaire. Cf. [`crate::terrain`].
    SetVote,
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

/// Médiane numérique des réponses (synthèse). `None` si aucune n'est numérique. Filtre les non-finis et
/// trie de façon NaN-sûre (défense en profondeur, revue) : aucun `partial_cmp` sur NaN ne fait paniquer.
fn median_numeric(answers: &[Value]) -> Option<Value> {
    let mut nums: Vec<f64> = answers.iter().filter_map(parse_num).filter(|f| f.is_finite()).collect();
    if nums.is_empty() {
        return None;
    }
    nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let m = nums.len();
    let med = if m % 2 == 1 { nums[m / 2] } else { (nums[m / 2 - 1] + nums[m / 2]) / 2.0 };
    Some(json!(med))
}

/// **HB.4** — moyenne **tronquée** : retire `⌊TRIM_FRAC·k⌋` de CHAQUE queue, moyenne le reste. Repli
/// médiane si la troncature viderait tout (k petit). Synthèse : à k≥3 la moyenne tombe en général hors
/// des candidats. Sœur robuste de la médiane (tronquer la mauvaise queue tire vers le vrai sous bimodalité).
fn trimmed_mean_numeric(answers: &[Value]) -> Option<Value> {
    let mut nums: Vec<f64> = answers.iter().filter_map(parse_num).filter(|f| f.is_finite()).collect();
    if nums.is_empty() {
        return None;
    }
    nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let m = nums.len();
    let tc = (TRIM_FRAC * m as f64).floor() as usize;
    if 2 * tc >= m {
        // troncature trop agressive → repli médiane (jamais de tranche vide / panic).
        let med = if m % 2 == 1 { nums[m / 2] } else { (nums[m / 2 - 1] + nums[m / 2]) / 2.0 };
        return Some(json!(med));
    }
    let kept = &nums[tc..m - tc];
    Some(json!(kept.iter().sum::<f64>() / kept.len() as f64))
}

/// Médiane **pondérée** d'une liste (valeur, poids) : trie par valeur, marche la masse cumulée jusqu'à
/// `W/2`. Traversée nette → la valeur qui enjambe (sélection) ; frontière exacte (masse partagée entre
/// deux voisines) → leur moyenne (synthèse). Déterministe (tri stable + bande 1e-9 documentée).
fn weighted_median_value(mut pairs: Vec<(f64, f64)>) -> Option<f64> {
    pairs.retain(|(v, w)| v.is_finite() && w.is_finite() && *w > 0.0);
    if pairs.is_empty() {
        return None;
    }
    pairs.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let total: f64 = pairs.iter().map(|(_, w)| *w).sum();
    let half = total / 2.0;
    let mut cum = 0.0;
    for i in 0..pairs.len() {
        cum += pairs[i].1;
        if (cum - half).abs() <= 1e-9 {
            // frontière exacte → synthèse : moyenne avec la voisine de droite (ou soi-même si dernière).
            let next = pairs.get(i + 1).map(|p| p.0).unwrap_or(pairs[i].0);
            return Some((pairs[i].0 + next) / 2.0);
        }
        if cum > half {
            return Some(pairs[i].0); // ce candidat enjambe la moitié → sélection
        }
    }
    pairs.last().map(|p| p.0)
}

/// **HB.4 — la fusion HB.3×HB.4** : médiane pondérée par l'audit de re-exécution. Sans pondérateur, repli
/// sur la médiane numérique nue (donc déterministe et toujours défini). Sans label par signature (l'audit
/// ne voit que l'énoncé + le candidat).
fn audit_weighted_median(
    task: &Value,
    ans: &[Value],
    weigher: Option<&dyn ReasoningWeigher>,
) -> Option<Value> {
    let weigher = match weigher {
        Some(w) => w,
        None => return median_numeric(ans),
    };
    let prompt_ints = crate::reexec::ints_in_text(task.get("text").and_then(|v| v.as_str()).unwrap_or(""));
    let pairs: Vec<(f64, f64)> = ans
        .iter()
        .filter_map(|a| {
            let v = parse_num(a).filter(|f| f.is_finite())?;
            let sc = crate::reexec::audit_candidate(&prompt_ints, &crate::scorer::answer_text(a));
            Some((v, crate::reexec::weight_from(&sc, weigher.w_min(), weigher.w_max())))
        })
        .collect();
    weighted_median_value(pairs).map(|v| json!(v))
}

/// **HB.T — vote par ÉLÉMENT** (l'opérateur loyal pour les ensembles) : compte la fréquence de chaque
/// élément parmi les `kk` candidats (les illisibles ne votent pour rien), et retient les éléments
/// présents dans **≥ la moitié** des candidats (`2·count ≥ kk`). Renvoie l'ensemble trié (déterministe).
/// `None` si **aucun** candidat n'est lisible comme ensemble (abstention). Partage le noyau `as_set`.
fn set_vote(answers: &[Value]) -> Option<Value> {
    let kk = answers.len();
    if kk == 0 {
        return None;
    }
    let mut tally: BTreeMap<String, usize> = BTreeMap::new();
    let mut parsed = 0usize;
    for a in answers {
        if let Some(set) = crate::scorer::as_set(a) {
            parsed += 1;
            for e in set {
                *tally.entry(e).or_insert(0) += 1;
            }
        }
    }
    if parsed == 0 {
        return None; // aucun ensemble lisible → abstention (échec, comme le mode)
    }
    // Majorité par élément (denominateur = TOUS les k candidats — un illisible compte contre). BTreeMap
    // ordonné → sortie triée → clé canonique stable, re-prouvable.
    let selected: Vec<String> = tally.into_iter().filter(|(_, c)| 2 * c >= kk).map(|(e, _)| e).collect();
    Some(json!(selected))
}

/// **HB.4 — plafond-oracle de la SYNTHÈSE** : le label tombe-t-il dans l'**enveloppe** [min, max] des
/// candidats numériques ? Une médiane/moyenne ne peut atteindre le label que s'il y est. Utilise le label
/// (plafond-oracle, comme pass@K) ; on a toujours `pass@K ≤ within_range`.
fn within_range(s: &SampleSet, k: usize) -> bool {
    let kk = k.min(s.answers.len());
    let nums: Vec<f64> = s.answers[..kk].iter().filter_map(parse_num).filter(|f| f.is_finite()).collect();
    if nums.is_empty() {
        return false;
    }
    let lab = match parse_num(&s.label) {
        Some(l) if l.is_finite() => l,
        _ => return false,
    };
    let lo = nums.iter().cloned().fold(f64::INFINITY, f64::min);
    let hi = nums.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    lab >= lo && lab <= hi
}

/// **HB.4 — l'anti-mensonge** : la valeur émise est-elle FABRIQUÉE (sa clé n'est celle d'AUCUN candidat
/// tiré) ? Une « synthèse » qui rend un candidat est une sélection déguisée et doit être lue comme telle.
fn is_fabricated(emitted: &Value, ans: &[Value], scorer: &ScorerKind) -> bool {
    match scorer.canonical_key(emitted) {
        Some(ek) => !ans.iter().any(|a| scorer.canonical_key(a).as_deref() == Some(ek.as_str())),
        None => false,
    }
}

/// La réponse agrégée d'un mécanisme sur les `k` premières réponses (None = abstention).
fn aggregate(
    s: &SampleSet,
    k: usize,
    mech: Mechanism,
    ver: Option<&dyn DeployableVerifier>,
    weigher: Option<&dyn ReasoningWeigher>,
) -> Option<Value> {
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
        Mechanism::ReExecWeighted => match weigher {
            Some(w) => reexec_select(&s.task, ans, &s.scorer, w.w_min(), w.w_max()),
            None => mode(ans, &s.scorer, None), // pas de pondérateur → SC nu (repli déployable)
        },
        Mechanism::TrimmedMeanNumeric => trimmed_mean_numeric(ans),
        Mechanism::AuditWeightedMedian => audit_weighted_median(&s.task, ans, weigher),
        Mechanism::SetVote => set_vote(ans),
    }
}

/// **Le cœur de HB.3** : le vote pondéré par re-exécution, SOUS GARDE de non-infériorité.
///
/// On audite l'arithmétique écrite de chaque candidat ([`crate::reexec::audit_candidate`]), on en dérive
/// un poids, et on vote via le `mode()` pondéré déjà prouvé. Puis le **garde** : RXW ne **dévie** du choix
/// de la self-consistency QUE si (a) la SC abstient (aucune majorité à protéger), ou (b) l'appui audité
/// (Σ H2) de la clé minoritaire dépasse celui de la clé majoritaire d'au moins `MARGIN`. Sinon — vote
/// pondéré nul (égalité), ou override non décisif — RXW **rend le mode SC**, à l'octet. Ainsi RXW n'est
/// **jamais pire que SC** sur les items à faible signal *par construction* ; il ne prend de risque que là
/// où la minorité est **décisivement** mieux auditée (et ce risque — mauvais montage, bon calcul — est la
/// limite assumée, mesurée, jamais une garantie). Sélection pure ⇒ borné par pass@K.
fn reexec_select(task: &Value, ans: &[Value], scorer: &ScorerKind, w_min: f64, w_max: f64) -> Option<Value> {
    let prompt_ints = crate::reexec::ints_in_text(task.get("text").and_then(|v| v.as_str()).unwrap_or(""));
    let scores: Vec<crate::reexec::AuditScore> =
        ans.iter().map(|a| crate::reexec::audit_candidate(&prompt_ints, &crate::scorer::answer_text(a))).collect();
    let weights: Vec<f64> = scores.iter().map(|sc| crate::reexec::weight_from(sc, w_min, w_max)).collect();
    let rxw = mode(ans, scorer, Some(&weights));
    let sc = mode(ans, scorer, None);
    match rxw {
        None => sc, // égalité pondérée → repli SC (jamais pire que SC)
        Some(r) => match &sc {
            None => Some(r), // SC abstient (pas de majorité) → RXW peut seulement aider
            Some(c) => {
                let (rk, ck) = (scorer.canonical_key(&r), scorer.canonical_key(c));
                if rk == ck {
                    return Some(r); // RXW = SC : aucun override
                }
                // Override de la majorité : exigé que l'appui audité de la minorité domine de MARGIN.
                let sum_h2 = |key: &Option<String>| -> i64 {
                    ans.iter().zip(&scores).filter(|(a, _)| scorer.canonical_key(a) == *key).map(|(_, s)| s.h2).sum()
                };
                if sum_h2(&rk) >= sum_h2(&ck) + crate::reexec::MARGIN {
                    Some(r)
                } else {
                    sc // override non décisif → on rend le mode SC (à l'octet)
                }
            }
        },
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
fn mech_correct(
    s: &SampleSet,
    k: usize,
    mech: Mechanism,
    ver: Option<&dyn DeployableVerifier>,
    weigher: Option<&dyn ReasoningWeigher>,
) -> bool {
    let lk = s.scorer.canonical_key(&s.label);
    aggregate(s, k, mech, ver, weigher).map(|a| answer_correct(&a, lk.as_deref(), &s.scorer)).unwrap_or(false)
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
    /// **HB.3** — justesse du vote pondéré par re-exécution (sous garde). `0` si aucun pondérateur.
    pub re_exec_weighted: f64,
    /// Justesse de la médiane ; `NaN` si la tâche n'est pas numérique (médiane inapplicable).
    pub median: f64,
    /// **HB.4** — justesse de la moyenne tronquée (synthèse). `NaN` si non numérique.
    pub trimmed_mean: f64,
    /// **HB.4 — la fusion** — justesse de la médiane pondérée par l'audit. `NaN` si non numérique.
    pub audit_weighted_median: f64,
    /// **HB.T** — justesse du vote par élément (ensembles). `NaN` si la tâche n'est pas ensembliste.
    pub set_vote: f64,
    /// **Plafond de SÉLECTION** (monotone par construction — ne sert qu'à réfuter / chiffrer l'écart).
    pub pass_at_k: f64,
    /// **HB.4 — plafond de SYNTHÈSE** : fraction d'items où le label est dans l'enveloppe [min,max] des
    /// candidats (la médiane/moyenne ne peut l'atteindre que là). Plafond-oracle ; `pass@K ≤ within_range`.
    pub within_range: f64,
    /// **HB.4 — l'anti-mensonge** : fraction d'items où la médiane FABRIQUE (sortie ∉ candidats). ≈0 ⇒ la
    /// médiane *sélectionne* (lire au plafond pass@K), &gt;0 ⇒ vraie synthèse (lire au plafond within_range).
    pub fabrication_frac: f64,
    /// **HB.4** — dispersion déployable (sans label) : variance numérique moyenne des candidats par item.
    pub dispersion_var: f64,
    /// **HB.4** — dispersion : fraction distincte moyenne (= `mean_distinct_frac`, redite pour le bundle).
    pub dispersion_distinct: f64,
    /// **HB.4** — dispersion : entropie de Shannon normalisée moyenne des poids d'audit (1.0 sans pondérateur).
    pub dispersion_entropy: f64,
    /// **HB.4** — meilleur mécanisme de SYNTHÈSE (médiane/tronquée/pondérée). Plafond = within_range, PAS pass@K.
    pub best_synthesis: f64,
    /// **Couverture** RXH : fraction moyenne de candidats **vérifiables** (≥1 équation auditée) parmi les
    /// k. Plafonne le gain atteignable (un candidat sans équation vote au plancher). `0` si pas de pondérateur.
    pub verifiable_frac: f64,
    /// **Taux d'override** : fraction d'items où le garde a laissé RXW **dévier** de la self-consistency.
    /// Si ≈ 0, RXW ≈ SC — la conclusion honnête est « plat, mais avec télémétrie d'audit ». `0` si pas de pondérateur.
    pub override_rate: f64,
    /// Max des mécanismes de **SÉLECTION** purs (SC/pondéré/vérifié/RXW) — toujours ≤ pass@K, monotone ;
    /// **seul** ce champ alimente le genou (HB.4 : les mécanismes de synthèse sont dans `best_synthesis`).
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

/// Calcule la courbe pour les largeurs `ks`. `verifier` alimente le mécanisme `Verified` (sinon SC nu) ;
/// `weigher` alimente `ReExecWeighted` (HB.3) et la télémétrie de couverture/override (sinon ces champs = 0).
pub fn curve(
    samples: &[SampleSet],
    ks: &[usize],
    verifier: Option<&dyn DeployableVerifier>,
    weigher: Option<&dyn ReasoningWeigher>,
) -> Vec<CurvePoint> {
    let n = samples.len().max(1) as f64;
    let numeric = samples.iter().all(|s| matches!(s.scorer, ScorerKind::Numeric { .. })) && !samples.is_empty();
    let set_scored = samples.iter().all(|s| matches!(s.scorer, ScorerKind::Set)) && !samples.is_empty();
    ks.iter()
        .map(|&k| {
            let (mut sc, mut wt, mut vf, mut rxw) = (0usize, 0usize, 0usize, 0usize);
            let (mut md, mut tm, mut awm, mut ps, mut wr, mut fab) = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
            let mut sv = 0usize; // HB.T : vote par élément (ensembles)
            let (mut in_off, mut in_on, mut out, mut distinct, mut degen) = (0.0, 0.0, 0.0, 0.0, 0.0);
            let (mut verif_frac, mut override_n) = (0.0, 0usize);
            let (mut disp_var, mut disp_dist, mut disp_ent) = (0.0, 0.0, 0.0);
            for s in samples {
                // Borné aux réponses RÉELLES (sans `.max(1)` qui ferait paniquer un item à 0 réponse —
                // état légitime : K appels tous illisibles, ou K=0).
                let kk = k.min(s.answers.len());
                if mech_correct(s, k, Mechanism::SelfConsistency, None, None) {
                    sc += 1;
                }
                if mech_correct(s, k, Mechanism::Weighted, None, None) {
                    wt += 1;
                }
                if mech_correct(s, k, Mechanism::Verified, verifier, None) {
                    vf += 1;
                }
                if mech_correct(s, k, Mechanism::ReExecWeighted, None, weigher) {
                    rxw += 1;
                }
                if numeric {
                    if mech_correct(s, k, Mechanism::Median, None, None) {
                        md += 1;
                    }
                    if mech_correct(s, k, Mechanism::TrimmedMeanNumeric, None, None) {
                        tm += 1;
                    }
                    if mech_correct(s, k, Mechanism::AuditWeightedMedian, None, weigher) {
                        awm += 1;
                    }
                    if within_range(s, k) {
                        wr += 1;
                    }
                    // Taux de fabrication : la médiane fabrique-t-elle (sortie ∉ candidats) ? (anti-mensonge)
                    if let Some(emitted) = aggregate(s, k, Mechanism::Median, None, None) {
                        if is_fabricated(&emitted, &s.answers[..kk], &s.scorer) {
                            fab += 1;
                        }
                    }
                    let (dv, dd, de) = dispersion_index(s, k, weigher);
                    disp_var += dv;
                    disp_dist += dd;
                    disp_ent += de;
                }
                if set_scored && mech_correct(s, k, Mechanism::SetVote, None, None) {
                    sv += 1;
                }
                if pass_at_k(s, k) {
                    ps += 1;
                }
                // Télémétrie RXW (couverture + déviation vs SC) — seulement si un pondérateur est fourni.
                if let Some(w) = weigher {
                    let (vfrac, overrode) = reexec_item_telemetry(s, k, w);
                    verif_frac += vfrac;
                    if overrode {
                        override_n += 1;
                    }
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
            let nan_if_not = |x: usize| if numeric { x as f64 / n } else { f64::NAN };
            let median = nan_if_not(md);
            // SÉLECTION pure (le genou) : SC/pondéré/vérifié/RXW seulement — toujours ≤ pass@K, monotone.
            let best_selection = (sc.max(wt).max(vf).max(rxw)) as f64 / n;
            // SYNTHÈSE : médiane/tronquée/pondérée — plafond within_range, PAS pass@K (jamais mêlé au genou).
            let best_synthesis = if numeric { (md.max(tm).max(awm)) as f64 / n } else { f64::NAN };
            CurvePoint {
                k,
                self_consistency: sc as f64 / n,
                weighted: wt as f64 / n,
                verified: vf as f64 / n,
                re_exec_weighted: rxw as f64 / n,
                median,
                trimmed_mean: nan_if_not(tm),
                audit_weighted_median: nan_if_not(awm),
                set_vote: if set_scored { sv as f64 / n } else { f64::NAN },
                pass_at_k: ps as f64 / n,
                within_range: nan_if_not(wr),
                fabrication_frac: if numeric { fab as f64 / n } else { f64::NAN },
                dispersion_var: if numeric { disp_var / n } else { f64::NAN },
                dispersion_distinct: if numeric { disp_dist / n } else { f64::NAN },
                dispersion_entropy: if numeric { disp_ent / n } else { f64::NAN },
                best_synthesis,
                best_deployable: best_selection,
                verifiable_frac: verif_frac / n,
                override_rate: override_n as f64 / n,
                mean_in_cache_off: in_off / n,
                mean_in_cache_on: in_on / n,
                mean_out: out / n,
                mean_distinct_frac: distinct / n,
                mean_degenerate_frac: degen / n,
            }
        })
        .collect()
}

/// **HB.4 — diagnostic de DISPERSION (déployable, sans label)** d'un item : (variance numérique des k
/// candidats, fraction distincte = clés distinctes / k, entropie de Shannon normalisée des poids d'audit).
/// Prédit quand la synthèse PEUT aider (forte dispersion non biaisée) ; **nécessaire, non suffisant**.
/// Sans pondérateur, l'entropie vaut 1.0 (uniforme). Ne lit JAMAIS le label (I-HB4-LABELFENCE).
pub fn dispersion_index(s: &SampleSet, k: usize, weigher: Option<&dyn ReasoningWeigher>) -> (f64, f64, f64) {
    let kk = k.min(s.answers.len());
    if kk == 0 {
        return (0.0, 0.0, 1.0);
    }
    let ans = &s.answers[..kk];
    let nums: Vec<f64> = ans.iter().filter_map(parse_num).filter(|f| f.is_finite()).collect();
    let var = if nums.len() >= 2 {
        let mean = nums.iter().sum::<f64>() / nums.len() as f64;
        nums.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / nums.len() as f64
    } else {
        0.0
    };
    let distinct: std::collections::BTreeSet<String> = ans.iter().filter_map(|a| s.scorer.canonical_key(a)).collect();
    let distinct_frac = distinct.len() as f64 / kk as f64;
    // Entropie normalisée des poids d'audit (concentration de la confiance). 1.0 = uniforme (sans pondérateur).
    let entropy = match weigher {
        None => 1.0,
        Some(w) => {
            let prompt_ints = crate::reexec::ints_in_text(s.task.get("text").and_then(|v| v.as_str()).unwrap_or(""));
            let ws: Vec<f64> = ans
                .iter()
                .map(|a| {
                    let sc = crate::reexec::audit_candidate(&prompt_ints, &crate::scorer::answer_text(a));
                    crate::reexec::weight_from(&sc, w.w_min(), w.w_max())
                })
                .collect();
            let total: f64 = ws.iter().sum();
            if total <= 0.0 || ws.len() < 2 {
                1.0
            } else {
                let h: f64 = ws
                    .iter()
                    .map(|&x| x / total)
                    .filter(|&p| p > 0.0)
                    .map(|p| -p * p.ln())
                    .sum();
                (h / (ws.len() as f64).ln()).clamp(0.0, 1.0)
            }
        }
    };
    (var, distinct_frac, entropy)
}

/// **HB.4 — rapport de SYMÉTRIE** (recherche, label-using, cloisonné).
#[derive(Debug, Clone, Serialize)]
pub struct SymmetryReport {
    /// Biais signé moyen : `médiane_item − label`. ≈ 0 ⇒ médiane non biaisée (la synthèse DEVRAIT payer si
    /// dispersée) ; ≠ 0 ⇒ la médiane systématiquement trop haute/basse (synthèse vaine ou corpus-fragile).
    pub signed_bias: f64,
    /// Asymétrie (skewness) des écarts `candidat − médiane_item` agrégés. |skew| &lt; 0,1 ⇒ erreurs ~symétriques ;
    /// &gt; 0,5 ⇒ erreur systématique (tout gain de synthèse à un K est chance/bruit).
    pub skewness: f64,
    /// Fraction d'items où `label ∈ [min,max]` candidats — le **plafond global de la synthèse** (label-using).
    pub within_range_frac: f64,
    /// Items effectivement comptés (numériques, label lisible).
    pub n_items: usize,
}

/// **HB.4 — test de SYMÉTRIE (RECHERCHE, utilise le LABEL, CLOISONNÉ).** Explique POURQUOI la synthèse paie
/// ou non : les erreurs du petit modèle sont-elles ~symétriques/non biaisées (la médiane/moyenne devrait
/// tomber juste si dispersée) ou systématiques (tout gain est coïncidence) ? Il touche le label UNIQUEMENT
/// pour EXPLIQUER — il n'est **jamais** appelé par `aggregate`/`curve`/`verdict` (I-HB4-LABELFENCE), et ne
/// renvoie qu'un rapport, jamais une décision. Utilise TOUS les candidats de chaque item.
pub fn symmetry_test(small: &[SampleSet]) -> SymmetryReport {
    let mut devs: Vec<f64> = Vec::new(); // candidat − médiane_item (label-free, pour l'asymétrie)
    let (mut signed, mut within, mut counted) = (0.0, 0usize, 0usize);
    for s in small {
        let mut nums: Vec<f64> = s.answers.iter().filter_map(parse_num).filter(|f| f.is_finite()).collect();
        if nums.is_empty() {
            continue;
        }
        let lab = match parse_num(&s.label) {
            Some(l) if l.is_finite() => l,
            _ => continue,
        };
        counted += 1;
        nums.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let m = nums.len();
        let med = if m % 2 == 1 { nums[m / 2] } else { (nums[m / 2 - 1] + nums[m / 2]) / 2.0 };
        for &x in &nums {
            devs.push(x - med);
        }
        signed += med - lab; // label-using : biais de la médiane vs vérité
        let (lo, hi) = (nums[0], nums[m - 1]);
        if lab >= lo && lab <= hi {
            within += 1;
        }
    }
    let n = counted.max(1) as f64;
    SymmetryReport { signed_bias: signed / n, skewness: skewness(&devs), within_range_frac: within as f64 / n, n_items: counted }
}

/// Asymétrie (moment standardisé d'ordre 3) d'un échantillon. 0 si &lt; 3 points ou variance nulle.
fn skewness(xs: &[f64]) -> f64 {
    if xs.len() < 3 {
        return 0.0;
    }
    let n = xs.len() as f64;
    let mean = xs.iter().sum::<f64>() / n;
    let sd = (xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n).sqrt();
    if sd <= 1e-12 {
        return 0.0;
    }
    xs.iter().map(|x| ((x - mean) / sd).powi(3)).sum::<f64>() / n
}

/// Télémétrie RXW d'**un** item : (fraction de candidats vérifiables parmi les k, RXW a-t-il dévié de SC ?).
/// Sert à auditer la revendication de *headroom* (si la couverture est basse ou l'override ≈ 0, on le dit).
fn reexec_item_telemetry(s: &SampleSet, k: usize, w: &dyn ReasoningWeigher) -> (f64, bool) {
    let kk = k.min(s.answers.len());
    if kk == 0 {
        return (0.0, false);
    }
    let ans = &s.answers[..kk];
    let prompt_ints = crate::reexec::ints_in_text(s.task.get("text").and_then(|v| v.as_str()).unwrap_or(""));
    let verifiable = ans
        .iter()
        .filter(|a| crate::reexec::audit_candidate(&prompt_ints, &crate::scorer::answer_text(a)).e > 0)
        .count();
    let rxw = reexec_select(&s.task, ans, &s.scorer, w.w_min(), w.w_max());
    let sc = mode(ans, &s.scorer, None);
    let overrode = match (&rxw, &sc) {
        (Some(r), Some(c)) => s.scorer.canonical_key(r) != s.scorer.canonical_key(c),
        (Some(_), None) => true, // RXW a tranché là où SC abstenait → déviation
        _ => false,
    };
    (verifiable as f64 / kk as f64, overrode)
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
    weigher: Option<&dyn ReasoningWeigher>,
) -> Vec<PairedItem> {
    let big_by: BTreeMap<&str, &SampleSet> = big.iter().map(|s| (s.case_id.as_str(), s)).collect();
    small
        .iter()
        .filter_map(|s| {
            let b = big_by.get(s.case_id.as_str())?;
            let sc = mech_correct(s, k_star, mech, ver, weigher);
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

/// **Exposé** : la comparaison appariée (petit `mechanism`@`k` vs gros@1) par item, pour les harnais
/// externes (le Terrain) qui construisent leurs propres `GapCell` / tests de famille. Mêmes items
/// appariés par `case_id` que [`verdict_at_k`].
pub fn paired_correctness(
    small: &[SampleSet],
    big: &[SampleSet],
    mechanism: Mechanism,
    k: usize,
    verifier: Option<&dyn DeployableVerifier>,
    weigher: Option<&dyn ReasoningWeigher>,
) -> Vec<PairedItem> {
    pair_correct(small, big, mechanism, k, verifier, weigher)
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
    weigher: Option<&dyn ReasoningWeigher>,
    delta: f64,
    alpha: f64,
    boot_iters: usize,
    seed: u64,
    price_small: IoPrice,
    price_big: IoPrice,
) -> HbVerdict {
    let pairs = pair_correct(small, big, mechanism, k_star, verifier, weigher);
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
    weigher: Option<&dyn ReasoningWeigher>,
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
            mat.push(ks.iter().map(|&k| mech_correct(s, k, mechanism, verifier, weigher)).collect());
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

/// Extrait un f64 d'une réponse — **délègue au noyau partagé** [`crate::scorer::as_number`] (qui pèle
/// l'emballage de diagnostic puis `{value|text}`, puis prend le dernier nombre d'une chaîne). Une seule
/// définition « dernier nombre » dans EFE (guardrail revue) : la médiane, la sanité du vérificateur et le
/// scoring lisent le même nombre. Réconcilie l'ancien `parse_num` qui ne pelait pas le wrapper diagnostic.
fn parse_num(v: &Value) -> Option<f64> {
    crate::scorer::as_number(v)
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
        assert!(mech_correct(&s, 3, Mechanism::SelfConsistency, None, None));
        // {138,142,144} (pas de majorité, 1 chacun) → abstention → faux MÊME si 142 présent.
        let s2 = num_sample("b", &[138, 142, 144], 142);
        assert!(!mech_correct(&s2, 3, Mechanism::SelfConsistency, None, None), "égalité → abstention = échec");
        // mais pass@3 voit 142 présent.
        assert!(pass_at_k(&s2, 3), "pass@k : la bonne réponse est présente");
    }

    #[test]
    fn sc_amplifies_correlated_error() {
        // Modèle biaisé : majorité unanimement FAUSSE → SC amplifie l'erreur (le poison).
        let s = num_sample("a", &[100, 100, 100, 142], 142);
        assert!(!mech_correct(&s, 4, Mechanism::SelfConsistency, None, None), "mode=100 faux");
        assert!(pass_at_k(&s, 4), "mais 142 est présent → pass@4 vrai");
    }

    #[test]
    fn k1_is_raw_accuracy_equals_pass_at_1() {
        let correct = num_sample("a", &[142], 142);
        assert!(mech_correct(&correct, 1, Mechanism::SelfConsistency, None, None));
        assert!(pass_at_k(&correct, 1));
        let wrong = num_sample("b", &[7], 142);
        assert!(!mech_correct(&wrong, 1, Mechanism::SelfConsistency, None, None));
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
            let sc = mech_correct(&s, k, Mechanism::SelfConsistency, None, None);
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
        assert!(mech_correct(&s, 2, Mechanism::Median, None, None), "médiane sur {{value:…}} = 142 (enveloppe pelée)");
    }

    #[test]
    fn median_peels_diagnostic_gossip_wrapper() {
        // Régression (revue) : `parse_num` délègue désormais à `scorer::as_number`, qui pèle AUSSI
        // l'emballage de diagnostic (gossip/débat/sizing) — l'ancien `parse_num` ne le faisait pas. Une
        // réponse `{value: {value: 141}, gossip: …}` doit donc donner 141 (et la médiane 142), pas None.
        let s = SampleSet::new(
            "a",
            json!({}),
            vec![
                json!({"value": {"value": 141}, "gossip": {"rounds": 3}}),
                json!({"value": {"value": 143}, "gossip": {"rounds": 3}}),
            ],
            json!(142),
            ScorerKind::Numeric { tolerance: 0.0 },
        );
        assert!(mech_correct(&s, 2, Mechanism::Median, None, None), "médiane sous emballage gossip = 142");
    }

    #[test]
    fn median_does_not_panic_on_nan_text() {
        // Régression (revue HIGH) : une réponse « nan » ne doit PAS faire paniquer le tri de la médiane.
        // `as_number` rejette le non-fini (→ None) et `median_numeric` filtre par sûreté.
        let s = SampleSet::new(
            "a",
            json!({}),
            vec![json!({"value": "nan"}), json!({"value": "42"}), json!({"value": "44"})],
            json!(43),
            ScorerKind::Numeric { tolerance: 0.0 },
        );
        // médiane({42,44}) = 43 (le « nan » est écarté, pas de panic).
        assert!(mech_correct(&s, 3, Mechanism::Median, None, None), "nan écarté → médiane(42,44)=43");
    }

    #[test]
    fn median_can_exceed_pass_at_k() {
        // Aucun échantillon n'est 142, mais la médiane de {140,142,144}... non — prenons {141,142,143}→142
        // avec label 142 : 142 n'est tiré par AUCUN (140,144 + un 143). Médiane({140,143,144})=143≠142.
        // Cas net : {141,143} médiane=142 = label, ABSENT des tirages → dépasse pass@k.
        let s = num_sample("a", &[141, 143], 142);
        assert!(!pass_at_k(&s, 2), "142 absent des tirages → pass@2 faux");
        assert!(mech_correct(&s, 2, Mechanism::Median, None, None), "médiane(141,143)=142 = label : la SYNTHÈSE dépasse pass@k");
    }

    // ── vérificateur déployable ──

    #[test]
    fn verified_sc_filters_implausible_answers() {
        // Réponses : -5 (négatif, rejeté), 3.5 (décimal, rejeté), 142, 142. Le vérificateur garde {142,142}.
        let s = SampleSet::new("a", json!({}), vec![json!(-5), json!(3.5), json!(142), json!(142)], json!(142), ScorerKind::Numeric { tolerance: 0.0 });
        let ver = PositiveInteger::default();
        assert!(mech_correct(&s, 4, Mechanism::Verified, Some(&ver), None), "mode parmi les passants = 142");
        // Sans vérificateur : SC nu sur {−5,3.5,142,142} → 142 majoritaire aussi ici.
        assert!(mech_correct(&s, 4, Mechanism::SelfConsistency, None, None));
    }

    // ── vote pondéré ──

    #[test]
    fn weighted_vote_overrides_naive_majority() {
        // {142, 100, 100} : SC nu → 100 (majorité). Mais poids confiants sur 142 → vote pondéré = 142.
        let mut s = num_sample("a", &[142, 100, 100], 142);
        s.weights = Some(vec![5.0, 1.0, 1.0]);
        assert!(!mech_correct(&s, 3, Mechanism::SelfConsistency, None, None), "SC nu → 100 (faux)");
        assert!(mech_correct(&s, 3, Mechanism::Weighted, None, None), "pondéré → 142 (juste)");
    }

    // ── courbe & genou ──

    #[test]
    fn curve_and_knee() {
        // Deux items ; à k croissant, SC s'améliore puis plafonne.
        let samples = vec![
            num_sample("a", &[142, 142, 142, 142], 142),
            num_sample("b", &[7, 50, 50, 50], 50),
        ];
        let c = curve(&samples, &[1, 2, 4], None, None);
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
        let c = curve(std::slice::from_ref(&s), &[3], None, None);
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
        let v = verdict_at_k(&small, &big, Mechanism::SelfConsistency, 3, None, None, 0.05, 0.05, 1000, 7, price, IoPrice { in_per_1k: 1.2, out_per_1k: 1.2 });
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
        assert!(mech_correct(&s, 2, Mechanism::SelfConsistency, None, None), "majorité 'finance' (espaces ignorés) = correct");
    }

    #[test]
    fn curve_does_not_panic_on_zero_answer_item() {
        // Régression (revue) : un item à 0 réponse (K appels tous illisibles / K=0) est légitime →
        // 100 % dégénéré, et ne doit pas faire paniquer la courbe.
        let empty = SampleSet::new("a", json!({}), vec![], json!(42), ScorerKind::Numeric { tolerance: 0.0 });
        let good = num_sample("b", &[42, 42], 42);
        let c = curve(&[empty, good], &[1, 2], None, None);
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
                verdict_sup_k(&small, &big, Mechanism::SelfConsistency, &[1, 3], None, None, 0.5, 0.05, 400, seed).non_inferior_some_k
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
        let v = verdict_sup_k(&small, &big, Mechanism::SelfConsistency, &[1, 3, 5], None, None, 0.05, 0.05, 1500, 11);
        assert!(v.sup_stat > 0.0, "un K franchit dans les données");
        assert!(v.argmax_k >= 3, "le sup est atteint à K≥3 (où SC=1.0)");
        assert!(v.non_inferior_some_k, "borne inf > 0 → ∃K non-inférieur (FWER contrôlé) : {v:?}");
        // Déterminisme.
        let v2 = verdict_sup_k(&small, &big, Mechanism::SelfConsistency, &[1, 3, 5], None, None, 0.05, 0.05, 1500, 11);
        assert_eq!(v.sup_lower, v2.sup_lower);
    }

    #[test]
    fn sup_k_stays_negative_when_small_never_catches_up() {
        // Le petit plafonne sous le gros à tout K → sup-stat ≤ 0, pas de franchissement.
        let small: Vec<SampleSet> = (0..120).map(|i| num_sample(&format!("c{i}"), &[7, 7, 7], 142)).collect();
        let big: Vec<SampleSet> = (0..120).map(|i| num_sample(&format!("c{i}"), &[142], 142)).collect();
        let v = verdict_sup_k(&small, &big, Mechanism::SelfConsistency, &[1, 3], None, None, 0.05, 0.05, 1000, 5);
        assert!(!v.non_inferior_some_k, "petit toujours faux → aucun K non-inférieur");
    }

    // ── HB.3 : le vote pondéré par re-exécution (RXW), sous garde de non-infériorité ──

    use crate::reexec::ReExecAudit;

    /// Construit un item à partir de **textes de raisonnement** réels (réponses `{value: …}`), scorer
    /// numérique. Le label est le dernier-nombre attendu ; chaque texte porte sa propre clé canonique.
    fn cot_sample(case: &str, task_text: &str, texts: &[&str], label: i64) -> SampleSet {
        SampleSet::new(
            case,
            json!({ "text": task_text }),
            texts.iter().map(|t| json!({ "value": *t })).collect(),
            json!(label),
            ScorerKind::Numeric { tolerance: 0.0 },
        )
    }

    #[test]
    fn rxw_recovers_audited_minority_over_prose_majority() {
        // Régime (b) : majorité (3×) BÂCLÉE « 50 » sans équation (E=0, poids plancher) ; minorité (2×)
        // bien RAISONNÉE « 45 » avec arithmétique vérifiée. SC élit la majorité fausse ; RXW re-pondère
        // et, l'appui audité dépassant la marge, élit la minorité juste. C'EST la récupération de headroom.
        let w = ReExecAudit::default();
        let s = cot_sample(
            "a",
            "4 rows of 17 pencils, take away 23",
            &["about 50", "i think 50", "roughly 50", "4 * 17 = 68 ; 68 - 23 = 45", "4 * 17 = 68, minus 23 is 45"],
            45,
        );
        assert!(!mech_correct(&s, 5, Mechanism::SelfConsistency, None, None), "SC → 50 (majorité fausse)");
        assert!(mech_correct(&s, 5, Mechanism::ReExecWeighted, None, Some(&w)), "RXW → 45 (minorité audité-juste)");
    }

    #[test]
    fn rxw_gate_blocks_echo_only_override() {
        // Le garde tient la ligne : la minorité juste « 142 » n'est appuyée que par un **écho** (un fait
        // recopié de l'énoncé, h2=1), tandis que la majorité fausse « 100 » a des équations cassées (poids
        // déprimé). Le vote pondéré bascule vers 142, MAIS l'appui audité (Σh2=1) ne franchit pas la marge
        // (≥ +2) → le garde REND le mode SC (100). Conservateur : un écho ne suffit pas à renverser.
        let w = ReExecAudit::default();
        let s = cot_sample(
            "a",
            "2 times 71 is 142, with 100 more",
            &["90 + 5 = 100", "90 + 5 = 100", "2 * 71 = 142"],
            142,
        );
        // SC élit 100 (2 contre 1). RXW, garde activé, NE dévie PAS (écho insuffisant) → rend 100 aussi.
        assert!(!mech_correct(&s, 3, Mechanism::SelfConsistency, None, None), "SC → 100");
        assert!(!mech_correct(&s, 3, Mechanism::ReExecWeighted, None, Some(&w)), "RXW garde → 100 (écho insuffisant)");
        // Et le pick RXW == pick SC (à l'octet) : jamais pire que SC sur ce flip non décisif.
        let rxw = aggregate(&s, 3, Mechanism::ReExecWeighted, None, Some(&w));
        let sc = aggregate(&s, 3, Mechanism::SelfConsistency, None, None);
        assert_eq!(rxw.and_then(|v| s.scorer.canonical_key(&v)), sc.and_then(|v| s.scorer.canonical_key(&v)));
    }

    #[test]
    fn rxw_all_prose_equals_self_consistency() {
        // Aucun candidat n'écrit d'équation (E=0 partout) → tous poids = plancher → vote pondéré ≡ SC.
        let w = ReExecAudit::default();
        let s = cot_sample("a", "some problem", &["i guess 50", "maybe 50", "probably 45"], 50);
        let rxw = aggregate(&s, 3, Mechanism::ReExecWeighted, None, Some(&w));
        let sc = aggregate(&s, 3, Mechanism::SelfConsistency, None, None);
        assert_eq!(
            rxw.and_then(|v| s.scorer.canonical_key(&v)),
            sc.and_then(|v| s.scorer.canonical_key(&v)),
            "tout-prose → RXW = SC"
        );
    }

    #[test]
    fn rxw_without_weigher_falls_back_to_sc() {
        // Pas de pondérateur fourni → ReExecWeighted = SC nu (repli déployable, jamais un panic/None surprise).
        let s = cot_sample("a", "p", &["4 * 17 = 68 ; 68 - 23 = 45", "about 50", "about 50"], 45);
        assert_eq!(
            mech_correct(&s, 3, Mechanism::ReExecWeighted, None, None),
            mech_correct(&s, 3, Mechanism::SelfConsistency, None, None),
            "sans pondérateur, RXW ≡ SC"
        );
    }

    #[test]
    fn rxw_correct_majority_is_preserved() {
        // Garde-fou « jamais pire sans raison » : si la majorité est DÉJÀ juste, RXW ne doit pas la casser
        // (aucun override sans appui décisif d'une minorité). Majorité juste « 45 » audité + un bruit.
        let w = ReExecAudit::default();
        let s = cot_sample(
            "a",
            "4 rows of 17 minus 23",
            &["4 * 17 = 68 ; 68 - 23 = 45", "4 * 17 = 68 ; 68 - 23 = 45", "noise 99"],
            45,
        );
        assert!(mech_correct(&s, 3, Mechanism::SelfConsistency, None, None), "SC → 45 (majorité juste)");
        assert!(mech_correct(&s, 3, Mechanism::ReExecWeighted, None, Some(&w)), "RXW préserve 45");
    }

    #[test]
    fn rxw_selection_is_bounded_by_pass_at_k() {
        // Invariant de SÉLECTION (≠ synthèse) : RXW n'émet qu'une réponse TIRÉE → RXW correct ⇒ pass@k.
        let w = ReExecAudit::default();
        let s = cot_sample("a", "4 rows of 17 minus 23", &["50", "50", "4 * 17 = 68 ; 68 - 23 = 45"], 45);
        for k in 1..=3 {
            let rxw = mech_correct(&s, k, Mechanism::ReExecWeighted, None, Some(&w));
            assert!(!rxw || pass_at_k(&s, k), "RXW correct ⇒ pass@k (sélection ⊆ tirées) à k={k}");
        }
    }

    #[test]
    fn rxw_curve_reports_coverage_and_override() {
        // La télémétrie d'honnêteté : couverture (fraction vérifiable) et taux d'override sont peuplés.
        let w = ReExecAudit::default();
        let flip = cot_sample("a", "4 rows of 17 minus 23", &["50", "50", "50", "4 * 17 = 68 ; 68 - 23 = 45", "4 * 17 = 68, 68 - 23 = 45"], 45);
        let prose = cot_sample("b", "another", &["12", "12", "13"], 12);
        let c = curve(&[flip, prose], &[5], None, Some(&w));
        // flip : 2 vérifiables / 5 = 0.4 ; prose : 0/3 = 0 → moyenne 0.2.
        assert!((c[0].verifiable_frac - 0.2).abs() < 1e-9, "couverture moyenne : {}", c[0].verifiable_frac);
        // flip dévie (override), prose non → 1/2.
        assert!((c[0].override_rate - 0.5).abs() < 1e-9, "taux d'override : {}", c[0].override_rate);
        // Et RXW récupère le headroom sur le flip (0.5 vs SC 0.0 ; pass@K 1.0).
        assert!(c[0].re_exec_weighted >= c[0].self_consistency, "RXW ≥ SC sur ce jeu");
        assert!(c[0].re_exec_weighted <= c[0].pass_at_k + 1e-9, "RXW ≤ pass@K (sélection)");
    }

    #[test]
    fn rxw_misselects_wrong_setup_right_arithmetic() {
        // HONNÊTETÉ (contrainte V) : un candidat au MAUVAIS montage mais CALCUL juste est audité valide →
        // RXW le classe haut → MAL sélectionné. La re-exécution valide le calcul, jamais la modélisation.
        // Problème « 4 rows of 17, minus 23 » (= 45). Le candidat majoritaire MULTIPLIE au lieu de soustraire
        // « 4 * 17 = 68 » (juste, mais s'arrête à 68, mauvais montage), la minorité juste « 45 » est en prose.
        let w = ReExecAudit::default();
        let s = cot_sample(
            "a",
            "4 rows of 17 pencils, take away 23",
            &["4 * 17 = 68", "4 * 17 = 68", "the remainder is 45"],
            45,
        );
        // RXW préfère le 68 audité (mauvais montage) → faux. On ENCODE la limite, on ne l'assène pas en prose.
        assert!(!mech_correct(&s, 3, Mechanism::SelfConsistency, None, None), "SC → 68 (majorité)");
        assert!(!mech_correct(&s, 3, Mechanism::ReExecWeighted, None, Some(&w)), "RXW mal-sélectionne 68 (calcul juste, montage faux)");
    }

    // ── HB.4 : la synthèse (fabriquer vs sélectionner) ──

    #[test]
    fn trim_frac_is_frozen_constant() {
        // Garde de pré-enregistrement (I-HB4-PREREG) : tout re-réglage silencieux casse CI.
        assert!((TRIM_FRAC - 0.25).abs() < 1e-12, "TRIM_FRAC gelé à 0.25");
    }

    #[test]
    fn trimmed_mean_synthesizes_a_value_absent_from_candidates() {
        // {141,143} : tc=⌊0.25·2⌋=0 → moyenne(141,143)=142 = label, ABSENT des tirages → SYNTHÈSE > pass@K.
        let s = num_sample("a", &[141, 143], 142);
        assert!(!pass_at_k(&s, 2), "142 absent → pass@2 faux (sélection ne peut l'atteindre)");
        assert!(mech_correct(&s, 2, Mechanism::TrimmedMeanNumeric, None, None), "moyenne tronquée fabrique 142");
        // ... et c'est bien une fabrication (anti-mensonge).
        let emitted = aggregate(&s, 2, Mechanism::TrimmedMeanNumeric, None, None).unwrap();
        assert!(is_fabricated(&emitted, &s.answers, &s.scorer), "142 ∉ candidats → fabriqué");
    }

    #[test]
    fn trimmed_mean_no_panic_on_small_k() {
        // k=1 et k=2 : pas de tranche vide / panic (repli médiane si sur-troncature).
        let s = num_sample("a", &[142], 142);
        assert!(mech_correct(&s, 1, Mechanism::TrimmedMeanNumeric, None, None));
        let s2 = num_sample("b", &[140, 144], 142);
        assert!(aggregate(&s2, 2, Mechanism::TrimmedMeanNumeric, None, None).is_some());
    }

    #[test]
    fn within_range_is_the_synthesis_ceiling() {
        // label dans l'enveloppe → atteignable par synthèse ; hors enveloppe → inatteignable.
        let inside = num_sample("a", &[141, 143], 142);
        assert!(within_range(&inside, 2), "142 ∈ [141,143]");
        let outside = num_sample("b", &[141, 143], 200);
        assert!(!within_range(&outside, 2), "200 ∉ [141,143] → synthèse ne peut l'atteindre");
        // pass@K ≤ within_range toujours : ici 142 absent (pass@2 faux) mais dans l'enveloppe (within vrai).
        assert!(!pass_at_k(&inside, 2) && within_range(&inside, 2));
    }

    #[test]
    fn audit_weighted_median_without_weigher_is_plain_median() {
        // Repli déployable : sans pondérateur, AWM ≡ médiane numérique (déterministe, jamais de panic).
        for answers in [&[141i64, 143][..], &[7, 50, 50][..], &[1, 2, 3, 4][..]] {
            let s = num_sample("a", answers, 0);
            let awm = aggregate(&s, answers.len(), Mechanism::AuditWeightedMedian, None, None);
            let med = aggregate(&s, answers.len(), Mechanism::Median, None, None);
            assert_eq!(
                awm.and_then(|v| s.scorer.canonical_key(&v)),
                med.and_then(|v| s.scorer.canonical_key(&v)),
                "AWM sans pondérateur = médiane"
            );
        }
    }

    #[test]
    fn audit_weighted_median_pulls_toward_audited_candidate() {
        // La fusion HB.3×HB.4 : la médiane NUE dirait 50 (majorité), mais l'audit pèse le 45 bien-calculé →
        // la médiane PONDÉRÉE bascule vers 45. C'est la récupération que ni RXW ni la médiane nue n'avaient.
        let w = ReExecAudit::default();
        let s = cot_sample("a", "4 rows of 17 minus 23", &["4 * 17 = 68 ; 68 - 23 = 45", "about 50", "about 50"], 45);
        assert!(!mech_correct(&s, 3, Mechanism::Median, None, None), "médiane nue → 50 (faux)");
        assert!(mech_correct(&s, 3, Mechanism::AuditWeightedMedian, None, Some(&w)), "médiane pondérée → 45 (juste)");
    }

    #[test]
    fn audit_weighted_median_does_not_cure_modeling_error() {
        // HONNÊTETÉ : la fusion hérite de la limite de HB.3 — un montage faux proprement calculé (68) est
        // audité valide → la médiane pondérée s'y déplace → mal-sélectionne. Encodé, pas assené.
        let w = ReExecAudit::default();
        let s = cot_sample("a", "4 rows of 17 minus 23", &["4 * 17 = 68", "4 * 17 = 68", "the answer is 45"], 45);
        assert!(!mech_correct(&s, 3, Mechanism::AuditWeightedMedian, None, Some(&w)), "AWM se déplace vers 68 (montage faux)");
    }

    #[test]
    fn audit_weighted_median_selection_bounded_by_pass_at_k_at_odd_k() {
        // À K impair la médiane pondérée enjambe → un candidat (sélection) ⇒ correct ⇒ pass@k.
        let w = ReExecAudit::default();
        let s = cot_sample("a", "4 rows of 17 minus 23", &["50", "50", "4 * 17 = 68 ; 68 - 23 = 45"], 45);
        for k in [1usize, 3, 5] {
            let awm = mech_correct(&s, k, Mechanism::AuditWeightedMedian, None, Some(&w));
            // À K impair une médiane (même pondérée à traversée nette) rend un candidat tiré.
            if awm {
                assert!(pass_at_k(&s, k), "AWM correct à K impair ⇒ pass@k (sélection) à k={k}");
            }
        }
    }

    #[test]
    fn dispersion_index_separates_scatter_from_cluster() {
        // Diagnostic déployable : forte dispersion (variance, diversité) vs cluster.
        let scatter = num_sample("a", &[140, 141, 142, 143, 144], 142);
        let cluster = num_sample("b", &[50, 50, 50, 50], 50);
        let (sv, sd, se) = dispersion_index(&scatter, 5, None);
        let (cv, cd, _) = dispersion_index(&cluster, 4, None);
        assert!(sv > cv, "variance dispersée {sv} > cluster {cv}");
        assert!(sd > cd, "diversité dispersée {sd} > cluster {cd}");
        assert!((se - 1.0).abs() < 1e-12, "entropie = 1.0 sans pondérateur (uniforme)");
        // L'entropie des poids d'audit est bornée [0,1] avec un pondérateur (pas de NaN/inf).
        let w = ReExecAudit::default();
        let (_, _, e2) = dispersion_index(&scatter, 5, Some(&w));
        assert!((0.0..=1.0).contains(&e2), "entropie ∈ [0,1] : {e2}");
    }

    #[test]
    fn curve_splits_selection_ceiling_from_synthesis() {
        // best_deployable (sélection) ≤ pass@K à tout K (invariant monotone préservé) ; la synthèse peut
        // dépasser pass@K (elle fabrique). Jeu où la médiane @K2 synthétise le label absent.
        let samples = vec![num_sample("a", &[141, 143], 142), num_sample("b", &[7, 9], 8)];
        let c = curve(&samples, &[2], None, None);
        assert!(c[0].best_deployable <= c[0].pass_at_k + 1e-9, "sélection bornée par pass@K");
        assert!(c[0].within_range >= c[0].pass_at_k - 1e-9, "within_range ≥ pass@K");
        // médiane @K2 : (141,143)→142 juste, (7,9)→8 juste → 100 % > pass@2 (0 %, ni 142 ni 8 tirés).
        assert!(c[0].best_synthesis > c[0].pass_at_k, "synthèse dépasse pass@K : {} > {}", c[0].best_synthesis, c[0].pass_at_k);
        assert!(c[0].fabrication_frac > 0.0, "la médiane a fabriqué");
    }

    #[test]
    fn symmetry_test_is_label_using_research_only() {
        // Corpus à erreurs symétriques autour du vrai → biais signé ≈ 0, label dans l'enveloppe.
        let small = vec![
            num_sample("a", &[140, 144], 142), // médiane 142 = label, symétrique
            num_sample("b", &[8, 12], 10),
            num_sample("c", &[19, 21], 20),
        ];
        let r = symmetry_test(&small);
        assert_eq!(r.n_items, 3);
        assert!(r.signed_bias.abs() < 1e-9, "médianes non biaisées : {}", r.signed_bias);
        assert!((r.within_range_frac - 1.0).abs() < 1e-9, "tous les labels dans l'enveloppe");
        // Corpus biaisé : médianes systématiquement trop hautes vs label → biais signé > 0.
        let biased = vec![num_sample("a", &[50, 60], 40), num_sample("b", &[50, 60], 40)];
        assert!(symmetry_test(&biased).signed_bias > 0.0, "biais systématique détecté");
    }

    // ── HB.T : le vote par élément (SetVote), l'opérateur loyal pour les ensembles ──

    fn set_sample(case: &str, cands: &[&[&str]], label: &[&str]) -> SampleSet {
        SampleSet::new(
            case,
            json!({}),
            cands.iter().map(|c| json!(c.iter().collect::<Vec<_>>())).collect(),
            json!(label.iter().collect::<Vec<_>>()),
            ScorerKind::Set,
        )
    }

    #[test]
    fn set_vote_is_per_element_majority() {
        // {a,b},{a,c},{a,b} → a×3, b×2, c×1 sur 3 (2·c≥3) → {a,b} = label.
        let s = set_sample("x", &[&["a", "b"], &["a", "c"], &["a", "b"]], &["a", "b"]);
        assert!(mech_correct(&s, 3, Mechanism::SetVote, None, None), "vote par élément → {{a,b}}");
    }

    #[test]
    fn set_vote_recovers_diverse_errors_where_exact_mode_abstains() {
        // LE cas §5 : erreurs DIVERSES (chaque candidat oublie un élément différent). {a,b},{b,c},{a,c},
        // label {a,b,c}. Mode-exact : 3 ensembles DISTINCTS → aucune majorité → SC abstient → faux. Vote
        // par élément : a,b,c chacun dans 2/3 (2·2≥3) → {a,b,c} = label → SetVote RÉCUPÈRE.
        let s = set_sample("x", &[&["a", "b"], &["b", "c"], &["a", "c"]], &["a", "b", "c"]);
        assert!(!mech_correct(&s, 3, Mechanism::SelfConsistency, None, None), "mode-exact : 3 ensembles distincts → abstention");
        assert!(mech_correct(&s, 3, Mechanism::SetVote, None, None), "vote par élément → {{a,b,c}} (récupère la diversité)");
    }

    #[test]
    fn set_vote_drops_minority_noise() {
        // Un intrus minoritaire (z dans 1/5) est rejeté ; les cibles (dans ≥3/5) sont retenues.
        let s = set_sample(
            "x",
            &[&["a", "b"], &["a", "b"], &["a", "b", "z"], &["a", "b"], &["a"]],
            &["a", "b"],
        );
        assert!(mech_correct(&s, 5, Mechanism::SetVote, None, None), "z minoritaire écarté, {{a,b}} retenu");
        // a×5, b×4, z×1 → a:10≥5, b:8≥5, z:2≥5 ✗.
    }

    #[test]
    fn set_vote_appears_in_curve_for_set_scored() {
        // La courbe peuple `set_vote` (et NaN pour les mécanismes numériques) quand le scorer est Set.
        let s = set_sample("x", &[&["a", "b"], &["b", "c"], &["a", "c"]], &["a", "b", "c"]);
        let c = curve(std::slice::from_ref(&s), &[3], None, None);
        assert!((c[0].set_vote - 1.0).abs() < 1e-9, "SetVote récupère cet item → 100 %");
        assert!(c[0].median.is_nan(), "médiane NaN sur une tâche ensembliste");
    }
}
