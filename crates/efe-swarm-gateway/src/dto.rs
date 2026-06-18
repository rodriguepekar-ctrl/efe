// SPDX-License-Identifier: UNLICENSED
//! **DTO de fil** — le contrat HTTP **versionné**, découplé des types SDK (qui ne dérivent
//! pas `Deserialize`). On *désérialise* ici, puis on *mappe* vers `TopologySpec`/`Budget`/
//! `ClientMode`. Le SDK reste pur ; le contrat de fil évolue indépendamment.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

use efe_swarm_sdk::{
    Budget, ClientMode, ConvergenceMetric, FamilyKey, FamilyMode, KSpec, MemoryConfig, OnExceed,
    Pattern, PatternSpec, StopRule, SwarmRun, TopologySpec, DEFAULT_BATCH,
};

use crate::error::GatewayError;

fn default_true() -> bool {
    true
}

fn default_k() -> KDto {
    KDto::Fixed(3)
}
/// Gossip/Debate gardent un K nu (l'itératif est hors périmètre adaptatif — ADR-014 §4.4).
fn default_k_usize() -> usize {
    3
}

/// Corps de `POST /v1/swarm/run`. `deny_unknown_fields` : une typo de champ
/// (`"widht"`) est un 400, jamais un défaut silencieux qui change la facture
/// (revue ADR-014).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRequest {
    /// La tâche (JSON libre) passée à l'essaim.
    pub task: Value,
    /// La topologie à exécuter (enum tagué par `kind`).
    pub topology: TopologyDto,
    /// Le budget plafonné (défaut : illimité, politique `stop`).
    #[serde(default)]
    pub budget: BudgetDto,
    /// Le mode client (défaut : `keyless`).
    #[serde(default)]
    pub mode: ModeDto,
    /// **ADR-014 (CP-2)** : largeur d'exécution demandée (optionnelle). Plafonnée
    /// **côté serveur** (`EFE_GATEWAY_MAX_WIDTH`, défaut 32) — un client ne dimensionne
    /// pas la machine d'autrui. I-CP1 : ne change jamais le résultat, seulement les durées.
    #[serde(default)]
    pub width: Option<usize>,
    /// **ADR-018** : fenêtre temporelle demandée en millisecondes (optionnelle). Plafonnée
    /// **côté serveur** (`EFE_GATEWAY_MAX_DEADLINE_MS`, défaut 300 000 = 5 min ; plancher
    /// 50 ms) — un client ne monopolise pas le temps machine d'autrui, et ne pose pas non
    /// plus un deadline absurdement court qui refuserait tout. À la DIFFÉRENCE de `width`,
    /// le deadline PEUT tronquer la sortie (le temps est un plafond, pas un réglage) : un
    /// run deadliné reste re-prouvable (R0) mais non reproductible (R1) — cf. ADR-017/018.
    #[serde(default)]
    pub deadline_ms: Option<u64>,
    /// **ADR-019 (MEM-3)** : mémoire/apprentissage inter-runs (opt-in). Absent ⇒ mémoire OFF
    /// (chemin historique, I-MEM8). Présent ⇒ le run lit un instantané gelé (warm-start +
    /// trace MemoryRead) et replie s'il est propre (I-MEM3).
    #[serde(default)]
    pub memory: Option<MemoryDto>,
}

impl RunRequest {
    /// Démonte le DTO en types SDK prêts pour `swarm.run_traced(task, spec, budget)`.
    /// (Le deadline est porté à part par [`resolved_deadline`](Self::resolved_deadline) —
    /// il se pose sur le `Swarm` via `with_deadline`, pas dans la signature de run.)
    pub fn into_parts(self) -> Result<(Value, TopologySpec, Budget, ClientMode), GatewayError> {
        let spec = self.topology.into_spec()?;
        let budget = self.budget.into_budget();
        let mode = self.mode.into_mode();
        Ok((self.task, spec, budget, mode))
    }

    /// **ADR-018** : la fenêtre temporelle demandée, **clampée côté serveur** à
    /// `[MIN_DEADLINE_MS, EFE_GATEWAY_MAX_DEADLINE_MS]`. `None` ⇒ aucun deadline (run non
    /// borné en temps — chemin historique strictement inchangé). Lue AVANT `into_parts`
    /// (qui consomme la requête), exactement comme `width`.
    pub fn resolved_deadline(&self) -> Option<std::time::Duration> {
        self.deadline_ms.map(|ms| {
            let ms = ms.clamp(MIN_DEADLINE_MS, max_deadline_ms());
            std::time::Duration::from_millis(ms)
        })
    }
}

/// **ADR-018** : plancher du deadline serveur — sous 50 ms, même un run trivial n'a pas le
/// temps d'émettre une vague ; on relève au plancher plutôt que de garantir un run vide.
const MIN_DEADLINE_MS: u64 = 50;

/// **ADR-018** : plafond serveur du deadline run. Env `EFE_GATEWAY_MAX_DEADLINE_MS` ;
/// défaut 300 000 (5 min). Borne le temps machine qu'un client peut réserver.
fn max_deadline_ms() -> u64 {
    std::env::var("EFE_GATEWAY_MAX_DEADLINE_MS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(300_000)
        .max(MIN_DEADLINE_MS)
}

/// **ADR-019 (MEM-3)** : l'empreinte de famille de fil — `structural` (défaut souverain) ou
/// `exact`. La sémantique (embedding) reste hors périmètre HTTP (opt-in serveur futur).
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FamilyModeDto {
    Exact,
    #[default]
    Structural,
}

impl From<FamilyModeDto> for FamilyMode {
    fn from(m: FamilyModeDto) -> Self {
        match m {
            FamilyModeDto::Exact => FamilyMode::Exact,
            FamilyModeDto::Structural => FamilyMode::Structural,
        }
    }
}

/// **ADR-019 (MEM-3)** : la politique mémoire de fil. `deny_unknown_fields` : une typo de
/// gouvernance d'apprentissage est un 400, jamais un défaut silencieux. Tous les booléens
/// défaut à `true` (présent ⇒ on veut la mémoire) ; `pin_version` épingle une version passée
/// (reproductibilité, I-MEM7) ; `family` impose une famille hex (benchmark).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryDto {
    #[serde(default = "default_true")]
    pub recall: bool,
    #[serde(default = "default_true")]
    pub learn: bool,
    #[serde(default)]
    pub mode: FamilyModeDto,
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub pin_version: Option<u64>,
}

impl MemoryDto {
    /// Mappe vers la `MemoryConfig` du SDK. La famille hex (si fournie) devient un override.
    pub fn to_config(&self) -> MemoryConfig {
        MemoryConfig {
            recall: self.recall,
            learn: self.learn,
            mode: self.mode.into(),
            family_override: self.family.as_ref().map(|h| FamilyKey::from_hex(h.clone())),
            pin_version: self.pin_version,
        }
    }
}

/// **Taille d'essaim de fil** (ADR-014) — **untagged** : un nombre nu (`"k": 12`,
/// rétro-compatible au bit près) **ou** une politique adaptative
/// (`"k": { "min": 4, "max": 24, "agreement": 0.9 }`). Exactement UNE cible parmi
/// `agreement` / `se_target` / `verified_quota` (sinon 400), au domaine VALIDE
/// (agreement ∈ [0,1], se_target > 0, quota ≤ max — une cible structurellement
/// inatteignable serait une facture k_max silencieuse, revue ADR-014). `batch` ∈ spec
/// (défaut 4, constante — jamais la machine : architecture §2.3).
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum KDto {
    Fixed(usize),
    Adaptive(AdaptiveKDto),
}

/// L'objet adaptatif, en struct dédiée : `deny_unknown_fields` y est fiable
/// (`"se_targt"`/`"bacth"` → 400, jamais une politique silencieusement différente).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdaptiveKDto {
    pub min: usize,
    pub max: usize,
    #[serde(default = "default_batch")]
    pub batch: usize,
    #[serde(default)]
    pub agreement: Option<f64>,
    #[serde(default)]
    pub se_target: Option<f64>,
    #[serde(default)]
    pub verified_quota: Option<usize>,
}

fn default_batch() -> usize {
    DEFAULT_BATCH
}

impl KDto {
    fn into_kspec(self, topo: &'static str) -> Result<KSpec, GatewayError> {
        match self {
            KDto::Fixed(k) => Ok(KSpec::Fixed(k.max(1))),
            KDto::Adaptive(a) => {
                let AdaptiveKDto { min, max, batch, agreement, se_target, verified_quota } = a;
                let stop = match (agreement, se_target, verified_quota) {
                    (Some(t), None, None) => {
                        if !t.is_finite() || !(0.0..=1.0).contains(&t) {
                            return Err(GatewayError::BadRequest(format!(
                                "topology.{topo}.k : `agreement` doit être dans [0, 1] (l'accord est une fraction de paires), eu {t}"
                            )));
                        }
                        StopRule::Agreement { threshold: t }
                    }
                    (None, Some(s), None) => {
                        if !s.is_finite() || s <= 0.0 {
                            return Err(GatewayError::BadRequest(format!(
                                "topology.{topo}.k : `se_target` doit être > 0 (une erreur-type nulle est inatteignable), eu {s}"
                            )));
                        }
                        StopRule::StdError { target: s }
                    }
                    (None, None, Some(q)) => {
                        let q = q.max(1);
                        if q > max.max(min).max(1) {
                            return Err(GatewayError::BadRequest(format!(
                                "topology.{topo}.k : `verified_quota` ({q}) > k_max ({max}) — quota structurellement inatteignable (facture k_max garantie)"
                            )));
                        }
                        StopRule::VerifiedQuota { quota: q }
                    }
                    _ => {
                        return Err(GatewayError::BadRequest(format!(
                            "topology.{topo}.k adaptatif : exactement UNE cible parmi `agreement`, `se_target`, `verified_quota`"
                        )))
                    }
                };
                Ok(KSpec::Adaptive { k_min: min, k_max: max, batch, stop })
            }
        }
    }
}

/// Topologie de fil — enum **externe-tagué** par `kind` (snake_case).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TopologyDto {
    /// T1 — Generate-Verify (K générateurs + vérificateur déterministe).
    GenerateVerify {
        generator: String,
        verifier: String,
        #[serde(default = "default_k")]
        k: KDto,
    },
    /// T2 — Pipeline (étages nommés chaînés).
    Pipeline { stages: Vec<String> },
    /// T3 — Hiérarchique (décompose → résout → fusionne).
    Hierarchical { decomposer: String, solver: String, merger: String },
    /// T-Consensus (K générateurs + panel de juges → Vote/Borda/Condorcet).
    Consensus {
        generator: String,
        judges: Vec<String>,
        #[serde(default = "default_k")]
        k: KDto,
        #[serde(default)]
        pattern: PatternDto,
        #[serde(default)]
        escalate: bool,
    },
    /// T4 — Gossip (itératif, ADR-003) : K refiners × tours, arrêt à ε ou r_max.
    Gossip {
        refiner: String,
        #[serde(default = "default_k_usize")]
        k: usize,
        #[serde(default = "default_rounds")]
        max_rounds: usize,
        #[serde(default = "default_epsilon")]
        epsilon: f64,
        #[serde(default)]
        metric: MetricDto,
    },
    /// Debate (itératif, ADR-005) : consensus à dossier contradictoire.
    Debate {
        generator: String,
        judges: Vec<String>,
        #[serde(default = "default_pro")]
        pro: String,
        #[serde(default = "default_contra")]
        contra: String,
        #[serde(default = "default_k_usize")]
        k: usize,
        #[serde(default)]
        pattern: PatternDto,
        #[serde(default = "default_rounds")]
        rounds: usize,
        /// Escalade à trois étages (Vote → jury → débat).
        #[serde(default)]
        escalate: bool,
    },
    /// Auto — choisit selon la tâche + le mode.
    Auto,
}

fn default_rounds() -> usize {
    2
}
fn default_epsilon() -> f64 {
    0.05
}
fn default_pro() -> String {
    "Argue FOR the candidate: cite its strongest evidence.".into()
}
fn default_contra() -> String {
    "Argue AGAINST the candidate: cite its weakest point.".into()
}

/// Métrique de convergence de fil (snake_case).
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricDto {
    #[default]
    Variance,
    Agreement,
    Stability,
}
impl From<MetricDto> for ConvergenceMetric {
    fn from(m: MetricDto) -> Self {
        match m {
            MetricDto::Variance => ConvergenceMetric::Variance,
            MetricDto::Agreement => ConvergenceMetric::Agreement,
            MetricDto::Stability => ConvergenceMetric::Stability,
        }
    }
}

impl TopologyDto {
    fn into_spec(self) -> Result<TopologySpec, GatewayError> {
        Ok(match self {
            TopologyDto::GenerateVerify { generator, verifier, k } => {
                TopologySpec::GenerateVerify { generator, verifier, k: k.into_kspec("generate_verify")? }
            }
            TopologyDto::Pipeline { stages } => {
                if stages.is_empty() {
                    return Err(GatewayError::BadRequest("topology.pipeline : `stages` vide".into()));
                }
                TopologySpec::Pipeline { stages }
            }
            TopologyDto::Hierarchical { decomposer, solver, merger } => {
                TopologySpec::Hierarchical { decomposer, solver, merger }
            }
            TopologyDto::Consensus { generator, judges, k, pattern, escalate } => {
                if judges.is_empty() {
                    return Err(GatewayError::BadRequest("topology.consensus : `judges` vide".into()));
                }
                let k = k.into_kspec("consensus")?;
                // verified_quota exige un vérificateur : 400 NET au fil (plutôt qu'une
                // erreur SDK tardive) — le contrat HTTP reste explicite.
                if let KSpec::Adaptive { stop, .. } = &k {
                    if matches!(stop, StopRule::VerifiedQuota { .. }) {
                        return Err(GatewayError::BadRequest(
                            "topology.consensus.k : `verified_quota` requiert un vérificateur — utilisez kind: generate_verify".into(),
                        ));
                    }
                }
                // ADR-019 (read-back A) : `escalate` est subsumé par `pattern: adaptive` (le
                // sélecteur adaptatif décide lui-même l'escalade) → un combo ignoré silencieusement
                // est un 400 NET (revue), comme verified_quota au-dessus.
                if matches!(pattern, PatternDto::Adaptive) && escalate {
                    return Err(GatewayError::BadRequest(
                        "topology.consensus : `escalate` est ignoré par `pattern: adaptive` (le sélecteur adaptatif subsume l'escalade) — retirez `escalate` ou utilisez un pattern fixe".into(),
                    ));
                }
                TopologySpec::Consensus { generator, judges, k, pattern: pattern.into(), escalate }
            }
            TopologyDto::Gossip { refiner, k, max_rounds, epsilon, metric } => {
                TopologySpec::Gossip { refiner, k, max_rounds, epsilon, metric: metric.into() }
            }
            TopologyDto::Debate { generator, judges, pro, contra, k, pattern, rounds, escalate } => {
                if judges.is_empty() {
                    return Err(GatewayError::BadRequest("topology.debate : `judges` vide".into()));
                }
                // ADR-019 (read-back A) : le pattern adaptatif est CONSENSUS-only. Sur le débat, le
                // token `adaptive` retomberait silencieusement sur Borda (`From<PatternDto> for
                // Pattern`) → on REFUSE net (revue) plutôt que de trahir l'intention de l'appelant.
                if matches!(pattern, PatternDto::Adaptive) {
                    return Err(GatewayError::BadRequest(
                        "topology.debate : `pattern: adaptive` n'existe que pour le consensus — choisissez vote/borda/condorcet".into(),
                    ));
                }
                TopologySpec::Debate { generator, judges, pro, contra, k, pattern: pattern.into(), rounds, escalate }
            }
            TopologyDto::Auto => TopologySpec::Auto,
        })
    }
}

/// Pattern de consensus de fil. `adaptive` (ADR-019, read-back A) ⇒ pattern **adaptatif** biaisé
/// par la mémoire (consensus seulement). Token de fil : `vote`/`borda`/`condorcet`/`adaptive`.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatternDto {
    Vote,
    #[default]
    Borda,
    Condorcet,
    /// **ADR-019 (read-back A)** : « choisis pour moi » — sélecteur adaptatif biaisé par le prior.
    Adaptive,
}

impl From<PatternDto> for Pattern {
    /// Pour le **débat** (pas de pattern adaptatif) : `Adaptive` retombe gracieusement sur Borda.
    fn from(p: PatternDto) -> Self {
        match p {
            PatternDto::Vote => Pattern::Vote,
            PatternDto::Borda | PatternDto::Adaptive => Pattern::Borda,
            PatternDto::Condorcet => Pattern::Condorcet,
        }
    }
}

impl From<PatternDto> for PatternSpec {
    /// Pour le **consensus** : `adaptive` → sélecteur biaisé par la mémoire ; sinon pattern fixe.
    fn from(p: PatternDto) -> Self {
        match p {
            PatternDto::Vote => PatternSpec::Fixed(Pattern::Vote),
            PatternDto::Borda => PatternSpec::Fixed(Pattern::Borda),
            PatternDto::Condorcet => PatternSpec::Fixed(Pattern::Condorcet),
            PatternDto::Adaptive => PatternSpec::adaptive_default(),
        }
    }
}

/// Budget de fil. `deny_unknown_fields` : une typo de gouvernance est un 400.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BudgetDto {
    pub max_cents: u32,
    #[serde(default)]
    pub on_exceed: OnExceedDto,
    /// Prix par appel (cents). `None` ⇒ le prix du mode est utilisé.
    #[serde(default)]
    pub price_per_call_cents: Option<u32>,
}

impl Default for BudgetDto {
    fn default() -> Self {
        // Défaut sûr : illimité + arrêt propre (jamais de dépassement, jamais de refus).
        Self { max_cents: u32::MAX, on_exceed: OnExceedDto::Stop, price_per_call_cents: None }
    }
}

impl BudgetDto {
    fn into_budget(self) -> Budget {
        let mut b = Budget::cents(self.max_cents).with_policy(self.on_exceed.into());
        if let Some(p) = self.price_per_call_cents {
            b = b.with_price_per_call(p);
        }
        b
    }
}

/// Politique de dépassement de fil.
#[derive(Debug, Clone, Copy, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnExceedDto {
    Error,
    #[default]
    Stop,
    Degrade,
}

impl From<OnExceedDto> for OnExceed {
    fn from(o: OnExceedDto) -> Self {
        match o {
            OnExceedDto::Error => OnExceed::Error,
            OnExceedDto::Stop => OnExceed::Stop,
            OnExceedDto::Degrade => OnExceed::Degrade,
        }
    }
}

/// Mode client de fil — **externe-tagué** : `"keyless"` (chaîne) **ou**
/// `{"self_host":{"endpoint":..,"model":..}}` **ou** `{"network":{"provider":..}}`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeDto {
    #[default]
    Keyless,
    SelfHost { endpoint: String, model: String },
    Network { provider: String },
}

impl ModeDto {
    fn into_mode(self) -> ClientMode {
        match self {
            ModeDto::Keyless => ClientMode::Keyless,
            ModeDto::SelfHost { endpoint, model } => ClientMode::SelfHost { endpoint, model },
            ModeDto::Network { provider } => ClientMode::Network { provider },
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
//  Réponses de fil (Serialize)
// ─────────────────────────────────────────────────────────────────────────────

/// Réponse de `POST /v1/swarm/run`.
#[derive(Debug, Clone, Serialize)]
pub struct RunResponse {
    pub run_id: Uuid,
    /// Le `SwarmRun` (sortie + coût + budget + provenance{root,verified} + qualité).
    pub run: SwarmRun,
}

/// Réponse de `GET /v1/swarm/verify/:id` — **la re-preuve indépendante** (le *moat*).
#[derive(Debug, Clone, Serialize)]
pub struct VerifyResponse {
    pub run_id: Uuid,
    /// Re-preuve COMPLÈTE : chaîne d'essaim OK **ET** (zéro Mode C, ou chaîne N17 intacte
    /// ET pont entièrement cohérent). Un run keyless/Mode A reste `verified` trivialement.
    pub verified: bool,
    /// `provenance_root` reconstruit depuis les records + la clé tenant.
    pub root: Option<String>,
    pub chain_intact: bool,
    pub content_intact: bool,
    pub dag_consistent: bool,
    pub traced_pct: f64,
    pub records: usize,
    /// Premier enregistrement en faute (forensique), si altération.
    pub first_break: Option<Uuid>,
    /// **ADR-016 — la re-preuve forensique N17 (offline, ici en HTTP).**
    pub n17: N17VerifyView,
}

/// **ADR-016** : le verdict du pont N17 + de la chaîne forensique, dans la re-preuve.
#[derive(Debug, Clone, Serialize)]
pub struct N17VerifyView {
    /// Nombre de records forensiques persistés (0 = run Mode A, pont trivial).
    pub records: usize,
    /// La chaîne N17 se recompose-t-elle depuis les records persistés + la clé tenant ?
    pub chain_intact: bool,
    /// `n17_root` recalculé (hex).
    pub root: Option<String>,
    /// Le pont est-il entièrement cohérent (aucun Mode C non lié, aucune rupture) ?
    pub bridge_consistent: bool,
    /// Mode C liés et cohérents (résolvent vers un N17 persisté, même tenant/swarm).
    pub mode_c_consistent: usize,
    /// Mode C **sans** lien (lacune).
    pub mode_c_unlinked: usize,
    /// Liens rompus (pointeur pendant / incohérence d'isolation).
    pub mode_c_broken: usize,
    /// **ADR-016 — localisation forensique** : la `provenance_id` (UUID) du 1ᵉʳ record N17
    /// en faute quand la chaîne casse par mutation de contenu. `None` si la chaîne est
    /// intacte, ou si la rupture est une forge de feuille/racine (non localisable finement).
    /// C'est la valeur ajoutée de la persistance des feuilles : le client sait QUEL appel
    /// LLM a été altéré.
    pub first_break: Option<String>,
}

// ── ADR-017 — rejeu déterministe (/replay) ────────────────────────────────────

/// **ADR-017** — l'échelle de rejouabilité d'un record, projetée sur le fil (taxonomie
/// `DeterminismLevel` de la couche nano, ré-exprimée localement à la passerelle).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeterminismLevel {
    /// Mode A — agent Rust pur : reproductible **bit-à-bit** (la re-exécution DOIT matcher).
    FullDeterministic,
    /// Mode C avec graine épinglée : best-effort (le provider ne garantit pas le bit-à-bit).
    Seeded,
    /// Mode C rejouable par identité : `cache_hit` ou `replay_of` relisible (déterministe).
    LlmReplayable,
    /// Mode C éphémère : ni cache, ni graine, ni `replay_of` → **jamais** rejouable bit-à-bit.
    LlmEphemeral,
}

/// **ADR-017** — le verdict par-opération d'un rejeu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchOutcome {
    /// Sortie rejouée == sortie scellée (bit-à-bit).
    Match,
    /// Sortie rejouée ≠ scellée (divergence réelle).
    Mismatch,
    /// Non rejoué honnêtement (Mode C éphémère) — ni « match » ni « mismatch ».
    SkippedNonDeterministic,
    /// Non exercé par ce tier (R0 ne re-exécute pas).
    NotReexecuted,
}

/// **ADR-017** — une opération (record) dans le rapport de rejeu.
#[derive(Debug, Clone, Serialize)]
pub struct ReplayOpView {
    pub record_id: String,
    pub agent_id: String,
    /// `"mode_a"` ou `"mode_c"`.
    pub kind: String,
    pub level: DeterminismLevel,
    pub outcome: MatchOutcome,
}

/// **ADR-017** — la 1ʳᵉ divergence localisée (position en ordre canonique).
#[derive(Debug, Clone, Serialize)]
pub struct DivergenceView {
    /// Position dans l'ordre canonique (ADR-007).
    pub position: usize,
    pub record_id: String,
    pub agent_id: String,
    /// `"content_break"` (R0 : payload_hash ne recalcule pas) ou `"output_mismatch"` (R1).
    pub reason: String,
}

/// **ADR-017** — le résumé du tier R1 (re-exécution), absent en R0.
#[derive(Debug, Clone, Serialize)]
pub struct ExecutionReplayView {
    /// La re-exécution a-t-elle eu lieu ? (false si Mode C scopé-dehors, ou erreur).
    pub ran: bool,
    /// **Le verdict de reproductibilité de la COMPUTATION** (I-RP1) : la re-exécution
    /// re-dérive-t-elle les MÊMES sorties d'agents (`output_hash`) + le MÊME flux de données
    /// (`input_hash`, lectures/écritures de cellules) ? `true` pour un Mode A honnête. `null`
    /// si aucun verdict n'a pu être émis (Mode C scopé-dehors, ou `task_mismatch`).
    pub outputs_reproduced: Option<bool>,
    /// **Garde anti-confusion (§5.1)** : la task re-fournie ne correspond PAS aux entrées
    /// scellées du run (les `input_hash` des générateurs divergent). On NE produit alors PAS
    /// un faux `outputs_reproduced: false` (qui laisserait croire le run non reproductible) :
    /// `outputs_reproduced` reste `null`, et ce drapeau dit « erreur d'opérateur : mauvaise
    /// task », distinct d'une vraie non-reproduction.
    pub task_mismatch: bool,
    /// Nombre d'agents Mode A dont la sortie a été re-dérivée et comparée.
    pub mode_a_records: usize,
    pub mode_c_records: usize,
    /// Racine fraîche (hex) — **diffère forcément** de la scellée : la racine lie l'**identité
    /// par-run** (`swarm_id`, `record_id`, parents causaux). Purement informatif ; le verdict
    /// est `outputs_reproduced`, **jamais** une égalité de racine (re-provable par R0, pas
    /// reproductible par re-exécution — même nature que `n17_root`).
    pub fresh_root: Option<String>,
    /// Note honnête (ex. « mode_a : sorties re-dérivées à l'identique » / « mode_c hors-périmètre »).
    pub note: String,
}

/// **ADR-017** — le rapport de rejeu déterministe (`/v1/swarm/replay/:id`).
#[derive(Debug, Clone, Serialize)]
pub struct ReplayReport {
    pub run_id: Uuid,
    /// `"R0"` (re-sceller, GET) ou `"R1"` (re-exécuter, POST).
    pub tier: &'static str,
    /// La racine re-dérivée == la racine scellée ? (R0 : re-scellement ; R1 : re-exécution).
    pub root_reproduced: bool,
    /// Racine re-dérivée (hex) et racine scellée (hex) — pour comparaison transparente.
    pub reproduced_root: Option<String>,
    pub stored_root: Option<String>,
    /// Chaque `payload_hash` se recalcule (intégrité de contenu des records scellés).
    pub content_intact: bool,
    /// Nombre de records (essaim).
    pub records: usize,
    /// La 1ʳᵉ divergence localisée (si rejeu non reproduit).
    pub first_divergence: Option<DivergenceView>,
    /// Inventaire de rejouabilité : combien de records par niveau de déterminisme.
    pub determinism: ReplayDeterminismSummary,
    /// Le détail par-opération.
    pub per_op: Vec<ReplayOpView>,
    /// La forensique N17 re-vérifiée (PAS re-produite — I-RP7).
    pub n17: N17VerifyView,
    /// Le tier R1 (re-exécution), présent en POST uniquement.
    pub execution: Option<ExecutionReplayView>,
}

/// **ADR-017** — l'inventaire de rejouabilité d'un run (compte par niveau).
#[derive(Debug, Clone, Default, Serialize)]
pub struct ReplayDeterminismSummary {
    pub full_deterministic: usize,
    pub seeded: usize,
    pub llm_replayable: usize,
    pub llm_ephemeral: usize,
}

/// Vue d'un run (`GET /v1/swarm/runs/:id`) — **terminé** (dans le store) **ou en cours**
/// (un job live encore en exécution). Tagué par `status`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RunStatusView {
    /// Run terminé et persisté (records re-vérifiables via `verify/:id`).
    Finished {
        run_id: Uuid,
        output: Value,
        topology: String,
        root: Option<String>,
        records: usize,
        /// Date de création (epoch, secondes).
        created_at_unix: u64,
    },
    /// Job live encore en cours — suivez le flux SSE `events`.
    Running { run_id: Uuid, events: String },
}

/// Paramètres de requête de `POST /v1/swarm/run` (`?stream=true` → mode **job** live).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RunParams {
    /// `true` ⇒ exécution **asynchrone** observée (dashboard live) : réponse 202 immédiate.
    #[serde(default)]
    pub stream: bool,
}

/// Réponse 202 d'un **job** live (`POST /v1/swarm/run?stream=true`).
#[derive(Debug, Clone, Serialize)]
pub struct JobAccepted {
    pub run_id: Uuid,
    pub status: &'static str,
    /// URL du flux SSE des événements live.
    pub events: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deserializes_consensus_with_defaults() {
        let req: RunRequest = serde_json::from_value(json!({
            "task": { "text": "x" },
            "topology": { "kind": "consensus", "generator": "g", "judges": ["j1", "j2"] }
        }))
        .unwrap();
        let (_t, spec, budget, mode) = req.into_parts().unwrap();
        match spec {
            TopologySpec::Consensus { k, judges, pattern, .. } => {
                assert_eq!(k, KSpec::Fixed(3), "k par défaut (nombre nu → Fixed)");
                assert_eq!(judges.len(), 2);
                assert_eq!(pattern, PatternSpec::Fixed(Pattern::Borda), "pattern par défaut (fixe Borda)");
            }
            other => panic!("attendu consensus, eu {other:?}"),
        }
        assert_eq!(budget.max_cents, u32::MAX, "budget par défaut illimité");
        assert_eq!(mode, ClientMode::Keyless, "mode par défaut keyless");
    }

    #[test]
    fn deadline_ms_is_optional_and_clamped_server_side() {
        // ADR-018 : absent ⇒ None (run non borné en temps — chemin historique).
        let no_dl: RunRequest = serde_json::from_value(json!({
            "task": {}, "topology": { "kind": "auto" }
        }))
        .unwrap();
        assert_eq!(no_dl.resolved_deadline(), None, "pas de deadline par défaut");

        // Demande raisonnable ⇒ honorée telle quelle.
        let ok: RunRequest = serde_json::from_value(json!({
            "task": {}, "topology": { "kind": "auto" }, "deadline_ms": 2000
        }))
        .unwrap();
        assert_eq!(ok.resolved_deadline(), Some(std::time::Duration::from_millis(2000)));

        // Demande absurdement courte ⇒ relevée au plancher (jamais un run garanti vide).
        let tiny: RunRequest = serde_json::from_value(json!({
            "task": {}, "topology": { "kind": "auto" }, "deadline_ms": 1
        }))
        .unwrap();
        assert_eq!(tiny.resolved_deadline(), Some(std::time::Duration::from_millis(MIN_DEADLINE_MS)));

        // Demande gigantesque ⇒ plafonnée serveur (un client ne réserve pas le temps d'autrui).
        let huge: RunRequest = serde_json::from_value(json!({
            "task": {}, "topology": { "kind": "auto" }, "deadline_ms": 9_999_999_999u64
        }))
        .unwrap();
        assert_eq!(huge.resolved_deadline(), Some(std::time::Duration::from_millis(max_deadline_ms())));
    }

    #[test]
    fn dto_k_accepts_number_and_object() {
        // ADR-014 : rétro-compat au bit près — `"k": 12` reste un nombre nu…
        let req: RunRequest = serde_json::from_value(json!({
            "task": {},
            "topology": { "kind": "consensus", "generator": "g", "judges": ["j"], "k": 12 }
        }))
        .unwrap();
        match req.topology.into_spec().unwrap() {
            TopologySpec::Consensus { k, .. } => assert_eq!(k, KSpec::Fixed(12)),
            other => panic!("attendu consensus, eu {other:?}"),
        }
        // … et l'objet ouvre l'adaptatif (batch par défaut = constante de spec).
        let req: RunRequest = serde_json::from_value(json!({
            "task": {},
            "topology": { "kind": "consensus", "generator": "g", "judges": ["j"],
                          "k": { "min": 4, "max": 24, "agreement": 0.9 } }
        }))
        .unwrap();
        match req.topology.into_spec().unwrap() {
            TopologySpec::Consensus { k: KSpec::Adaptive { k_min, k_max, batch, stop }, .. } => {
                assert_eq!((k_min, k_max, batch), (4, 24, DEFAULT_BATCH));
                assert_eq!(stop, StopRule::Agreement { threshold: 0.9 });
            }
            other => panic!("attendu consensus adaptatif, eu {other:?}"),
        }
        // T1 + quota vérifié : le cas-phare.
        let req: RunRequest = serde_json::from_value(json!({
            "task": {},
            "topology": { "kind": "generate_verify", "generator": "g", "verifier": "v",
                          "k": { "min": 2, "max": 8, "batch": 2, "verified_quota": 1 } }
        }))
        .unwrap();
        match req.topology.into_spec().unwrap() {
            TopologySpec::GenerateVerify { k: KSpec::Adaptive { stop, .. }, .. } => {
                assert_eq!(stop, StopRule::VerifiedQuota { quota: 1 });
            }
            other => panic!("attendu T1 adaptatif, eu {other:?}"),
        }
    }

    #[test]
    fn dto_k_adaptive_rejects_bad_combos() {
        // zéro cible → 400.
        let req: RunRequest = serde_json::from_value(json!({
            "task": {}, "topology": { "kind": "consensus", "generator": "g", "judges": ["j"],
                                       "k": { "min": 2, "max": 8 } }
        }))
        .unwrap();
        assert!(matches!(req.topology.into_spec(), Err(GatewayError::BadRequest(_))));
        // deux cibles → 400.
        let req: RunRequest = serde_json::from_value(json!({
            "task": {}, "topology": { "kind": "consensus", "generator": "g", "judges": ["j"],
                                       "k": { "min": 2, "max": 8, "agreement": 0.9, "se_target": 0.5 } }
        }))
        .unwrap();
        assert!(matches!(req.topology.into_spec(), Err(GatewayError::BadRequest(_))));
        // verified_quota sur consensus (pas de vérificateur) → 400 NET au fil.
        let req: RunRequest = serde_json::from_value(json!({
            "task": {}, "topology": { "kind": "consensus", "generator": "g", "judges": ["j"],
                                       "k": { "min": 2, "max": 8, "verified_quota": 1 } }
        }))
        .unwrap();
        let err = req.topology.into_spec().unwrap_err();
        assert!(matches!(&err, GatewayError::BadRequest(m) if m.contains("generate_verify")), "{err}");
    }

    #[test]
    fn dto_k_adaptive_rejects_out_of_domain_targets() {
        // Revue ADR-014 : une cible structurellement inatteignable serait une facture
        // k_max silencieuse — domaine VALIDÉ au fil.
        for k in [
            json!({ "min": 2, "max": 8, "agreement": 1.7 }),
            json!({ "min": 2, "max": 8, "agreement": -0.1 }),
            json!({ "min": 2, "max": 8, "se_target": 0.0 }),
            json!({ "min": 2, "max": 8, "se_target": -1.0 }),
            json!({ "min": 2, "max": 4, "verified_quota": 10 }),
        ] {
            let req: RunRequest = serde_json::from_value(json!({
                "task": {}, "topology": { "kind": "consensus", "generator": "g", "judges": ["j"], "k": k }
            }))
            .unwrap();
            assert!(
                matches!(req.topology.into_spec(), Err(GatewayError::BadRequest(_))),
                "cible hors domaine acceptée : {k}"
            );
        }
        // les bornes du domaine restent VALIDES (1.0 et 0.9 passent).
        let req: RunRequest = serde_json::from_value(json!({
            "task": {}, "topology": { "kind": "consensus", "generator": "g", "judges": ["j"],
                                       "k": { "min": 2, "max": 8, "agreement": 1.0 } }
        }))
        .unwrap();
        assert!(req.topology.into_spec().is_ok(), "agreement = 1.0 (unanimité stricte) est légal");
    }

    #[test]
    fn adaptive_pattern_combinations_are_validated_at_the_wire() {
        // read-back A (revue) : `pattern: adaptive` est CONSENSUS-only et SUBSUME l'escalade —
        // les combos qui seraient silencieusement trahis sont des 400 NETS.
        let rejected = |topo: serde_json::Value| {
            let req: RunRequest = serde_json::from_value(json!({ "task": {}, "topology": topo })).unwrap();
            matches!(req.topology.into_spec(), Err(GatewayError::BadRequest(_)))
        };
        // ① adaptive sur DÉBAT → 400 (ne pas retomber silencieusement sur Borda).
        assert!(
            rejected(json!({ "kind": "debate", "generator": "g", "judges": ["j"], "pro": "p",
                             "contra": "c", "k": 3, "pattern": "adaptive", "rounds": 1, "escalate": false })),
            "adaptive sur débat → 400"
        );
        // ② adaptive + escalate sur CONSENSUS → 400 (`escalate` ignoré silencieusement sinon).
        assert!(
            rejected(json!({ "kind": "consensus", "generator": "g", "judges": ["j"], "k": 3,
                             "pattern": "adaptive", "escalate": true })),
            "adaptive + escalate → 400"
        );
        // ③ adaptive sur CONSENSUS sans escalate → LÉGAL (la surface même du read-back A).
        let ok: RunRequest = serde_json::from_value(json!({
            "task": {}, "topology": { "kind": "consensus", "generator": "g", "judges": ["j"], "k": 3, "pattern": "adaptive" }
        }))
        .unwrap();
        assert!(ok.topology.into_spec().is_ok(), "adaptive sur consensus (sans escalate) est légal");
    }

    #[test]
    fn unknown_fields_are_rejected_not_swallowed() {
        // Revue ADR-014 : les typos changeaient la sémantique et la facture en silence.
        // top niveau : "widht" (typo de width) → erreur de désérialisation.
        assert!(serde_json::from_value::<RunRequest>(json!({
            "task": {}, "topology": { "kind": "auto" }, "widht": 4
        }))
        .is_err());
        // objet adaptatif : "se_targt" / "bacth" → erreur (deny_unknown_fields sur la
        // struct dédiée — l'objet ne retombe PAS silencieusement sur une autre cible).
        assert!(serde_json::from_value::<RunRequest>(json!({
            "task": {}, "topology": { "kind": "consensus", "generator": "g", "judges": ["j"],
                                       "k": { "min": 2, "max": 4, "agreement": 0.0, "se_targt": 0.1 } }
        }))
        .is_err());
        assert!(serde_json::from_value::<RunRequest>(json!({
            "task": {}, "topology": { "kind": "consensus", "generator": "g", "judges": ["j"],
                                       "k": { "min": 2, "max": 4, "bacth": 2, "agreement": 0.9 } }
        }))
        .is_err());
        // budget : typo de gouvernance → erreur.
        assert!(serde_json::from_value::<RunRequest>(json!({
            "task": {}, "topology": { "kind": "auto" }, "budget": { "max_cents": 10, "on_exceeed": "stop" }
        }))
        .is_err());
    }

    #[test]
    fn width_field_parses_and_defaults_to_none() {
        let req: RunRequest = serde_json::from_value(json!({
            "task": {}, "topology": { "kind": "auto" }, "width": 4
        }))
        .unwrap();
        assert_eq!(req.width, Some(4));
        let req: RunRequest =
            serde_json::from_value(json!({ "task": {}, "topology": { "kind": "auto" } })).unwrap();
        assert_eq!(req.width, None, "absent ⇒ Auto côté SDK");
    }

    #[test]
    fn mode_keyless_is_a_bare_string() {
        let req: RunRequest = serde_json::from_value(json!({
            "task": {},
            "topology": { "kind": "auto" },
            "mode": "keyless"
        }))
        .unwrap();
        assert_eq!(req.into_parts().unwrap().3, ClientMode::Keyless);
    }

    #[test]
    fn mode_self_host_is_externally_tagged() {
        let req: RunRequest = serde_json::from_value(json!({
            "task": {},
            "topology": { "kind": "auto" },
            "mode": { "self_host": { "endpoint": "http://127.0.0.1:11434", "model": "gemma3:12b" } }
        }))
        .unwrap();
        match req.into_parts().unwrap().3 {
            ClientMode::SelfHost { endpoint, model } => {
                assert_eq!(endpoint, "http://127.0.0.1:11434");
                assert_eq!(model, "gemma3:12b");
            }
            other => panic!("attendu self_host, eu {other:?}"),
        }
    }

    #[test]
    fn empty_pipeline_is_rejected() {
        let req: RunRequest = serde_json::from_value(json!({
            "task": {},
            "topology": { "kind": "pipeline", "stages": [] }
        }))
        .unwrap();
        assert!(matches!(req.into_parts(), Err(GatewayError::BadRequest(_))));
    }

    #[test]
    fn budget_maps_policy_and_price() {
        let req: RunRequest = serde_json::from_value(json!({
            "task": {},
            "topology": { "kind": "auto" },
            "budget": { "max_cents": 50, "on_exceed": "degrade", "price_per_call_cents": 2 }
        }))
        .unwrap();
        let (_t, _s, budget, _m) = req.into_parts().unwrap();
        assert_eq!(budget.max_cents, 50);
        assert_eq!(budget.on_exceed, OnExceed::Degrade);
        assert_eq!(budget.price_per_call_cents, Some(2));
    }

    #[test]
    fn gossip_and_debate_wire_forms_parse() {
        // Gossip : défauts (k=3, r=2, ε=0.05, metric=variance).
        let req: RunRequest = serde_json::from_value(serde_json::json!({
            "task": { "initial": [1.0, 2.0] },
            "topology": { "kind": "gossip", "refiner": "mean_pull" }
        }))
        .unwrap();
        match req.topology.into_spec().unwrap() {
            TopologySpec::Gossip { refiner, k, max_rounds, epsilon, .. } => {
                assert_eq!(refiner, "mean_pull");
                assert_eq!((k, max_rounds), (3, 2));
                assert!((epsilon - 0.05).abs() < 1e-12);
            }
            other => panic!("attendu Gossip, eu {other:?}"),
        }
        // Debate : juges requis ; pro/contra ont des instructions par défaut.
        let req: RunRequest = serde_json::from_value(serde_json::json!({
            "task": { "text": "t" },
            "topology": { "kind": "debate", "generator": "g", "judges": ["j1"], "rounds": 1 }
        }))
        .unwrap();
        match req.topology.into_spec().unwrap() {
            TopologySpec::Debate { judges, rounds, pro, .. } => {
                assert_eq!(judges.len(), 1);
                assert_eq!(rounds, 1);
                assert!(pro.contains("FOR"), "instruction PRO par défaut");
            }
            other => panic!("attendu Debate, eu {other:?}"),
        }
        // Debate sans juge → 400.
        let req: RunRequest = serde_json::from_value(serde_json::json!({
            "task": {}, "topology": { "kind": "debate", "generator": "g", "judges": [] }
        }))
        .unwrap();
        assert!(req.topology.into_spec().is_err());
    }
}
