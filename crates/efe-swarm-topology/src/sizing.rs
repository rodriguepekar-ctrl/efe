// SPDX-License-Identifier: UNLICENSED
//! # `sizing` — la **taille d'essaim adaptative** (échantillonnage séquentiel borné).
//!
//! K cesse d'être un nombre pour devenir une politique : l'essaim génère par **vagues**
//! de `batch`, observe ce qu'il a (accord, erreur-type, candidats vérifiés), s'arrête
//! dès la cible atteinte — jamais avant `k_min`, jamais après `k_max`, jamais au-delà
//! du budget (ADR-SWARM-014 ; architecture
//! `docs/reports/EFE_Archi_K_Adaptatif_Concurrence_CPU_2026-06-12`).
//!
//! ## La séparation fondatrice (architecture §2.3)
//! Le **batch** (taille des vagues = points d'observation de la règle d'arrêt) est une
//! grandeur **sémantique** : il appartient à la spec et sa valeur par défaut est une
//! **constante fixe** ([`DEFAULT_BATCH`]) — jamais une propriété de la machine, sinon le
//! même run produirait des K_used différents selon le nombre de cœurs et le déterminisme
//! inter-machines (ADR-007) serait détruit. La **largeur** (`width`, permits du sémaphore
//! d'une vague) est une grandeur d'**exécution** : résolue au runtime, elle n'a aucun
//! droit d'influencer une cellule, un record ou une racine (I-CP1).
//!
//! ## Invariants (famille I-AK)
//! - **I-AK1** bornes dures : `k_min ≤ K_used ≤ k_max` — à l'unique exception près,
//!   assumée et tracée, du refus budgétaire de la PREMIÈRE vague (I-AK4 prime :
//!   plutôt `K_used = 0` + `stop_reason = "budget"` + run marqué non-propre par la
//!   sonde du SDK, que d'émettre une vague impayable).
//! - **I-AK2** décision pure : l'observable et l'arrêt sont des fonctions pures des
//!   candidats présents — ni horloge, ni aléa, ni largeur ; la trajectoire se re-dérive
//!   hors ligne (Mode A : `replay_outputs` ; Mode C : les sorties N17).
//! - **I-AK3** insensibilité au découpage : le candidat i ne dépend que de
//!   `(task, variant = i)` — la vague qui le porte n'injecte **rien** dans la tâche
//!   (contrairement au gossip, qui injecte `round` à dessein).
//! - **I-AK4** vagues payables : aucune vague n'est émise que le budget ne paie
//!   entièrement (sonde coopérative) — zéro sentinelle dans l'échantillon.
//! - **I-AK5** agrégation inchangée : le sizer choisit COMBIEN de candidats, jamais
//!   quoi en faire.

use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::consensus::agreement;
use crate::iterative::num_of;
use crate::traits::Generator;

/// Taille de vague par défaut — **constante de spec, jamais dérivée de la machine**
/// (architecture §2.3 : batch = sémantique ; width = exécution).
pub const DEFAULT_BATCH: usize = 4;

/// Sonde de budget coopérative (I-AK4) : « le budget peut-il payer `n` appels de plus,
/// entièrement ? ». Injectée par la couche qui possède la gouvernance (le SDK, via son
/// `BudgetGuard`) — la topologie reste découplée du modèle de prix.
pub type BudgetProbe = Arc<dyn Fn(usize) -> bool + Send + Sync>;

/// **Sonde de deadline coopérative (ADR-018, I-AD2)** — le **jumeau temporel** de
/// [`BudgetProbe`]. Consultée au MÊME point de la boucle de vagues (avant d'émettre une
/// vague), elle répond « la fenêtre temporelle reste-t-elle ouverte ? » (`true` = il
/// reste du temps, émettre ; `false` = deadline atteinte, arrêt propre). Injectée par le
/// SDK (qui possède la gouvernance, via son `DeadlineGuard` à horloge) — la topologie
/// ignore l'horloge tout comme elle ignore le modèle de prix. La sonde ne prend AUCUN
/// argument : le temps n'est pas indexé par la taille de la vague (asymétrie assumée avec
/// le budget, qui demande « payer *n* de plus ? »). Le scellement du partiel — vague déjà
/// produite, intacte dans le recorder — est ce qui distingue ce coopératif du `tokio`
/// backstop dur (§AD-3) : ici l'arrêt tombe sur une frontière de vague, jamais en plein vol.
pub type DeadlineProbe = Arc<dyn Fn() -> bool + Send + Sync>;

// ─────────────────────────────────────────────────────────────────────────────
//  StopRule — la cible d'arrêt (une par famille de tâches)
// ─────────────────────────────────────────────────────────────────────────────

/// Règle d'arrêt de l'échantillonnage séquentiel. L'observable est une **fonction pure**
/// des candidats présents (I-AK2) ; `satisfied` compare à la cible.
#[derive(Debug, Clone, PartialEq)]
pub enum StopRule {
    /// Catégoriel (consensus, labels) : fraction de paires de candidats identiques
    /// (réutilise [`crate::consensus::selector`]) ≥ seuil.
    Agreement { threshold: f64 },
    /// Numérique (estimation) : erreur-type `s/√n` sur les champs `value` ≤ cible.
    /// (< 2 valeurs numériques ⇒ +∞ : jamais convergé d'office — parité I-IT5.)
    StdError { target: f64 },
    /// Vérifiable (T1 Generate-Verify) : nombre de candidats ayant **passé** le
    /// vérificateur ≥ quota. L'observable vient du vérificateur (fourni par T1) —
    /// `observe_candidates` renvoie `None` pour cette règle.
    VerifiedQuota { quota: usize },
}

impl StopRule {
    /// Étiquette stable (trace de sizing, artefacts, dashboard).
    pub fn label(&self) -> &'static str {
        match self {
            StopRule::Agreement { .. } => "agreement",
            StopRule::StdError { .. } => "std_error",
            StopRule::VerifiedQuota { .. } => "verified_quota",
        }
    }

    /// La règle exige-t-elle un vérificateur (observable hors des seuls candidats) ?
    pub fn needs_verifier(&self) -> bool {
        matches!(self, StopRule::VerifiedQuota { .. })
    }

    /// Observable **pur** sur les candidats présents. `None` si la règle exige un
    /// vérificateur ([`StopRule::VerifiedQuota`] — T1 fournit le comptage).
    pub fn observe_candidates(&self, cands: &[Value]) -> Option<f64> {
        match self {
            StopRule::Agreement { .. } => Some(agreement(cands)),
            StopRule::StdError { .. } => Some(std_error(cands)),
            StopRule::VerifiedQuota { .. } => None,
        }
    }

    /// La cible est-elle atteinte pour cet observable ? (`NaN` ⇒ jamais — repli sûr.)
    pub fn satisfied(&self, observed: f64) -> bool {
        match self {
            StopRule::Agreement { threshold } => observed >= *threshold,
            StopRule::StdError { target } => observed <= *target,
            StopRule::VerifiedQuota { quota } => observed >= *quota as f64,
        }
    }

    /// Forme JSON de la cible (bloc `"sizing".policy.stop`).
    pub fn to_json(&self) -> Value {
        match self {
            StopRule::Agreement { threshold } => json!({ "agreement": threshold }),
            StopRule::StdError { target } => json!({ "se_target": target }),
            StopRule::VerifiedQuota { quota } => json!({ "verified_quota": quota }),
        }
    }
}

/// Erreur-type `s/√n` (écart-type **échantillon**, dénominateur n−1) des candidats
/// numériques (champ `value` ou scalaire nu). < 2 valeurs ⇒ `+∞` (jamais convergé
/// d'office). Fonction pure (I-AK2 / I-IT5).
fn std_error(cands: &[Value]) -> f64 {
    let xs: Vec<f64> = cands.iter().filter_map(num_of).collect();
    let n = xs.len();
    if n < 2 {
        return f64::INFINITY;
    }
    let mean = xs.iter().sum::<f64>() / n as f64;
    let var = xs.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / (n as f64 - 1.0);
    (var / n as f64).sqrt()
}

// ─────────────────────────────────────────────────────────────────────────────
//  KPlan — la politique bornée
// ─────────────────────────────────────────────────────────────────────────────

/// La taille d'essaim comme **politique bornée** : `[k_min, k_max]`, vagues de `batch`,
/// arrêt sur [`StopRule`]. `batch` appartient à la **spec** (§2.3) — défaut
/// [`DEFAULT_BATCH`], constante.
#[derive(Debug, Clone, PartialEq)]
pub struct KPlan {
    pub k_min: usize,
    pub k_max: usize,
    pub batch: usize,
    pub stop: StopRule,
}

impl KPlan {
    /// Normalise les bornes : `k_min ≥ 1`, `k_max ≥ k_min`, `batch ≥ 1` (I-AK1 dur).
    pub fn normalized(mut self) -> Self {
        self.k_min = self.k_min.max(1);
        self.k_max = self.k_max.max(self.k_min);
        self.batch = self.batch.max(1);
        self
    }

    fn policy_json(&self) -> Value {
        json!({
            "k_min": self.k_min,
            "k_max": self.k_max,
            "batch": self.batch,
            "stop": self.stop.to_json(),
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  SizingTrace — « pourquoi ce run a coûté K_used et pas k_max »
// ─────────────────────────────────────────────────────────────────────────────

/// Pourquoi l'échantillonnage s'est arrêté.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// La cible de la règle est atteinte (avec ≥ k_min candidats).
    Target,
    /// Le plafond `k_max` est atteint sans satisfaire la cible.
    KMax,
    /// Le budget ne peut plus payer une vague entière (I-AK4) — arrêt propre sur
    /// l'échantillon présent.
    Budget,
    /// **ADR-018 (I-AD2)** : la fenêtre temporelle (deadline run) est atteinte — arrêt
    /// **propre** sur l'échantillon présent, exactement comme le budget. La vague en cours
    /// n'est pas émise (jamais de demi-vague) ; le partiel déjà produit est scellable et
    /// re-prouvable. Comme `Budget`, ce motif peut, à l'extrême, tronquer SOUS `k_min`
    /// (le temps prime la borne basse — I-AD exception, jumelle de l'exception I-AK4).
    Deadline,
}

impl StopReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            StopReason::Target => "target",
            StopReason::KMax => "k_max",
            StopReason::Budget => "budget",
            StopReason::Deadline => "deadline",
        }
    }
}

/// Un point d'observation : « après `k` candidats, l'observable valait `observed` ».
#[derive(Debug, Clone, PartialEq)]
pub struct BatchPoint {
    pub k: usize,
    pub observed: f64,
}

/// La trace de dimensionnement — **narrative** (confort d'audit immédiat) : la preuve,
/// elle, n'en a pas besoin, car la trajectoire entière se re-dérive des records scellés
/// (architecture §7.2). Rejoint la sortie du run (bloc frère de `"gossip"`) et part sur
/// le bus (`sizer.decided`).
#[derive(Debug, Clone, PartialEq)]
pub struct SizingTrace {
    /// Taille d'essaim réellement dépensée (`k_min ≤ k_used ≤ k_max` — I-AK1 ; seule
    /// exception : `0` si le budget refuse la PREMIÈRE vague, `stop_reason = Budget`).
    pub k_used: usize,
    /// Trajectoire des observations, une par vague.
    pub batches: Vec<BatchPoint>,
    pub stop_reason: StopReason,
}

impl SizingTrace {
    /// Le bloc `"sizing"` de la sortie (architecture §4.5).
    pub fn to_json(&self, plan: &KPlan) -> Value {
        json!({
            "policy": plan.policy_json(),
            "k_used": self.k_used,
            "batches": self.batches.iter().map(|b| json!({ "k": b.k, "observed": round6(b.observed) })).collect::<Vec<_>>(),
            "stop_reason": self.stop_reason.as_str(),
        })
    }
}

/// Arrondi de présentation (la trace est narrative ; la preuve re-dérive l'exact).
fn round6(x: f64) -> Value {
    if x.is_finite() {
        Value::from((x * 1e6).round() / 1e6)
    } else {
        Value::Null // +∞ (pas-encore-observable) : null en JSON
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  run_waves — la boucle vague → observation pure → décision bornée
// ─────────────────────────────────────────────────────────────────────────────

/// Une vague indexée couvrant les **positions** `[from, from+want)`, exécutée sur
/// `min(want, width)` voies. Les résultats sont **placés par position** (ordre déterministe) — le
/// motif exact de `ConsensusTopology::generate`. Le **profil** (variant d'échantillonnage) exécuté
/// à la position `p` est `schedule[p]` ; sans ordonnancement (`None`) ou hors-borne ⇒
/// `variant = p` — l'identité, soit le comportement historique I-AK3 (le candidat à la position
/// `p` ne dépend que de `(task, variant = p)`). L'ordonnancement (ADR-019 R-b/R-c) ne fait que
/// **réassigner quel profil tombe à quelle position** ; il n'injecte rien dans la tâche.
async fn spawn_wave(
    gen: &Arc<dyn Generator>,
    task: &Value,
    from: usize,
    want: usize,
    width: usize,
    schedule: Option<&[usize]>,
) -> Vec<Value> {
    let sem = Arc::new(Semaphore::new(width.max(1)));
    let mut set: JoinSet<(usize, Value)> = JoinSet::new();
    for i in 0..want {
        let g = gen.clone();
        let task = task.clone();
        let sem = sem.clone();
        let pos = from + i;
        // R-b/R-c (ADR-019) : le profil exécuté à la position `pos` est `schedule[pos]` ; `None`
        // ou hors-borne ⇒ `variant = pos` (identité, I-AK3 historique — byte-identique).
        let variant = schedule.and_then(|s| s.get(pos).copied()).unwrap_or(pos);
        set.spawn(async move {
            let _p = sem.acquire_owned().await.expect("semaphore");
            (i, g.generate(&task, variant).await)
        });
    }
    let mut out = vec![Value::Null; want];
    while let Some(r) = set.join_next().await {
        if let Ok((i, v)) = r {
            out[i] = v;
        }
    }
    out
}

/// **La boucle du sizer** (architecture §4.2) : remplir `k_min` d'abord, puis par
/// vagues de `batch` ; après chaque vague, `observe(&candidats)` (fonction **pure**,
/// fournie par l'appelant — T1 y compte les vérifiés, le consensus y applique la règle
/// pure) ; arrêt sur cible, sur `k_max`, ou sur budget (I-AK4 : la sonde est consultée
/// **avant** la vague — jamais de vague partielle, zéro sentinelle dans l'échantillon).
///
/// Renvoie les candidats **en ordre de variant** (0..K_used) et la trace.
// La boucle du sizer prend ses paramètres EXPLICITES (générateur, plan, largeur, et les DEUX
// sondes coopératives jumelles budget+deadline — ADR-018) plutôt qu'un struct-config opaque :
// chacun a un rôle distinct et documenté. Comme `run_judges`, on assume le compte d'arguments.
#[allow(clippy::too_many_arguments)]
pub async fn run_waves(
    gen: &Arc<dyn Generator>,
    task: &Value,
    plan: &KPlan,
    width: usize,
    probe: Option<&BudgetProbe>,
    deadline: Option<&DeadlineProbe>,
    observe: &(dyn Fn(&[Value]) -> f64 + Sync),
) -> (Vec<Value>, SizingTrace) {
    // Délégation à profil **identité** (`variant = position`) — strictement le comportement
    // historique (I-AK3). L'ordonnancement réputationnel passe par [`run_waves_scheduled`].
    run_waves_scheduled(gen, task, plan, width, probe, deadline, None, observe).await
}

/// [`run_waves`] avec **ordonnancement de profils** (ADR-019 read-back B, R-b/R-c) : `schedule[p]`
/// impose le **profil d'échantillonnage** exécuté à la **position** `p` de la séquence de vagues
/// (cf. `Reputation::profile_schedule` — un ordre réputation-décroissant à rotation d'exploration
/// keyée sur `total`). `schedule == None` ⇒ identité (`variant = position`), strictement
/// [`run_waves`].
///
/// Le sizer choisit toujours **COMBIEN** de candidats (I-AK5 inchangé) ; l'ordonnancement choisit,
/// **à bornes et budget identiques**, **dans quel ordre** les profils sont tentés — donc lesquels
/// survivent à un arrêt anticipé (R-c, **gating doux** : les profils du bas de classement ne sont
/// pas atteints, jamais supprimés). `schedule` étant une **permutation de `[0, k_max)`**, le
/// plafond `k_max` (I-AK1) et l'enveloppe budgétaire (ADR-018) restent **intacts** : l'ordre ne
/// peut que faire **converger plus tôt** (moins de vagues), jamais dépasser le plafond.
#[allow(clippy::too_many_arguments)]
pub async fn run_waves_scheduled(
    gen: &Arc<dyn Generator>,
    task: &Value,
    plan: &KPlan,
    width: usize,
    probe: Option<&BudgetProbe>,
    deadline: Option<&DeadlineProbe>,
    schedule: Option<&[usize]>,
    observe: &(dyn Fn(&[Value]) -> f64 + Sync),
) -> (Vec<Value>, SizingTrace) {
    let plan = plan.clone().normalized();
    let mut cands: Vec<Value> = Vec::with_capacity(plan.k_min);
    let mut batches: Vec<BatchPoint> = Vec::new();
    let stop_reason = loop {
        // ① taille de la prochaine vague : plancher d'abord, puis par batch, borné k_max.
        let want = if cands.len() < plan.k_min {
            plan.k_min - cands.len()
        } else {
            (plan.k_max - cands.len()).min(plan.batch)
        };
        if want == 0 {
            break StopReason::KMax; // I-AK1 : plafond dur atteint
        }
        // ② I-AK4 : la vague entière doit être payable — sinon arrêt propre.
        if let Some(p) = probe {
            if !p(want) {
                break StopReason::Budget;
            }
        }
        // ②bis ADR-018 (I-AD2) : la fenêtre temporelle doit rester ouverte — sinon arrêt
        // propre, AU MÊME point coopératif que le budget. On consulte AVANT d'émettre :
        // la vague déjà produite reste donc une frontière nette (jamais de demi-vague),
        // ce qui rend le partiel scellable et re-prouvable (R0). Twin exact de I-AK4.
        if let Some(d) = deadline {
            if !d() {
                break StopReason::Deadline;
            }
        }
        // ③ la vague (∥ bornée par width — l'exécution, jamais la sémantique). `schedule` réassigne
        //    quel profil tombe à quelle position ; `None` ⇒ identité (variant = position, I-AK3).
        let wave = spawn_wave(gen, task, cands.len(), want, width, schedule).await;
        cands.extend(wave);
        // ④ observation PURE sur la vérité présente (I-AK2).
        let observed = observe(&cands);
        batches.push(BatchPoint { k: cands.len(), observed });
        if cands.len() >= plan.k_min && plan.stop.satisfied(observed) {
            break StopReason::Target;
        }
    };
    let trace = SizingTrace { k_used: cands.len(), batches, stop_reason };
    (cands, trace)
}

/// [`run_waves`] pour les règles **pures** ([`StopRule::Agreement`] / [`StopRule::StdError`]) :
/// l'observable vient de la règle elle-même. Une règle à vérificateur
/// ([`StopRule::VerifiedQuota`]) observerait `NaN` (jamais satisfaite) → court jusqu'à
/// `k_max` — repli sûr ; la couche de résolution du SDK refuse cette combinaison en amont.
pub async fn run_waves_pure(
    gen: &Arc<dyn Generator>,
    task: &Value,
    plan: &KPlan,
    width: usize,
    probe: Option<&BudgetProbe>,
    deadline: Option<&DeadlineProbe>,
) -> (Vec<Value>, SizingTrace) {
    run_waves_pure_scheduled(gen, task, plan, width, probe, deadline, None).await
}

/// [`run_waves_pure`] avec **ordonnancement de profils** (ADR-019 R-b/R-c) : `schedule[p]` impose le
/// profil exécuté à la position `p` (cf. `Reputation::profile_schedule`). `schedule == None` ⇒
/// identité, strictement [`run_waves_pure`]. C'est l'entrée des règles **pures**
/// ([`StopRule::Agreement`] / [`StopRule::StdError`]) sous priorisation réputationnelle.
#[allow(clippy::too_many_arguments)]
pub async fn run_waves_pure_scheduled(
    gen: &Arc<dyn Generator>,
    task: &Value,
    plan: &KPlan,
    width: usize,
    probe: Option<&BudgetProbe>,
    deadline: Option<&DeadlineProbe>,
    schedule: Option<&[usize]>,
) -> (Vec<Value>, SizingTrace) {
    let stop = plan.stop.clone();
    let observe = move |c: &[Value]| stop.observe_candidates(c).unwrap_or(f64::NAN);
    run_waves_scheduled(gen, task, plan, width, probe, deadline, schedule, &observe).await
}

// ─────────────────────────────────────────────────────────────────────────────
//  Tests (oracles purs + boucle)
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    /// Générateur numérique déterministe : valeurs 100 ± 2 alternées par variant —
    /// l'écart-type échantillon est stable, l'erreur-type fond en 1/√n.
    struct Alternating;
    #[async_trait]
    impl Generator for Alternating {
        async fn generate(&self, _task: &Value, variant: usize) -> Value {
            let sign = if variant % 2 == 0 { 1.0 } else { -1.0 };
            json!({ "value": 100.0 + 2.0 * sign })
        }
    }

    /// Générateur constant (accord parfait dès la première vague).
    struct Uniform;
    #[async_trait]
    impl Generator for Uniform {
        async fn generate(&self, _task: &Value, _variant: usize) -> Value {
            json!({ "value": "x" })
        }
    }

    /// Générateur tous-distincts (accord nul — la cible Agreement est inatteignable).
    struct AllDistinct;
    #[async_trait]
    impl Generator for AllDistinct {
        async fn generate(&self, _task: &Value, variant: usize) -> Value {
            json!({ "value": format!("v{variant}") })
        }
    }

    fn plan(k_min: usize, k_max: usize, batch: usize, stop: StopRule) -> KPlan {
        KPlan { k_min, k_max, batch, stop }
    }

    // ── observables purs ─────────────────────────────────────────────────────
    #[test]
    fn std_error_is_pure_and_shrinks_in_sqrt_n() {
        let mk = |n: usize| -> Vec<Value> {
            (0..n).map(|i| json!({ "value": 100.0 + if i % 2 == 0 { 2.0 } else { -2.0 } })).collect()
        };
        assert_eq!(std_error(&mk(0)), f64::INFINITY, "0 valeur → +∞ (jamais d'office)");
        assert_eq!(std_error(&mk(1)), f64::INFINITY, "1 valeur → +∞");
        let se4 = std_error(&mk(4));
        let se16 = std_error(&mk(16));
        assert!(se16 < se4, "l'erreur-type fond avec n : {se4} → {se16}");
        // s ≈ 2.07 (échantillon) ; SE(16) = s/4 ≈ 0.516.
        assert!((se16 - 0.5163).abs() < 0.01, "SE(16) ≈ 0.516, eu {se16}");
    }

    #[test]
    fn stop_rules_satisfaction_semantics() {
        let a = StopRule::Agreement { threshold: 0.9 };
        assert!(a.satisfied(0.95) && !a.satisfied(0.5));
        let s = StopRule::StdError { target: 0.5 };
        assert!(s.satisfied(0.4) && !s.satisfied(0.6));
        assert!(!s.satisfied(f64::INFINITY) && !s.satisfied(f64::NAN), "∞/NaN : jamais satisfait");
        let q = StopRule::VerifiedQuota { quota: 2 };
        assert!(q.satisfied(2.0) && !q.satisfied(1.0));
        assert!(q.needs_verifier() && !a.needs_verifier());
        assert_eq!(q.observe_candidates(&[]), None, "quota : l'observable vient du vérificateur");
    }

    #[test]
    fn kplan_normalizes_hard_bounds() {
        let p = plan(0, 0, 0, StopRule::Agreement { threshold: 0.9 }).normalized();
        assert_eq!((p.k_min, p.k_max, p.batch), (1, 1, 1), "I-AK1 : bornes ≥ 1, k_max ≥ k_min");
        let p = plan(8, 4, 2, StopRule::Agreement { threshold: 0.9 }).normalized();
        assert_eq!((p.k_min, p.k_max), (8, 8), "k_max relevé au plancher");
    }

    // ── la boucle ────────────────────────────────────────────────────────────
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn stops_at_k_min_when_target_is_immediate() {
        let gen: Arc<dyn Generator> = Arc::new(Uniform);
        let p = plan(4, 24, 4, StopRule::Agreement { threshold: 0.9 });
        let (cands, trace) = run_waves_pure(&gen, &json!({}), &p, 8, None, None).await;
        assert_eq!(cands.len(), 4, "unanimes → arrêt au plancher k_min");
        assert_eq!(trace.k_used, 4);
        assert_eq!(trace.stop_reason, StopReason::Target);
        assert_eq!(trace.batches.len(), 1, "une seule vague (k_min) a suffi");
        assert!((trace.batches[0].observed - 1.0).abs() < 1e-9);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn runs_to_k_max_when_target_is_unreachable() {
        let gen: Arc<dyn Generator> = Arc::new(AllDistinct);
        let p = plan(4, 10, 4, StopRule::Agreement { threshold: 0.9 });
        let (cands, trace) = run_waves_pure(&gen, &json!({}), &p, 8, None, None).await;
        assert_eq!(cands.len(), 10, "I-AK1 : plafond dur");
        assert_eq!(trace.stop_reason, StopReason::KMax);
        // vagues : 4 (k_min) puis 4 puis 2 (rognée au plafond).
        let ks: Vec<usize> = trace.batches.iter().map(|b| b.k).collect();
        assert_eq!(ks, vec![4, 8, 10], "la dernière vague est rognée à k_max");
        // candidats en ORDRE DE VARIANT (I-AK3) — insensible au découpage.
        for (i, c) in cands.iter().enumerate() {
            assert_eq!(c, &json!({ "value": format!("v{i}") }), "candidat {i} = variant {i}");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn schedule_reassigns_profile_per_position_and_keeps_k_max_ceiling() {
        // ADR-019 R-b/R-c : `schedule[p]` impose le profil exécuté à la position p. Sur AllDistinct
        // (chaque variant ⇒ valeur distincte), on LIT directement quel profil a tourné à chaque
        // position. L'identité (None) reste byte-identique au comportement historique (I-AK3) ; une
        // permutation conserve le MÊME plafond k_max (I-AK1) et le MÊME multiset (enveloppe budget
        // intacte, ADR-018) — l'ordre ne fait que réassigner, jamais dépasser ni inventer.
        let gen: Arc<dyn Generator> = Arc::new(AllDistinct);
        let p = plan(4, 8, 4, StopRule::Agreement { threshold: 0.9 }); // accord impossible → court à k_max

        let (id_cands, id_trace) = run_waves_pure_scheduled(&gen, &json!({}), &p, 8, None, None, None).await;
        assert_eq!(id_trace.k_used, 8, "cible inatteignable → plafond k_max");
        for (i, c) in id_cands.iter().enumerate() {
            assert_eq!(c, &json!({ "value": format!("v{i}") }), "None ⇒ identité : position {i} = variant {i}");
        }

        let schedule = [7usize, 6, 5, 4, 3, 2, 1, 0]; // permutation explicite des 8 profils
        let (sc_cands, sc_trace) =
            run_waves_pure_scheduled(&gen, &json!({}), &p, 8, None, None, Some(&schedule)).await;
        assert_eq!(sc_trace.k_used, 8, "permutation ⇒ MÊME plafond k_max (I-AK1, enveloppe inchangée)");
        for (pos, c) in sc_cands.iter().enumerate() {
            let prof = schedule[pos];
            assert_eq!(c, &json!({ "value": format!("v{prof}") }), "position {pos} = profil schedule[{pos}]={prof}");
        }
        let mut a: Vec<_> = id_cands.iter().map(|c| c["value"].as_str().unwrap().to_owned()).collect();
        let mut b: Vec<_> = sc_cands.iter().map(|c| c["value"].as_str().unwrap().to_owned()).collect();
        a.sort();
        b.sort();
        assert_eq!(a, b, "ordonnancement = permutation : même multiset de profils, rien d'inventé ni supprimé");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn std_error_target_stops_mid_course() {
        let gen: Arc<dyn Generator> = Arc::new(Alternating);
        // valeurs 100 ± 2 : s(n) = √(SS/(n−1)) avec SS = 4n → SE(4) = 1.155,
        // SE(8) = √(32/7)/√8 ≈ 0.756, SE(12) ≈ 0.603. Cible 0.8 → l'arrêt tombe à n=8
        // (pas à 4) — la cible de précision pilote réellement le K.
        let p = plan(4, 16, 4, StopRule::StdError { target: 0.8 });
        let (cands, trace) = run_waves_pure(&gen, &json!({}), &p, 8, None, None).await;
        assert_eq!(cands.len(), 8, "la cible de précision tombe à n=8");
        assert_eq!(trace.stop_reason, StopReason::Target);
        assert_eq!(trace.batches.len(), 2);
        assert!(trace.batches[0].observed > 0.8 && trace.batches[1].observed <= 0.8);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn budget_probe_refuses_unpayable_wave_cleanly() {
        let gen: Arc<dyn Generator> = Arc::new(AllDistinct);
        let p = plan(4, 16, 4, StopRule::Agreement { threshold: 0.9 });
        // la sonde paie la 1ʳᵉ vague (k_min = 4) puis refuse : arrêt PROPRE sur 4,
        // zéro sentinelle (I-AK4).
        let calls = std::sync::atomic::AtomicUsize::new(0);
        let probe: BudgetProbe = {
            let calls = Arc::new(calls);
            Arc::new(move |_n| calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0)
        };
        let (cands, trace) = run_waves_pure(&gen, &json!({}), &p, 8, Some(&probe), None).await;
        assert_eq!(cands.len(), 4, "l'échantillon s'arrête sur la dernière vague payée");
        assert_eq!(trace.stop_reason, StopReason::Budget);
        assert!(cands.iter().all(|c| !c.is_null()), "zéro sentinelle dans l'échantillon");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn first_wave_budget_refusal_yields_zero_sample_honestly() {
        // Le cas-limite assumé de I-AK1 (doc du module) : si MÊME la première vague
        // (k_min) est impayable, l'échantillon est VIDE — k_used = 0, raison `budget`,
        // zéro candidat fantôme. La sonde du SDK marque alors le run non-propre.
        let gen: Arc<dyn Generator> = Arc::new(AllDistinct);
        let p = plan(4, 16, 4, StopRule::Agreement { threshold: 0.9 });
        let probe: BudgetProbe = Arc::new(|_n| false); // budget déjà à sec
        let (cands, trace) = run_waves_pure(&gen, &json!({}), &p, 8, Some(&probe), None).await;
        assert!(cands.is_empty(), "aucune vague émise = aucun candidat");
        assert_eq!(trace.k_used, 0);
        assert_eq!(trace.stop_reason, StopReason::Budget);
        assert!(trace.batches.is_empty(), "aucune observation inventée");
    }

    // ── ADR-018 (I-AD2) : la sonde de deadline, jumelle de la sonde de budget ──────────
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn deadline_probe_stops_cleanly_at_a_wave_boundary() {
        // I-AD2 : la sonde de deadline laisse passer la 1ʳᵉ vague (k_min) puis ferme la
        // fenêtre → arrêt PROPRE sur l'échantillon présent, frontière de vague nette, zéro
        // sentinelle. Jumeau EXACT de `budget_probe_refuses_unpayable_wave_cleanly`.
        let gen: Arc<dyn Generator> = Arc::new(AllDistinct);
        let p = plan(4, 16, 4, StopRule::Agreement { threshold: 0.9 });
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let dl: DeadlineProbe = {
            let calls = calls.clone();
            Arc::new(move || calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0)
        };
        let (cands, trace) = run_waves_pure(&gen, &json!({}), &p, 8, None, Some(&dl)).await;
        assert_eq!(cands.len(), 4, "arrêt sur la dernière vague émise (k_min)");
        assert_eq!(trace.stop_reason, StopReason::Deadline);
        assert!(cands.iter().all(|c| !c.is_null()), "frontière de vague nette : zéro sentinelle");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn deadline_on_first_wave_yields_zero_sample_honestly() {
        // Cas-limite assumé (twin de l'exception budget I-AK4) : fenêtre déjà fermée →
        // AUCUNE vague émise, k_used = 0, raison `deadline`. Le SDK qualifie alors le run
        // `DeadlineExceeded` — partiel vide, mais honnête et re-prouvable.
        let gen: Arc<dyn Generator> = Arc::new(AllDistinct);
        let p = plan(4, 16, 4, StopRule::Agreement { threshold: 0.9 });
        let dl: DeadlineProbe = Arc::new(|| false); // hors fenêtre d'emblée
        let (cands, trace) = run_waves_pure(&gen, &json!({}), &p, 8, None, Some(&dl)).await;
        assert!(cands.is_empty(), "hors fenêtre d'emblée = aucun candidat");
        assert_eq!(trace.k_used, 0);
        assert_eq!(trace.stop_reason, StopReason::Deadline);
        assert!(trace.batches.is_empty(), "aucune observation inventée");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn first_ceiling_wins_and_is_named() {
        // I-AD5 (composition) — les DEUX plafonds coopératifs vivent dans la même boucle :
        // émettre SSI budget ∧ deadline. Le PREMIER plafond atteint arrête la vague, et la
        // RAISON le NOMME. « Premier » = précédence DÉTERMINISTE (le budget est testé avant
        // le deadline dans le code), PAS « premier dans le temps » : les deux sondes sont
        // consultées au même instant, avant la même vague. Ce gardien couvre les DEUX
        // directions + le cas double (jadis seul le sens budget-gagne était testé).
        let gen: Arc<dyn Generator> = Arc::new(AllDistinct);
        let p = plan(4, 16, 4, StopRule::Agreement { threshold: 0.9 });
        let open_b: BudgetProbe = Arc::new(|_n| true);
        let dry_b: BudgetProbe = Arc::new(|_n| false);
        let open_d: DeadlineProbe = Arc::new(|| true);
        let closed_d: DeadlineProbe = Arc::new(|| false);

        // ① plafond COÛT atteint, temps ouvert → la raison est `budget`.
        let (c, t) = run_waves_pure(&gen, &json!({}), &p, 8, Some(&dry_b), Some(&open_d)).await;
        assert_eq!(t.stop_reason, StopReason::Budget, "plafond coût → nommé budget");
        assert!(c.is_empty());

        // ② plafond TEMPS atteint, budget ouvert → la raison est `deadline` (direction inverse,
        //    auparavant NON couverte — c'est le cœur de l'écart fermé).
        let (c, t) = run_waves_pure(&gen, &json!({}), &p, 8, Some(&open_b), Some(&closed_d)).await;
        assert_eq!(t.stop_reason, StopReason::Deadline, "plafond temps → nommé deadline");
        assert!(c.is_empty());

        // ③ les DEUX fermés au même instant → précédence déterministe : le budget, testé en
        //    premier dans la boucle, l'emporte (jamais d'ambiguïté wall-clock).
        let (_c, t) = run_waves_pure(&gen, &json!({}), &p, 8, Some(&dry_b), Some(&closed_d)).await;
        assert_eq!(t.stop_reason, StopReason::Budget, "double plafond → budget prime (testé avant)");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn verified_quota_with_caller_observe_stops_at_quota() {
        // T1 simulé : le vérificateur accepte les variants pairs ; quota 2 → la
        // première vague (k_min 4 : variants 0..3, 2 pairs) suffit.
        let gen: Arc<dyn Generator> = Arc::new(AllDistinct);
        let p = plan(4, 16, 4, StopRule::VerifiedQuota { quota: 2 });
        let observe = |cands: &[Value]| {
            cands
                .iter()
                .filter(|c| {
                    c.get("value")
                        .and_then(Value::as_str)
                        .and_then(|s| s.trim_start_matches('v').parse::<usize>().ok())
                        .map(|n| n % 2 == 0)
                        .unwrap_or(false)
                })
                .count() as f64
        };
        let (cands, trace) = run_waves(&gen, &json!({}), &p, 8, None, None, &observe).await;
        assert_eq!(cands.len(), 4);
        assert_eq!(trace.stop_reason, StopReason::Target);
        assert_eq!(trace.batches[0].observed, 2.0, "2 vérifiés comptés par l'appelant");
    }

    #[test]
    fn trace_json_has_the_documented_shape() {
        let p = plan(4, 24, 4, StopRule::Agreement { threshold: 0.9 });
        let t = SizingTrace {
            k_used: 8,
            batches: vec![BatchPoint { k: 4, observed: 0.667 }, BatchPoint { k: 8, observed: 0.929 }],
            stop_reason: StopReason::Target,
        };
        let j = t.to_json(&p);
        assert_eq!(j["k_used"], json!(8));
        assert_eq!(j["stop_reason"], json!("target"));
        assert_eq!(j["policy"]["k_max"], json!(24));
        assert_eq!(j["policy"]["stop"]["agreement"], json!(0.9));
        assert_eq!(j["batches"][1]["k"], json!(8));
        // +∞ (pas-encore-observable) sérialise en null, jamais en NaN invalide.
        let t2 = SizingTrace {
            k_used: 2,
            batches: vec![BatchPoint { k: 2, observed: f64::INFINITY }],
            stop_reason: StopReason::KMax,
        };
        assert_eq!(t2.to_json(&p)["batches"][0]["observed"], Value::Null);
    }
}
