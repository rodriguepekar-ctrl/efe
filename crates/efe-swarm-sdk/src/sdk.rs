// SPDX-License-Identifier: UNLICENSED
//! `Swarm` — l'entrée unique : `swarm.run(task, topology, budget)`.

use std::sync::Arc;

use efe_blackboard::{SubstrateMetrics, WriteSink};
use efe_nano_sdk::{SwarmId, TenantId};
use efe_swarm_provenance::{
    verify_swarm, AgentKind, HmacKeySource, ProvenanceMetrics, ProvenanceRecorder, ProvenanceSink,
};
use efe_swarm_topology::{AgentFleet, QualitySignals, SwarmContext, TopologyOutput};

use crate::agents::Meter;
use crate::backend::{build_local_backend, EndpointPin, LocalBackend};
use crate::budget::{Budget, BudgetGuard, OnExceed};
use crate::deadline::DeadlineGuard;
use crate::n17::LlmProvenanceCollector;
use crate::client::{ClientMode, WidthSpec};
use crate::observer::{SwarmObserver, TeeSink};
use crate::resolver::Resolver;
use crate::memory::MemoryHandle;
use crate::result::{Accounting, ProvenanceSummary, RunOutcome, RunTermination, SwarmError, SwarmRun};
use crate::spec::TopologySpec;

/// L'essaim productisé. Construit via [`Swarm::builder`] ou [`Swarm::keyless`].
pub struct Swarm {
    fleet: Arc<AgentFleet>,
    mode: ClientMode,
    tenant: TenantId,
    key: Vec<u8>, // clé de provenance pré-dérivée (HKDF par tenant)
    provenance: bool,
    assumed_subtasks: usize,
    /// Métriques de provenance partagées (alimentées par chaque run) — pour `/metrics`.
    metrics: Option<Arc<ProvenanceMetrics>>,
    /// Métriques de substrat partagées (Blackboard/EventBus des topologies) — pour `/metrics`.
    substrate_metrics: Option<Arc<SubstrateMetrics>>,
    /// Observateur de cycle de vie (dashboard live, Inc.3) — `None` ⇒ comportement Inc.1/2.
    observer: Option<Arc<dyn SwarmObserver>>,
    /// **Garde-fou souveraineté (G2)** : autorise un backend self-host **non-local**
    /// (egress). `false` par défaut — un endpoint non-`127.0.0.1`/`localhost` est refusé.
    allow_remote_backend: bool,
    /// **RC-1 (G2-remote)** : épingles d'endpoints distants (host → empreinte cert/SPKI). Un
    /// backend distant autorisé mais **absent** de cette liste est refusé
    /// ([`SwarmError::RemoteEndpointNotPinned`]). Vide par défaut (souverain : aucun distant).
    remote_pins: Vec<EndpointPin>,
    /// **Health-check (S.a)** : sonde le backend self-host au démarrage (échec rapide si
    /// injoignable). `false` par défaut (compatibilité).
    require_healthy_backend: bool,
    /// **ADR-014 (CP-1)** : largeur d'exécution des vagues — `Auto` = cœurs (Mode A) /
    /// capacité déclarée du backend (Mode C). Politique d'exécution, jamais une
    /// sémantique (I-CP1) : la racine de provenance n'en dépend pas.
    width: WidthSpec,
    /// **ADR-018** : fenêtre temporelle (deadline run) optionnelle. `Some(d)` ⇒ l'essaim
    /// borne sa DURÉE d'exécution : sonde coopérative dans les vagues adaptatives (scelle
    /// le partiel sur frontière de vague) + backstop `tokio::timeout(d + grâce)` universel
    /// (scelle le partiel du recorder). `None` (défaut) ⇒ chemin historique inchangé.
    /// Comme la largeur, c'est une politique d'**exécution** ; mais à la différence d'elle,
    /// le deadline PEUT tronquer la sortie (le temps est un plafond, pas un simple réglage).
    deadline: Option<std::time::Duration>,
    /// **ADR-019 (MEM-2)** : la poignée de mémoire inter-runs (store + politique recall/learn).
    /// `Some` ⇒ le run lit un instantané gelé (warm-start + trace MemoryRead) et replie s'il
    /// est propre. `None` (défaut) ⇒ mémoire OFF, run byte-identique au chemin historique (I-MEM8).
    memory: Option<crate::memory::MemoryHandle>,
}

impl Swarm {
    pub fn builder() -> SwarmBuilder {
        SwarmBuilder::default()
    }

    /// **ADR-018** : pose (ou retire) la fenêtre temporelle du run. Pratique pour la
    /// passerelle, qui dérive un essaim par requête et y greffe le `deadline_ms` plafonné
    /// **côté serveur**. `None` ⇒ aucun deadline (chemin historique). Idempotent, bon marché.
    pub fn with_deadline(mut self, deadline: Option<std::time::Duration>) -> Self {
        self.deadline = deadline;
        self
    }

    /// **ADR-019 (MEM-2/3)** : branche (ou retire) la mémoire inter-runs sur un essaim déjà
    /// construit. Pratique pour la passerelle, qui dérive un essaim par requête et y greffe
    /// une poignée mémoire selon le DTO `memory`. `None` ⇒ mémoire OFF (chemin historique).
    pub fn with_memory(mut self, memory: Option<MemoryHandle>) -> Self {
        self.memory = memory;
        self
    }

    /// Essaim **keyless** par défaut (flotte standard, provenance activée). Le mode
    /// démo/dev en une ligne.
    pub fn keyless() -> Self {
        SwarmBuilder::default().build().expect("clé de test toujours dérivable")
    }

    pub fn mode(&self) -> &ClientMode {
        &self.mode
    }
    pub fn fleet(&self) -> &Arc<AgentFleet> {
        &self.fleet
    }

    /// **`swarm.run(task, topology, budget)`** — exécute un essaim, plafonné en coût et
    /// **auto-tracé** : renvoie la sortie, le coût exact, et un `provenance_root`
    /// re-vérifiable par un tiers. C'est le wrapper Inc.1 (il jette la `SealedProvenance`).
    pub async fn run(
        &self,
        task: serde_json::Value,
        topology: TopologySpec,
        budget: Budget,
    ) -> Result<SwarmRun, SwarmError> {
        self.run_traced(task, topology, budget).await.map(|o| o.run)
    }

    /// **`swarm.run_traced(...)`** — identique à [`Swarm::run`], mais **expose la
    /// provenance scellée** ([`RunOutcome::sealed`]) : les `AgentCallRecord` bruts que la
    /// passerelle HTTP persiste pour servir `GET /v1/swarm/verify/:id` (le *moat* en HTTP).
    /// `run()` n'est qu'un wrapper qui jette `sealed`.
    pub async fn run_traced(
        &self,
        task: serde_json::Value,
        topology: TopologySpec,
        budget: Budget,
    ) -> Result<RunOutcome, SwarmError> {
        // ClientMode::Network nécessite la passerelle serveur (clés) → pas d'in-process.
        if matches!(self.mode, ClientMode::Network { .. }) {
            return Err(SwarmError::NetworkRequiresGateway);
        }
        let price = budget.price_per_call_cents.unwrap_or_else(|| self.mode.price_per_call_cents());

        // 0. concrétise Auto (selon tâche + mode) + applique la politique de PRÉ-VOL.
        let spec = topology.concretize(&task, &self.mode);
        let spec = match budget.on_exceed {
            OnExceed::Degrade => spec.degraded_to_fit(budget.max_cents, price, self.assumed_subtasks),
            OnExceed::Error => {
                // Revue ADR-014 : arithmétique u64 SANS cast tronquant — un produit
                // k×rounds énorme doit être REFUSÉ net, jamais admis par débordement.
                let est = (spec.estimated_calls(self.assumed_subtasks) as u64).saturating_mul(price as u64);
                if est > budget.max_cents as u64 {
                    return Err(SwarmError::BudgetTooLow {
                        needed_cents: u32::try_from(est).unwrap_or(u32::MAX),
                        max_cents: budget.max_cents,
                    });
                }
                spec
            }
            OnExceed::Stop => spec,
        };

        // 0.b — ADR-019 (MEM-2) : RECALL. Si la mémoire est branchée, on calcule la FamilyKey
        // de la tâche et on lit un INSTANTANÉ GELÉ (lecture seule, tenant-scopé). La famille
        // sert au recall ET au learn ; le warm-start de `k_min` n'est extrait que sur le chemin
        // ADAPTATIF et sera CLAMPÉ aux bornes du run par le resolver (I-MEM4 — la mémoire
        // amorce, ne gouverne jamais). `None` (mémoire OFF) ⇒ chemin historique inchangé (I-MEM8).
        let mem = self.memory.clone();
        let tenant_str = self.tenant.to_string();
        let mut mem_family: Option<efe_swarm_memory::FamilyKey> = None;
        let mut mem_snapshot: Option<efe_swarm_memory::MemorySnapshot> = None;
        let mut mem_warm_k_min: Option<usize> = None;
        // MEM-2.b : exemplaires few-shot recall-és (aperçus des meilleurs gagnants passés), à
        // injecter dans les prompts Mode C. Bornés (≤ FEWSHOT_N). Vide ⇒ aucune injection (I-MEM8).
        let mut mem_exemplars: Vec<String> = Vec::new();
        // read-backs A/B : signaux du prior pour BIAISER le pattern + ROUTER par réputation. `None`
        // ⇒ aucune influence (sélecteur = base, agrégation uniforme — byte-identique, I-PAT2/I-REP2).
        let mut mem_pattern_bias: Option<efe_swarm_memory::PatternBias> = None;
        let mut mem_reputation: Option<efe_swarm_memory::Reputation> = None;
        // read-back B (R-b/R-c) : la version pinnée = clé de rotation de l'ordonnancement de profils.
        let mut mem_version: Option<u64> = None;
        if let Some(h) = &mem {
            let family = crate::memory::family_for(&task, &spec, &h.config);
            if h.config.recall {
                // I-MEM7 : une version épinglée lit l'instantané PASSÉ (repro) ; sinon le courant.
                let snap_read = match h.config.pin_version {
                    Some(v) => h.store.snapshot_at(&tenant_str, &family, v).await,
                    None => h.store.snapshot(&tenant_str, &family).await,
                };
                if let Ok(snap) = snap_read {
                    mem_warm_k_min = spec
                        .adaptive_bounds()
                        .and_then(|(lo, hi)| snap.cells.size.suggested_k_min(lo as u32, hi as u32))
                        .map(|k| k as usize);
                    mem_exemplars = crate::memory::recalled_exemplars(&snap);
                    // read-back A : signaux bruts du prior de pattern (le resolver clampe).
                    mem_pattern_bias = Some(snap.cells.pattern.bias());
                    // read-back B : la cellule de réputation (le resolver calcule les poids /k_max).
                    mem_reputation = Some(snap.cells.reputation.clone());
                    // read-back B (R-b/R-c) : la version (= nb de folds) — clé de rotation, lue AVANT
                    // le move du snapshot (u64 Copy) pour rester atomique avec la réputation ci-dessus.
                    mem_version = Some(snap.version);
                    mem_snapshot = Some(snap);
                }
            }
            mem_family = Some(family);
        }

        // 1. mode → backend d'agents + nature (Mode A flotte / Mode C self-host). En
        //    self-host, le garde-fou souveraineté (G2) REFUSE un endpoint non-local par
        //    défaut (egress interdit) ; le backend est enveloppé d'un circuit-breaker (G3).
        let (backend, kind): (Option<Arc<dyn LocalBackend>>, AgentKind) = match &self.mode {
            ClientMode::Keyless => (None, AgentKind::ModeA),
            ClientMode::SelfHost { endpoint, model } => {
                (Some(build_local_backend(endpoint, model, self.allow_remote_backend, &self.remote_pins)?), AgentKind::ModeC)
            }
            ClientMode::Network { .. } => unreachable!("écarté en tête de run"),
        };
        // Health-check (S.a) : sonde rapide du backend local — échec net plutôt que N timeouts.
        if self.require_healthy_backend {
            if let Some(b) = &backend {
                if !b.health().await {
                    return Err(SwarmError::BackendUnavailable { endpoint: b.descriptor().endpoint });
                }
            }
        }
        // Forensique : descripteur du backend local (pont N17) — capté AVANT le move.
        let backend_info = backend.as_ref().map(|b| {
            let d = b.descriptor();
            crate::result::BackendInfo { engine: d.kind.as_str().to_string(), model: d.model, endpoint: d.endpoint }
        });

        // 2. provenance + budget : recorder, sink (→ contexte), garde de budget.
        let swarm_id = SwarmId::new();
        let recorder = Arc::new(ProvenanceRecorder::new(swarm_id, self.tenant));
        let guard = Arc::new(BudgetGuard::new(budget.max_cents, price));
        // Pont N17 par-appel : un collecteur de `LlmProvenanceRecord` pour les runs Mode C
        // (self-host). Mode A (keyless) ⇒ aucun appel LLM, aucun collecteur.
        let n17 = matches!(kind, AgentKind::ModeC).then(LlmProvenanceCollector::new);
        let meter = Meter {
            rec: recorder.clone(),
            guard: guard.clone(),
            kind,
            observer: self.observer.clone(),
            n17: n17.clone(),
            swarm_id,
            tenant: self.tenant,
        };

        // Inc.3 : si un observateur fournit un exutoire, on TEE provenance + observateur
        // (la provenance reste byte-identique) ; sinon, provenance seule (Inc.1/2 inchangé).
        let prov_sink: Arc<dyn WriteSink> = Arc::new(ProvenanceSink::new(recorder.clone()));
        let sink: Arc<dyn WriteSink> = match self.observer.as_ref().and_then(|o| o.write_sink()) {
            Some(dash) => Arc::new(TeeSink(vec![prov_sink, dash])),
            None => prov_sink,
        };
        let mut ctx = SwarmContext::new(self.fleet.clone()).with_write_sink(sink);
        if let Some(sm) = &self.substrate_metrics {
            ctx = ctx.with_substrate_metrics(sm.clone());
        }
        if let Some(tap) = self.observer.as_ref().and_then(|o| o.event_tap()) {
            ctx = ctx.with_event_tap(tap); // Inc.3 : trafic inter-agent → dashboard
        }

        // 2.a — ADR-019 (I-MEM2) : trace la LECTURE mémoire dans la chaîne de provenance. Le
        // record MemoryRead (rôle Memory, lecture pure) ancre dans la `provenance_root` la
        // famille, la version et le `content_hash` de l'instantané lu : la racine DÉPEND donc
        // de ce qui a été recall-é (une sortie influencée par un recall non tracé serait
        // invalide). N'est émis que si un instantané a réellement été lu (recall ON).
        if let (Some(snap), Some(family)) = (&mem_snapshot, &mem_family) {
            recorder.capture(crate::memory::memory_read_capture(family, snap, mem_warm_k_min, mem_exemplars.len()));
        }

        // 2.b — ADR-014 (CP-1) : résout la LARGEUR d'exécution du run, par mode
        // (Mode A : cœurs mesurés ; Mode C : capacité déclarée du backend ; env
        // EFE_SWARM_WIDTH prioritaire). Descendue dans chaque topologie au resolve.
        let width = self.width.resolve(&self.mode, backend.as_ref().map(|b| b.suggested_width()));

        // 2.c — ADR-018 : pose la fenêtre temporelle du run (si demandée). Le guard démarre
        // SON chrono ici — la résolution de topologie et le pré-vol budget, déjà faits, ne
        // sont pas comptés : on borne le temps d'EXÉCUTION de l'essaim. La sonde coopérative
        // (I-AD2) en est dérivée par le resolver (`deadline_probe` → topologies adaptatives).
        let deadline = self.deadline.map(|d| Arc::new(DeadlineGuard::new(d)));

        // 3. assemble la topologie (agents flotte OU LLM, enveloppés) + exécute.
        let resolver = Resolver {
            fleet: &self.fleet,
            backend,
            meter,
            width,
            deadline: deadline.clone(),
            memory_k_min: mem_warm_k_min, // ADR-019 (I-MEM4) : warm-start borné dans resolve()
            // ADR-019 (MEM-2.b) : few-shot recall-é (déjà compté dans MemoryRead ci-dessus) —
            // injecté dans les prompts Mode C par `generator()`. Mode A : ignoré (aucun prompt).
            memory_exemplars: mem_exemplars,
            // read-back A/B : biais de pattern (clampé dans `biased_selector`) + réputation (poids
            // calculés dans `reputation_weights_for`). Le record MemoryRead a déjà tracé les signaux
            // bruts depuis l'instantané (l'épingle content_hash les lie déjà à la racine).
            memory_pattern_bias: mem_pattern_bias,
            memory_reputation: mem_reputation,
            // read-back B (R-b/R-c) : version pinnée = clé de rotation de l'ordonnancement (adaptatif).
            memory_version: mem_version,
        };
        let topo = spec.resolve(&resolver)?;
        let topo_name = topo.name().to_string();
        if let Some(o) = &self.observer {
            o.on_run_start(&topo_name, &self.mode.label(), budget.max_cents, self.assumed_subtasks);
        }
        // ADR-018 — exécution sous fenêtre temporelle. Sans deadline : `topo.run` nu, zéro
        // surcoût (chemin historique). Avec deadline : la sonde COOPÉRATIVE arrête déjà le
        // sizer adaptatif sur une frontière de vague (cas courant, point de coupe propre) ;
        // le BACKSTOP dur `tokio::timeout(deadline + grâce)` ne tranche que le non-coopératif
        // (K fixe, T2/T3) ou un hang. Point CLÉ du moat : le `ProvenanceRecorder` est PARTAGÉ
        // via le sink — tout record déjà écrit y SURVIT au drop du futur tranché. On scelle
        // donc le PARTIEL dans les deux cas (étape 4 ci-dessous, inchangée) ; seule la NETTETÉ
        // du point de coupe diffère. Un backstop n'est donc PAS une perte de preuve (I-AD3).
        let run_t0 = std::time::Instant::now();
        let (out, hard_aborted): (TopologyOutput, bool) = match &deadline {
            Some(g) => match tokio::time::timeout(g.hard_limit(), topo.run(&task, &ctx)).await {
                Ok(out) => (out, false),
                Err(_) => (
                    TopologyOutput {
                        // Sortie tronquée HONNÊTE : aucune valeur agrégée (le run n'a pas fini) ;
                        // la preuve, elle, est le partiel scellé (records réels). agents_used /
                        // cost_units sont raffinés depuis les records scellés (étape 5).
                        output: serde_json::json!({ "partial": true, "reason": "deadline_hard_abort" }),
                        agents_used: 0,
                        cost_units: 0.0,
                        elapsed_ms: run_t0.elapsed().as_millis() as u64,
                        quality: QualitySignals::default(),
                        // Backstop dur : run inachevé → aucun pattern décidé, aucune réponse
                        // acceptable (et `is_clean()` faux ⇒ jamais de learn de toute façon). ADR-019.
                        decided_pattern: None,
                        escalated: false,
                        produced_acceptable: false,
                        agent_contributions: Vec::new(), // run inachevé → aucune attribution
                    },
                    true,
                ),
            },
            None => (topo.run(&task, &ctx).await, false),
        };

        // 3. budget POST-VOL (politique Error : échec si épuisé en cours).
        if budget.on_exceed == OnExceed::Error && guard.exhausted() {
            return Err(SwarmError::BudgetExceeded { spent_cents: guard.spent_cents(), max_cents: guard.max_cents() });
        }

        // Pont N17 par-appel : les `LlmProvenanceRecord` forensiques collectés (Mode C).
        let llm_records = n17.as_ref().map(|c| c.records()).unwrap_or_default();
        // ADR-016 : scelle la forensique N17 en chaîne HMAC propre, avec la MÊME clé
        // tenant que la chaîne d'essaim — durable + tamper-evidente une fois persistée.
        // Vide en keyless/Mode A (aucun appel LLM ⇒ SealedN17 vide, no-op).
        let n17_sealed = efe_swarm_provenance::seal_n17(&llm_records, &self.key);

        // 4. scelle la provenance + re-prouve (verify_swarm) → provenance_root. La
        //    `SealedProvenance` est conservée (records bruts) pour la persistance HTTP ;
        //    les métriques partagées (si présentes) sont alimentées ici → `/metrics`.
        let (provenance, sealed, n17_summary) = if self.provenance {
            let sealed = recorder.seal_with_key(self.key.clone());
            let report = verify_swarm(&sealed.records, &self.key);
            if let Some(m) = &self.metrics {
                m.observe_sealed(&sealed);
                m.observe_verify(&report);
            }
            let summary = ProvenanceSummary {
                enabled: true,
                records: sealed.len(),
                traced_pct: report.traced_pct,
                verified: report.ok(),
                root: report.root_hex(),
            };
            if let Some(o) = &self.observer {
                o.on_seal(sealed.len(), report.traced_pct, summary.root.as_deref(), report.ok());
            }
            // N17 : vérifie le pont (chaque record Mode C lié à un LlmProvenanceRecord cohérent).
            let n17_summary = build_n17_summary(&sealed.records, &llm_records);
            (summary, Some(sealed), n17_summary)
        } else {
            (ProvenanceSummary::disabled(), None, None)
        };

        // ADR-018 — STATUT de terminaison (les trois plafonds, une priorité). Le backstop
        // dur prime (coupe la plus grossière) ; sinon la troncature coopérative par le temps ;
        // sinon l'arrêt budgétaire ; sinon nominal. `tripped()` lit l'AtomicBool partagé que
        // la sonde coopérative a (ou non) armé pendant le run — un run qui FRÔLE la deadline
        // sans jamais voir la sonde refuser reste honnêtement `Completed`.
        let budget_stopped = budget.on_exceed == OnExceed::Stop && guard.exhausted();
        let termination = if hard_aborted {
            RunTermination::DeadlineHardAbort
        } else if deadline.as_ref().is_some_and(|g| g.tripped()) {
            RunTermination::DeadlineExceeded
        } else if budget_stopped {
            RunTermination::BudgetStopped
        } else {
            RunTermination::Completed
        };

        let mut run = SwarmRun {
            output: out.output,
            topology: topo_name,
            accounting: Accounting {
                // Sur backstop dur, la sortie synthétique ne porte pas le décompte : on le
                // RAFFINE depuis les records réellement scellés (la seule vérité du partiel).
                agents_used: if hard_aborted { provenance.records } else { out.agents_used },
                cost_units: if hard_aborted { provenance.records as f64 } else { out.cost_units },
                elapsed_ms: out.elapsed_ms,
                width, // ADR-014 : la largeur résolue, rapportée par-run (jamais une sémantique)
            },
            budget: guard.snapshot(),
            provenance,
            quality: out.quality.into(),
            budget_stopped,
            termination,
            backend: backend_info,
            n17: n17_summary,
            memory: None, // ADR-019 (MEM-2) : rempli juste après (recall + learn).
            // ADR-019 (MEM-2.b) : le pattern décidé par la topologie (consensus), rendu
            // observable + replié dans le prior de pattern par `summary_from_run`.
            decided_pattern: out.decided_pattern.clone(),
        };

        // ── ADR-019 (MEM-2) : LEARN + résumé mémoire ────────────────────────────────────
        // La PORTE I-MEM3 : on ne replie QUE si le run est `is_clean()` (Completed ∧ verified
        // — la même condition qu'ADR-018). Un run tronqué (budget/deadline) ou non re-prouvé
        // est mis en quarantaine : il ne pollue pas la mémoire. Un échec de repli est
        // best-effort (la mémoire n'est pas le moat) : il n'invalide pas le run.
        if let Some(h) = &mem {
            let mut learned = false;
            if h.config.learn && run.is_clean() {
                if let Some(family) = &mem_family {
                    let update = efe_swarm_memory::MemoryUpdate {
                        run_id: swarm_id.as_uuid(),
                        family: family.clone(),
                        // MEM-2.b/c : `escalated` + `produced_acceptable` viennent du
                        // `TopologyOutput` (champs Copy, lisibles malgré le move partiel de `out`).
                        // `produced_acceptable` (calculé PAR la topologie) pilote la mémoire d'ÉCHEC
                        // (niveau famille) ; le SDK ne devine plus l'échec depuis la forme du gagnant.
                        // read-back B : `out.agent_contributions` (attribution par-variante) est
                        // replié en réputation. `out` n'est que partiellement déplacé (output/quality),
                        // ce champ y reste lisible.
                        summary: crate::memory::summary_from_run(&run, out.escalated, out.produced_acceptable, &out.agent_contributions, family),
                    };
                    if let Ok(outcome) = h.store.fold(&tenant_str, update).await {
                        learned = outcome.folded;
                    }
                }
            }
            // Le résumé mémoire — VISIBLE (comme la terminaison d'ADR-018) : ce qui a été lu
            // (recall + l'épingle content_hash) et écrit (learned).
            run.memory = Some(crate::result::MemorySummary {
                recalled: mem_snapshot.is_some(),
                family: mem_family.as_ref().map(|f| f.as_str().to_string()).unwrap_or_default(),
                version: mem_snapshot.as_ref().map(|s| s.version).unwrap_or(0),
                content_hash: mem_snapshot.as_ref().map(|s| s.content_hash_hex()).unwrap_or_default(),
                warm_start_k_min: mem_warm_k_min,
                learned,
            });
        }

        if let Some(o) = &self.observer {
            o.on_finish(&run);
        }
        Ok(RunOutcome { run, sealed, llm_records, n17_sealed })
    }
}

/// Vérifie le **pont N17** sur un run : chaque `AgentCallRecord` Mode C doit référencer un
/// `LlmProvenanceRecord` cohérent (même tenant, même swarm). Renvoie `None` pour un run
/// keyless (Mode A — aucun appel LLM, aucun pont attendu).
fn build_n17_summary(
    records: &[efe_swarm_provenance::AgentCallRecord],
    llm_records: &[efe_nano_sdk::LlmProvenanceRecord],
) -> Option<crate::result::N17Summary> {
    let mode_c = records.iter().filter(|r| matches!(r.agent_kind, AgentKind::ModeC)).count();
    if mode_c == 0 && llm_records.is_empty() {
        return None; // run keyless : pas de pont N17
    }
    let resolver = efe_swarm_provenance::MapResolver::from_records(llm_records.iter());
    let bridge = efe_swarm_provenance::verify_bridge(records, &resolver);
    Some(crate::result::N17Summary {
        calls: llm_records.len(),
        mode_c_records: bridge.consistent + bridge.unlinked + bridge.broken,
        consistent: bridge.consistent,
        unlinked: bridge.unlinked,
        broken: bridge.broken,
        all_consistent: bridge.all_mode_c_consistent(),
    })
}

/// Constructeur de [`Swarm`].
pub struct SwarmBuilder {
    mode: ClientMode,
    tenant: TenantId,
    key_source: HmacKeySource,
    provenance: bool,
    assumed_subtasks: usize,
    fleet: Option<Arc<AgentFleet>>,
    metrics: Option<Arc<ProvenanceMetrics>>,
    substrate_metrics: Option<Arc<SubstrateMetrics>>,
    observer: Option<Arc<dyn SwarmObserver>>,
    allow_remote_backend: bool,
    remote_pins: Vec<EndpointPin>,
    require_healthy_backend: bool,
    width: WidthSpec,
    deadline: Option<std::time::Duration>,
    memory: Option<MemoryHandle>,
}

impl Default for SwarmBuilder {
    fn default() -> Self {
        Self {
            mode: ClientMode::default(), // Keyless
            tenant: TenantId::system(),
            key_source: HmacKeySource::test_key(), // dev/keyless ; prod : EnvHkdf (server-side)
            provenance: true,
            assumed_subtasks: 8,
            fleet: None,
            metrics: None,
            substrate_metrics: None,
            observer: None,
            allow_remote_backend: false, // souverain par défaut : egress interdit
            remote_pins: Vec::new(), // RC-1 : aucune épingle ⇒ aucun distant autorisé
            require_healthy_backend: false,
            width: WidthSpec::Auto, // ADR-014 : cœurs (Mode A) / backend déclaré (Mode C)
            deadline: None, // ADR-018 : aucun deadline par défaut (chemin historique)
            memory: None, // ADR-019 : mémoire OFF par défaut (chemin historique, I-MEM8)
        }
    }
}

impl SwarmBuilder {
    pub fn mode(mut self, mode: ClientMode) -> Self {
        self.mode = mode;
        self
    }
    pub fn tenant(mut self, tenant: TenantId) -> Self {
        self.tenant = tenant;
        self
    }
    /// Source de clé de provenance (prod : `HmacKeySource::EnvHkdf` server-side).
    pub fn key_source(mut self, src: HmacKeySource) -> Self {
        self.key_source = src;
        self
    }
    pub fn provenance(mut self, on: bool) -> Self {
        self.provenance = on;
        self
    }
    pub fn assumed_subtasks(mut self, n: usize) -> Self {
        self.assumed_subtasks = n.max(1);
        self
    }
    /// Flotte custom (par défaut : `AgentFleet::standard()`).
    pub fn fleet(mut self, fleet: Arc<AgentFleet>) -> Self {
        self.fleet = Some(fleet);
        self
    }

    /// Registre de métriques de provenance **partagé** : chaque run l'alimente
    /// (`observe_sealed`/`observe_verify`). La passerelle HTTP l'expose en `/metrics`
    /// (agrégé sur tous les runs d'un tenant).
    pub fn with_metrics(mut self, metrics: Arc<ProvenanceMetrics>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    /// Registre de métriques de **substrat** partagé (Blackboard/EventBus des topologies
    /// T2/T-Consensus) : chaque run l'alimente → `swarm_blackboard_*`, exposé par la
    /// passerelle en `/metrics`.
    pub fn with_substrate_metrics(mut self, metrics: Arc<SubstrateMetrics>) -> Self {
        self.substrate_metrics = Some(metrics);
        self
    }

    /// **Phase 5 Inc.3** : branche un observateur de cycle de vie (dashboard live, journal…).
    /// Tee l'exutoire de provenance + tap d'événements + notifications appel/seal/finish.
    /// **Non-breaking** : sans observateur, le run est inchangé.
    pub fn with_observer(mut self, observer: Arc<dyn SwarmObserver>) -> Self {
        self.observer = Some(observer);
        self
    }

    /// **Garde-fou souveraineté (G2)** : autorise un backend self-host **non-local** (egress).
    /// Par défaut `false` — seul `127.0.0.1`/`localhost`/`::1` est accepté (souverain). À
    /// n'activer **qu'explicitement**, pour un backend LAN de confiance.
    pub fn allow_remote_backend(mut self, allow: bool) -> Self {
        self.allow_remote_backend = allow;
        self
    }

    /// **RC-1 (G2-remote)** : **épingle** un endpoint distant — l'opérateur déclare, hors-bande,
    /// le couple `(host, empreinte cert/SPKI attendue)`. Un backend distant autorisé
    /// (`allow_remote_backend(true)`) mais **non épinglé** est refusé
    /// ([`SwarmError::RemoteEndpointNotPinned`]) : *aucun distant en clair, aucun distant non
    /// pinné*. L'empreinte (pas le hostname — TOCTOU) est la racine de confiance du canal, et
    /// elle est **enregistrée** dans le record N17 (barreau R4). Cumulable (appels multiples).
    pub fn pin_remote_endpoint(mut self, host: impl Into<String>, spki_fingerprint: impl Into<String>) -> Self {
        self.remote_pins.push(EndpointPin::new(host, spki_fingerprint));
        self
    }

    /// **Health-check (S.a)** : sonde le backend self-host au démarrage du run ; renvoie
    /// `SwarmError::BackendUnavailable` s'il est injoignable (échec rapide). `false` par défaut.
    pub fn require_healthy_backend(mut self, require: bool) -> Self {
        self.require_healthy_backend = require;
        self
    }

    /// **ADR-014 (CP-1)** : la **largeur d'exécution** des vagues. `Auto` (défaut) =
    /// `min(vague, P_eff)` — cœurs mesurés en Mode A, capacité déclarée du backend en
    /// Mode C. `Fixed(w)` impose. Env `EFE_SWARM_WIDTH` prioritaire sur tout (l'opérateur
    /// a le dernier mot). **I-CP1** : ce réglage ne change ni cellules, ni records, ni
    /// racine — seules les durées.
    pub fn width(mut self, w: WidthSpec) -> Self {
        self.width = w;
        self
    }

    /// **ADR-018** : pose la fenêtre temporelle (deadline run). `Some(d)` borne la durée
    /// d'exécution (coopératif + backstop) ; `None` (défaut) = aucun deadline.
    pub fn deadline(mut self, deadline: Option<std::time::Duration>) -> Self {
        self.deadline = deadline;
        self
    }

    /// **ADR-019 (MEM-2)** : branche la mémoire inter-runs (store + politique recall/learn).
    /// Sans cet appel, la mémoire est OFF (chemin historique, I-MEM8).
    pub fn memory(mut self, handle: MemoryHandle) -> Self {
        self.memory = Some(handle);
        self
    }

    /// Construit le `Swarm` (dérive la clé de provenance par tenant).
    pub fn build(self) -> Result<Swarm, SwarmError> {
        let key = self.key_source.derive(self.tenant).map_err(|e| SwarmError::KeyError(e.to_string()))?;
        Ok(Swarm {
            fleet: self.fleet.unwrap_or_else(|| Arc::new(AgentFleet::standard())),
            mode: self.mode,
            tenant: self.tenant,
            key,
            provenance: self.provenance,
            assumed_subtasks: self.assumed_subtasks,
            metrics: self.metrics,
            substrate_metrics: self.substrate_metrics,
            observer: self.observer,
            allow_remote_backend: self.allow_remote_backend,
            remote_pins: self.remote_pins,
            require_healthy_backend: self.require_healthy_backend,
            width: self.width,
            deadline: self.deadline,
            memory: self.memory,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ClientMode;
    use crate::spec::PatternSpec;
    use efe_swarm_topology::Pattern;
    use serde_json::json;

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn keyless_consensus_runs_and_is_verifiable() {
        let swarm = Swarm::keyless();
        let run = swarm
            .run(
                json!({ "text": "hello", "expected": "hello" }),
                TopologySpec::consensus("echo_generator", &["consistency", "length_band", "j_non_empty"], 5),
                Budget::cents(1000),
            )
            .await
            .unwrap();
        assert_eq!(run.topology, "T-consensus");
        assert!(run.backend.is_none(), "keyless Mode A : aucun backend local");
        assert!(run.n17.is_none(), "keyless Mode A : aucun appel LLM → aucun pont N17");
        // provenance auto : chaque run renvoie un root re-vérifiable.
        assert!(run.provenance.enabled);
        assert!(run.provenance.verified, "verify_swarm a re-prouvé le run");
        assert_eq!(run.provenance.root.as_ref().unwrap().len(), 64);
        assert!(run.budget.spent_cents <= run.budget.max_cost_cents);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn t1_generate_verify_runs() {
        let swarm = Swarm::keyless();
        let run = swarm
            .run(json!({ "text": "abc" }), TopologySpec::GenerateVerify { generator: "echo_generator".into(), verifier: "non_empty_text".into(), k: 3.into() }, Budget::cents(100))
            .await
            .unwrap();
        assert_eq!(run.topology, "T1:generate-verify");
        assert!(run.provenance.verified);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn t2_pipeline_runs() {
        let swarm = Swarm::keyless();
        let run = swarm
            .run(json!({ "text": "  Hello   World  " }), TopologySpec::Pipeline { stages: vec!["normalize_whitespace".into(), "uppercase".into()] }, Budget::cents(100))
            .await
            .unwrap();
        assert_eq!(run.topology, "T2:pipeline");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn t3_hierarchical_runs_with_stage_solver() {
        let swarm = Swarm::keyless();
        let run = swarm
            .run(json!({ "text": "alpha beta gamma" }), TopologySpec::Hierarchical { decomposer: "word_split".into(), solver: "uppercase".into(), merger: "join_lines".into() }, Budget::cents(1000))
            .await
            .unwrap();
        assert_eq!(run.topology, "T3:hierarchical");
        assert!(run.provenance.verified);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn auto_picks_a_topology_and_runs() {
        let swarm = Swarm::keyless();
        let run = swarm.run(json!({ "text": "x", "expected": "x" }), TopologySpec::Auto, Budget::cents(100)).await.unwrap();
        assert_eq!(run.topology, "T1:generate-verify"); // verifiable → T1
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn budget_error_policy_rejects_when_too_low() {
        let swarm = Swarm::keyless();
        // Keyless = 1¢/appel ; consensus k=7 + 3 juges = 10 appels = 10¢ > plafond 3¢.
        let err = swarm
            .run(
                json!({ "text": "x" }),
                TopologySpec::Consensus { generator: "echo_generator".into(), judges: vec!["consistency".into(), "length_band".into(), "j_non_empty".into()], k: 7.into(), pattern: PatternSpec::Fixed(Pattern::Borda), escalate: false },
                Budget::cents(3).with_policy(OnExceed::Error),
            )
            .await;
        assert!(matches!(err, Err(SwarmError::BudgetTooLow { .. })));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn budget_degrade_policy_shrinks_to_fit() {
        let swarm = Swarm::keyless();
        // plafond 3¢ @ 1¢/appel → 3 appels max ; consensus dégradé tient dans le budget.
        let run = swarm
            .run(
                json!({ "text": "x", "expected": "x" }),
                TopologySpec::Consensus { generator: "echo_generator".into(), judges: vec!["consistency".into(), "length_band".into(), "j_non_empty".into()], k: 7.into(), pattern: PatternSpec::Fixed(Pattern::Borda), escalate: false },
                Budget::cents(3).with_policy(OnExceed::Degrade),
            )
            .await
            .unwrap();
        assert!(run.budget.spent_cents <= 3, "jamais au-delà du plafond (I13)");
        assert!(!run.budget_stopped, "dégradé pour tenir, pas arrêté");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn budget_stop_policy_caps_spend() {
        let swarm = Swarm::keyless();
        let run = swarm
            .run(
                json!({ "text": "x" }),
                TopologySpec::Consensus { generator: "echo_generator".into(), judges: vec!["consistency".into(), "length_band".into(), "j_non_empty".into()], k: 7.into(), pattern: PatternSpec::Fixed(Pattern::Borda), escalate: false },
                Budget::cents(4).with_policy(OnExceed::Stop),
            )
            .await
            .unwrap();
        assert!(run.budget.spent_cents <= 4, "plafond JAMAIS dépassé (I13)");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn unknown_agent_errors() {
        let swarm = Swarm::keyless();
        let err = swarm.run(json!({}), TopologySpec::GenerateVerify { generator: "does_not_exist".into(), verifier: "non_empty_text".into(), k: 1.into() }, Budget::unlimited()).await;
        assert!(matches!(err, Err(SwarmError::UnknownAgent { role: "generator", .. })));
    }

    #[test]
    fn self_host_mode_builds_keyless() {
        // build seul (pas de run : SelfHost ferait de VRAIS appels Ollama — cf. l'exemple
        // selfhost_ollama). On vérifie : keyless + prix local nul.
        let swarm = Swarm::builder()
            .mode(ClientMode::SelfHost { endpoint: "http://127.0.0.1:11434".into(), model: "gemma3:12b".into() })
            .build()
            .unwrap();
        assert!(swarm.mode().is_keyless());
        assert_eq!(swarm.mode().price_per_call_cents(), 0, "self-host local = gratuit");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn self_host_remote_endpoint_is_denied_by_default() {
        // Garde-fou souveraineté (G2) : un backend self-host non-local est REFUSÉ avant tout
        // appel réseau (le garde-fou mord avant la résolution des agents) → egress=0.
        let swarm = Swarm::builder()
            .mode(ClientMode::SelfHost { endpoint: "http://example.com:11434".into(), model: "gemma3:12b".into() })
            .build()
            .unwrap();
        let err = swarm.run(json!({ "text": "x", "expected": "x" }), TopologySpec::Auto, Budget::unlimited()).await;
        assert!(matches!(err, Err(SwarmError::RemoteBackendDenied { .. })), "egress refusé par défaut (souverain)");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn require_healthy_backend_errors_on_dead_local_endpoint() {
        // Health-check (S.a) : endpoint LOCAL (passe le garde-fou egress) mais MORT (port 1,
        // connexion refusée instantanément) → BackendUnavailable, sans réseau externe.
        let swarm = Swarm::builder()
            .mode(ClientMode::SelfHost { endpoint: "http://127.0.0.1:1".into(), model: "m".into() })
            .require_healthy_backend(true)
            .build()
            .unwrap();
        let err = swarm.run(json!({ "text": "x", "expected": "x" }), TopologySpec::Auto, Budget::unlimited()).await;
        assert!(matches!(err, Err(SwarmError::BackendUnavailable { .. })), "health-check : échec rapide si injoignable");
    }

    #[test]
    fn allow_remote_backend_is_opt_in_and_off_by_default() {
        // build seul (pas de run) : on vérifie que le flag existe et est opt-in.
        let _ = Swarm::builder()
            .mode(ClientMode::SelfHost { endpoint: "http://10.0.0.5:11434".into(), model: "m".into() })
            .allow_remote_backend(true)
            .build()
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn remote_backend_without_pin_is_refused_even_with_egress_allowed() {
        // G2-remote (RC-1) : egress AUTORISÉ mais endpoint NON ÉPINGLÉ → refusé avant tout appel
        // réseau. *Aucun distant en clair, aucun distant non pinné* — le gate mord dans `run`.
        let swarm = Swarm::builder()
            .mode(ClientMode::SelfHost { endpoint: "http://gpu.internal:11434".into(), model: "gemma3:12b".into() })
            .allow_remote_backend(true) // egress autorisé… mais aucune épingle déclarée.
            .build()
            .unwrap();
        let err = swarm.run(json!({ "text": "x", "expected": "x" }), TopologySpec::Auto, Budget::unlimited()).await;
        assert!(
            matches!(err, Err(SwarmError::RemoteEndpointNotPinned { .. })),
            "un distant autorisé mais non épinglé doit être refusé"
        );
    }

    #[test]
    fn pin_remote_endpoint_is_accepted_by_the_builder() {
        // l'API publique d'épinglage chaîne et construit (le gate s'applique au run).
        let _ = Swarm::builder()
            .mode(ClientMode::SelfHost { endpoint: "http://gpu.internal:11434".into(), model: "m".into() })
            .allow_remote_backend(true)
            .pin_remote_endpoint("gpu.internal", "sha256/SIMULATED-REMOTE-PIN")
            .build()
            .unwrap();
    }

    /// **Pont N17 par-appel RÉEL** (Ollama local). Ignoré par défaut (CI sans modèle).
    /// Lancer : `cargo test -p efe-swarm-sdk --lib -- --ignored n17_real`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[ignore = "nécessite un Ollama local (ollama serve + gemma3:12b)"]
    async fn n17_per_call_bridge_links_real_self_host_calls() {
        let swarm = Swarm::builder()
            .mode(ClientMode::SelfHost { endpoint: "http://127.0.0.1:11434".into(), model: "gemma3:12b".into() })
            .build()
            .unwrap();
        let outcome = swarm
            .run_traced(
                json!({ "text": "The court ruled the clause unenforceable.", "expected": "legal" }),
                TopologySpec::Consensus {
                    generator: "Classify the topic in one word from {technology,finance,health,legal}.".into(),
                    judges: vec!["Judge how well the label fits.".into()],
                    k: 3.into(),
                    pattern: PatternSpec::Fixed(Pattern::Borda),
                    escalate: false,
                },
                Budget::cents(1000),
            )
            .await
            .unwrap();

        // chaque appel LLM a un record forensique ; le pont est cohérent.
        assert!(!outcome.llm_records.is_empty(), "des LlmProvenanceRecord par-appel");
        let n17 = outcome.run.n17.as_ref().expect("résumé N17 présent (Mode C)");
        assert!(n17.calls >= 4, "≥ 3 générateurs + ≥ 1 juge → ≥ 4 appels LLM");
        assert!(n17.all_consistent, "tous les Mode C liés à un N17 cohérent : {n17:?}");
        assert_eq!(n17.unlinked, 0);
        assert_eq!(n17.broken, 0);
        // le forensique porte le vrai modèle.
        assert!(outcome.llm_records.iter().all(|r| r.model_identity.model_id == "gemma3:12b"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn network_mode_requires_gateway() {
        let swarm = Swarm::builder().mode(ClientMode::Network { provider: "anthropic".into() }).build().unwrap();
        let err = swarm.run(json!({ "text": "x" }), TopologySpec::Auto, Budget::unlimited()).await;
        assert!(matches!(err, Err(SwarmError::NetworkRequiresGateway)), "Network → passerelle serveur (clés)");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn run_traced_exposes_sealed_records_matching_summary() {
        // Inc.2 : run_traced expose les AgentCallRecord bruts (la preuve persistable).
        let swarm = Swarm::keyless();
        let outcome = swarm
            .run_traced(
                json!({ "text": "hello", "expected": "hello" }),
                TopologySpec::consensus("echo_generator", &["consistency", "length_band", "j_non_empty"], 5),
                Budget::cents(1000),
            )
            .await
            .unwrap();
        // les records existent et leur compte == le résumé.
        let sealed = outcome.sealed.as_ref().expect("provenance activée → records scellés");
        assert!(!sealed.records.is_empty());
        assert_eq!(sealed.len(), outcome.run.provenance.records);
        assert_eq!(outcome.records().len(), sealed.len());
        // un tiers re-prouve depuis les SEULS records + une clé tenant.
        let key = HmacKeySource::test_key().derive(TenantId::system()).unwrap();
        let report = verify_swarm(outcome.records(), &key);
        assert!(report.ok(), "records bruts re-prouvés indépendamment");
        assert_eq!(report.root_hex(), outcome.run.provenance.root);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn with_metrics_accumulates_across_runs() {
        // Inc.2 : un registre partagé agrège la provenance de TOUS les runs (→ /metrics).
        let metrics = ProvenanceMetrics::new();
        let swarm = Swarm::builder().with_metrics(metrics.clone()).build().unwrap();
        for _ in 0..3 {
            swarm
                .run(json!({ "text": "x", "expected": "x" }), TopologySpec::Auto, Budget::cents(100))
                .await
                .unwrap();
        }
        let snap = metrics.snapshot();
        assert!(snap.records_total >= 3, "records agrégés sur les 3 runs");
        assert_eq!(snap.chain_breaks_total, 0, "runs sains → aucune rupture");
        assert!(metrics.render_prometheus().contains("efe_swarm_prov_records_total"));
    }

    // ════════════════════════════════════════════════════════════════════════════════
    //  ADR-018 — DEADLINE RUN (gardiens bout-en-bout du plafond TEMPS)
    // ════════════════════════════════════════════════════════════════════════════════

    /// Générateur LENT à valeurs DISTINCTES : dort `delay` puis renvoie un candidat unique
    /// par variant. L'accord reste ≈ 0 (jamais Target) → la boucle de vagues court jusqu'au
    /// deadline : l'outil pour observer la troncature temporelle de bout en bout.
    struct SlowDistinctGen {
        delay: std::time::Duration,
    }
    #[async_trait::async_trait]
    impl efe_swarm_topology::Generator for SlowDistinctGen {
        async fn generate(&self, _task: &serde_json::Value, variant: usize) -> serde_json::Value {
            tokio::time::sleep(self.delay).await;
            json!({ "text": format!("candidat-distinct-{variant}") })
        }
    }

    /// Essaim keyless dont le générateur `slow_gen` dort `delay_ms` par appel, avec un
    /// deadline run optionnel posé sur l'essaim (le reste = flotte standard, juges réels).
    fn swarm_with_slow_gen(delay_ms: u64, deadline: Option<std::time::Duration>) -> Swarm {
        let mut fleet = AgentFleet::standard();
        fleet.register_generator(
            "slow_gen",
            Arc::new(SlowDistinctGen { delay: std::time::Duration::from_millis(delay_ms) }),
        );
        Swarm::builder().fleet(Arc::new(fleet)).deadline(deadline).build().unwrap()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn run_without_deadline_terminates_completed() {
        // Régression : sans deadline, statut `Completed`, run `clean` — chemin historique
        // strictement préservé (aucun timeout enveloppé, aucune sonde branchée).
        let swarm = Swarm::keyless();
        let run = swarm
            .run(
                json!({ "text": "hello", "expected": "hello" }),
                TopologySpec::consensus("echo_generator", &["consistency", "j_non_empty"], 3),
                Budget::cents(1000),
            )
            .await
            .unwrap();
        assert_eq!(run.termination, RunTermination::Completed);
        assert!(run.is_clean());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn deadline_truncates_adaptive_run_and_seals_a_verifiable_partial() {
        // I-AD2/I-AD3 bout-en-bout : un consensus ADAPTATIF à générateur lent (80 ms/appel)
        // sous deadline serré (30 ms) est TRONQUÉ par le temps. 1ʳᵉ vague émise (guard frais),
        // sonde refuse la suivante → arrêt ; le PARTIEL scellé se re-prouve (R0). Un run
        // deadliné n'est PAS `clean`, mais reste re-prouvable. On assure `is_deadline()`
        // plutôt que la variante exacte (sur une machine très chargée, le backstop dur peut
        // devancer le coopératif — les DEUX scellent le partiel ; la variante coopérative
        // PRÉCISE est prouvée, déterministe, dans `sizing.rs`).
        let swarm = swarm_with_slow_gen(80, Some(std::time::Duration::from_millis(30)));
        let run = swarm
            .run(
                json!({ "text": "x" }),
                TopologySpec::consensus_adaptive("slow_gen", &["j_non_empty", "consistency"], 2, 24, 0.999),
                Budget::cents(1_000_000),
            )
            .await
            .unwrap();
        assert!(run.termination.is_deadline(), "tronqué par le TEMPS : {:?}", run.termination);
        assert!(run.provenance.verified, "le partiel scellé se re-prouve (R0) — I-AD3");
        assert!(!run.is_clean(), "deadliné = partiel (pas clean), mais re-prouvable");
        assert!(run.accounting.agents_used < 24, "partiel BORNÉ sous k_max (la deadline a mordu)");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn hard_backstop_aborts_a_noncooperative_run_yet_still_seals() {
        // I-AD3 backstop dur : un consensus À K FIXE (aucune sonde coopérative) à générateur
        // très lent (500 ms/appel) sous un deadline minuscule (20 ms) dépasse `deadline +
        // grâce` (~270 ms) → `tokio::timeout` tranche net. CRUCIAL : le run renvoie quand
        // même Ok, scelle le partiel présent dans le recorder (le moat SURVIT au drop du
        // futur tranché), et le statut est `DeadlineHardAbort`. Robuste quel que soit le
        // nombre de cœurs (500 ms ≫ 270 ms, en parallèle comme en série).
        let swarm = swarm_with_slow_gen(500, Some(std::time::Duration::from_millis(20)));
        let run = swarm
            .run(
                json!({ "text": "x" }),
                TopologySpec::consensus("slow_gen", &["j_non_empty"], 4),
                Budget::cents(1_000_000),
            )
            .await
            .unwrap();
        assert_eq!(run.termination, RunTermination::DeadlineHardAbort, "backstop dur tranché");
        assert!(!run.is_clean(), "tronqué net : pas propre");
        assert!(run.provenance.enabled, "provenance toujours scellée (le moat survit au drop)");
        assert_eq!(run.output["reason"], json!("deadline_hard_abort"), "sortie tronquée honnête");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn run_termination_names_the_winning_ceiling() {
        // I-AD5 (composition) bout-en-bout : le STATUT du run NOMME le plafond atteint. La
        // précédence run-level (sdk.rs) est hard_abort > deadline.tripped > budget_stopped >
        // Completed — déterministe, jamais « premier dans le temps ». Jumeau, au niveau run,
        // du gardien `first_ceiling_wins_and_is_named` (niveau sizing).
        //
        // (a) Plafond COÛT — budget Stop épuisé, AUCUN deadline → `BudgetStopped`.
        let budget_run = Swarm::keyless()
            .run(
                json!({ "text": "x" }),
                TopologySpec::Consensus {
                    generator: "echo_generator".into(),
                    judges: vec!["consistency".into(), "length_band".into(), "j_non_empty".into()],
                    k: 7.into(),
                    pattern: PatternSpec::Fixed(Pattern::Borda),
                    escalate: false,
                },
                Budget::cents(4).with_policy(OnExceed::Stop), // 10 appels @ 1¢ > 4¢ → épuisé
            )
            .await
            .unwrap();
        assert_eq!(budget_run.termination, RunTermination::BudgetStopped, "plafond coût → BudgetStopped");
        assert!(!budget_run.is_clean(), "budget-stoppé = partiel");

        // (b) Plafond TEMPS — deadline serré, budget LARGE → un variant deadline, JAMAIS
        //     confondu avec le coût (le statut distingue les plafonds).
        let deadline_run = swarm_with_slow_gen(80, Some(std::time::Duration::from_millis(30)))
            .run(
                json!({ "text": "x" }),
                TopologySpec::consensus_adaptive("slow_gen", &["j_non_empty", "consistency"], 2, 24, 0.999),
                Budget::cents(1_000_000), // budget large : seul le TEMPS peut mordre
            )
            .await
            .unwrap();
        assert!(deadline_run.termination.is_deadline(), "plafond temps → variant deadline : {:?}", deadline_run.termination);
        assert_ne!(deadline_run.termination, RunTermination::BudgetStopped, "le temps n'est pas confondu avec le coût");
        assert_ne!(deadline_run.termination, RunTermination::Completed, "le run a bien été tronqué");
    }

    // ════════════════════════════════════════════════════════════════════════════════
    //  ADR-019 (MEM-2) — RECALL / LEARN dans run_traced (gardiens I-MEM côté run)
    // ════════════════════════════════════════════════════════════════════════════════

    use efe_swarm_memory::{
        family_key, FamilyMode, InMemoryMemoryStore, MemoryStore, MemoryUpdate,
        RunSummary as MemRunSummary,
    };

    /// La famille telle que le SDK la calcule pour un consensus (kind + [generator]+judges).
    fn consensus_family(task: &serde_json::Value, generator: &str, judges: &[&str]) -> efe_swarm_memory::FamilyKey {
        let mut agents = vec![generator.to_string()];
        agents.extend(judges.iter().map(|s| s.to_string()));
        family_key(task, "consensus", &agents, FamilyMode::Structural)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn memory_off_is_the_historical_path() {
        // I-MEM8 : sans mémoire, run.memory == None (et tout le reste inchangé).
        let run = Swarm::keyless()
            .run(json!({ "text": "x", "expected": "x" }), TopologySpec::Auto, Budget::cents(100))
            .await
            .unwrap();
        assert!(run.memory.is_none(), "mémoire OFF par défaut → aucune interaction mémoire");
        assert!(run.is_clean());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn recall_traces_a_memory_read_in_the_root() {
        // I-MEM2 : un run avec recall émet un record MemoryRead (rôle Memory) dans la chaîne,
        // et le run reste re-prouvé (le record de lecture n'invalide pas la racine).
        let store = std::sync::Arc::new(InMemoryMemoryStore::new());
        let swarm = Swarm::builder()
            .memory(MemoryHandle::with_config(store, crate::memory::MemoryConfig::recall_only()))
            .build()
            .unwrap();
        let outcome = swarm
            .run_traced(
                json!({ "text": "hello", "expected": "hello" }),
                TopologySpec::consensus("echo_generator", &["consistency", "j_non_empty"], 3),
                Budget::cents(1000),
            )
            .await
            .unwrap();
        assert!(outcome.run.provenance.verified, "le MemoryRead n'invalide pas la preuve");
        let has_memory_read = outcome
            .records()
            .iter()
            .any(|r| r.agent_role == efe_swarm_provenance::AgentRole::Memory);
        assert!(has_memory_read, "le recall est ANCRÉ dans la chaîne de provenance (I-MEM2)");
        let m = outcome.run.memory.as_ref().expect("résumé mémoire présent");
        assert!(m.recalled);
        assert_eq!(m.version, 0, "famille neuve → instantané genèse v0");
        assert_eq!(m.content_hash.len(), 64, "épingle content_hash présente (hex)");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn recall_warm_starts_k_min_and_is_bounded() {
        // I-MEM4 : la mémoire AMORCE k_min depuis le prior de taille, CLAMPÉ aux bornes du run.
        let task = json!({ "text": "x", "expected": "x" });
        let tenant = efe_nano_sdk::TenantId::system().to_string();
        // const_generator renvoie une valeur IDENTIQUE par variant → accord = 1.0 → l'essaim
        // s'arrête au k_min (amorcé) ; c'est ce qui rend le warm-start OBSERVABLE sur k_used.
        let family = consensus_family(&task, "const_generator", &["consistency", "j_non_empty"]);

        // (a) prior K=6 → warm-start = 6 (dans [4,24]) → accord → s'arrête à 6 (pas 4).
        let store = std::sync::Arc::new(InMemoryMemoryStore::new());
        for r in 0u128..4 {
            let up = MemoryUpdate { run_id: uuid::Uuid::from_u128(0x6000 + r), family: family.clone(), summary: MemRunSummary::clean(6) };
            store.fold(&tenant, up).await.unwrap();
        }
        let swarm = Swarm::builder()
            .memory(MemoryHandle::with_config(store, crate::memory::MemoryConfig::recall_only()))
            .build()
            .unwrap();
        let run = swarm
            .run(task.clone(), TopologySpec::consensus_adaptive("const_generator", &["consistency", "j_non_empty"], 4, 24, 0.9), Budget::cents(1_000_000))
            .await
            .unwrap();
        assert_eq!(run.memory.as_ref().unwrap().warm_start_k_min, Some(6), "warm-start = K historique");
        assert_eq!(run.quality.candidates, 6, "echo agree → arrêt au k_min amorcé (6), pas au 4 d'origine");

        // (b) prior ABSURDE K=100 → warm-start CLAMPÉ à k_max=24 (la mémoire ne gouverne pas).
        let store2 = std::sync::Arc::new(InMemoryMemoryStore::new());
        for r in 0u128..4 {
            let up = MemoryUpdate { run_id: uuid::Uuid::from_u128(0x9000 + r), family: family.clone(), summary: MemRunSummary::clean(100) };
            store2.fold(&tenant, up).await.unwrap();
        }
        let swarm2 = Swarm::builder()
            .memory(MemoryHandle::with_config(store2, crate::memory::MemoryConfig::recall_only()))
            .build()
            .unwrap();
        let run2 = swarm2
            .run(task, TopologySpec::consensus_adaptive("const_generator", &["consistency", "j_non_empty"], 4, 24, 0.9), Budget::cents(1_000_000))
            .await
            .unwrap();
        assert_eq!(run2.memory.as_ref().unwrap().warm_start_k_min, Some(24), "amorce BORNÉE à k_max (I-MEM4)");
        assert_eq!(run2.quality.candidates, 24, "jamais au-delà de k_max, quoi que dise la mémoire");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn learn_only_on_clean_runs() {
        // I-MEM3 : un run propre replie ; un run tronqué (budget) NE replie PAS (quarantaine).
        let store = std::sync::Arc::new(InMemoryMemoryStore::new());
        let task = json!({ "text": "x", "expected": "x" });
        let tenant = efe_nano_sdk::TenantId::system().to_string();
        let family = consensus_family(&task, "echo_generator", &["consistency", "length_band", "j_non_empty"]);
        let swarm = Swarm::builder().memory(MemoryHandle::new(store.clone())).build().unwrap();

        // ① run propre → replié (la mémoire passe à v1).
        let clean = swarm
            .run(
                task.clone(),
                TopologySpec::consensus("echo_generator", &["consistency", "length_band", "j_non_empty"], 3),
                Budget::cents(1000),
            )
            .await
            .unwrap();
        assert!(clean.is_clean());
        assert!(clean.memory.as_ref().unwrap().learned, "run propre → replié");
        assert_eq!(store.snapshot(&tenant, &family).await.unwrap().version, 1, "la mémoire a appris (v1)");

        // ② run tronqué par le budget (Stop épuisé : 10 appels @ 1¢ > 4¢) → MÊME famille,
        //    mais NON replié — la porte is_clean() met le run sale en quarantaine (I-MEM3).
        let stopped = swarm
            .run(
                task.clone(),
                TopologySpec::Consensus {
                    generator: "echo_generator".into(),
                    judges: vec!["consistency".into(), "length_band".into(), "j_non_empty".into()],
                    k: 7.into(),
                    pattern: PatternSpec::Fixed(Pattern::Borda),
                    escalate: false,
                },
                Budget::cents(4).with_policy(OnExceed::Stop),
            )
            .await
            .unwrap();
        assert!(!stopped.is_clean(), "budget-stoppé → pas clean");
        assert!(!stopped.memory.as_ref().unwrap().learned, "run tronqué → PAS replié (I-MEM3)");
        assert_eq!(store.snapshot(&tenant, &family).await.unwrap().version, 1, "la mémoire n'a PAS bougé (toujours v1)");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn learn_populates_pattern_prior_and_recallable_exemplars() {
        // MEM-2.b : un run consensus PROPRE replie désormais (1) un VRAI prior de pattern
        // (auparavant `None`/vide — la lecture "pattern" différée) et (2) un exemplaire
        // recall-able (ce que le few-shot Mode C injecte). On vérifie les DEUX nouvelles captures.
        let store = std::sync::Arc::new(InMemoryMemoryStore::new());
        let tenant = efe_nano_sdk::TenantId::system().to_string();
        let task = json!({ "text": "x", "expected": "x" });
        let judges = ["consistency", "length_band", "j_non_empty"];
        let family = consensus_family(&task, "echo_generator", &judges);
        let swarm = Swarm::builder().memory(MemoryHandle::new(store.clone())).build().unwrap();

        let run = swarm
            .run(task.clone(), TopologySpec::consensus("echo_generator", &judges, 3), Budget::cents(1000))
            .await
            .unwrap();
        assert!(run.is_clean());
        // (1) le pattern décidé est EXPOSÉ sur le run (consensus Borda) — auparavant inexistant.
        assert_eq!(run.decided_pattern.as_deref(), Some("borda"), "pattern décidé observable sur SwarmRun");
        assert!(run.memory.as_ref().unwrap().learned, "run propre → replié");

        // (2) la mémoire a un VRAI prior de pattern (plus jamais vide en consensus) — MEM-2.b.
        let snap = store.snapshot(&tenant, &family).await.unwrap();
        assert_eq!(snap.cells.pattern.wins.get("borda"), Some(&1), "prior de pattern peuplé (borda × 1)");
        assert_eq!(snap.cells.pattern.total, 1, "un run de pattern observé");
        assert_eq!(snap.cells.pattern.escalations, 0, "Borda fixe ⇒ aucune escalade comptée");

        // (3) le gagnant est recall-able comme exemplaire few-shot — exactement ce que la prochaine
        //     génération Mode C injecterait dans son prompt (cf. llm::gen_prompt + le mock backend).
        let recalled = crate::memory::recalled_exemplars(&snap);
        assert_eq!(recalled.len(), 1, "le gagnant du run propre est recall-able (few-shot)");
        assert!(!recalled[0].is_empty(), "l'aperçu d'exemplaire n'est pas vide");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn failure_memory_counts_empty_winner_runs_and_skips_their_exemplars() {
        // MEM-2.c : la 5ᵉ cellule (FailureMemory) est désormais ALIMENTÉE depuis un vrai run.
        // Un run PROPRE dont le gagnant est nul/vide (const_generator → Null) compte un ÉCHEC
        // (fails++) SANS déposer d'exemplaire (on ne few-shot jamais un échec) ; un run à gagnant
        // réel (echo) compte un run SANS échec (runs++, fails inchangé) et dépose son exemplaire.
        let tenant = efe_nano_sdk::TenantId::system().to_string();
        let task = json!({ "text": "x", "expected": "x" });
        let judges = ["consistency", "j_non_empty"];

        // ① run ÉCHOUÉ : gagnant Null (const_generator).
        let store_f = std::sync::Arc::new(InMemoryMemoryStore::new());
        let fam_f = consensus_family(&task, "const_generator", &judges);
        let swarm_f = Swarm::builder().memory(MemoryHandle::new(store_f.clone())).build().unwrap();
        let run_f = swarm_f
            .run(task.clone(), TopologySpec::consensus("const_generator", &judges, 3), Budget::cents(1000))
            .await
            .unwrap();
        assert!(run_f.is_clean(), "le run complète proprement (le gagnant nul ne le rend pas non-clean)");
        assert!(run_f.memory.as_ref().unwrap().learned, "un run propre replie (même s'il a échoué)");
        let snap_f = store_f.snapshot(&tenant, &fam_f).await.unwrap();
        let rec_f = snap_f.cells.failure.of(fam_f.as_str()).expect("la famille a une entrée d'échec");
        assert_eq!(rec_f.runs, 1, "un run observé");
        assert_eq!(rec_f.fails, 1, "gagnant nul → un échec compté (fold ⑤)");
        assert_eq!(rec_f.fail_rate_milli(), 1000, "taux d'échec = 100%");
        assert!(snap_f.cells.exemplars.items.is_empty(), "on ne few-shot JAMAIS un échec");
        // read-back B (I-REP5, revue) : un run ÉCHOUÉ ne crédite AUCUNE réputation — sinon le run
        // dégénéré (candidats nuls « identiques » → tous « vérifiés », generator_000 « gagne »)
        // FABRIQUERAIT victoires/vérifications et empoisonnerait le routage.
        assert!(snap_f.cells.reputation.agents.is_empty(), "un échec ne FABRIQUE aucune réputation (I-REP5)");

        // ② run RÉUSSI : gagnant réel (echo_generator).
        let store_ok = std::sync::Arc::new(InMemoryMemoryStore::new());
        let fam_ok = consensus_family(&task, "echo_generator", &judges);
        let swarm_ok = Swarm::builder().memory(MemoryHandle::new(store_ok.clone())).build().unwrap();
        let run_ok = swarm_ok
            .run(task.clone(), TopologySpec::consensus("echo_generator", &judges, 3), Budget::cents(1000))
            .await
            .unwrap();
        assert!(run_ok.is_clean());
        let snap_ok = store_ok.snapshot(&tenant, &fam_ok).await.unwrap();
        let rec_ok = snap_ok.cells.failure.of(fam_ok.as_str()).expect("entrée d'échec présente");
        assert_eq!(rec_ok.runs, 1, "un run observé");
        assert_eq!(rec_ok.fails, 0, "gagnant réel → aucun échec");
        assert!(!snap_ok.cells.exemplars.items.is_empty(), "un gagnant réel dépose son exemplaire");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn real_escalation_is_folded_into_the_pattern_prior() {
        // MEM-2.b (moitié POSITIVE de l'escalade) : un run qui escalade RÉELLEMENT — echo →
        // candidats non unanimes (chacun porte son `variant`) → vote peu confiant → juges payés —
        // replie `escalations == 1` dans le prior de pattern. Le pendant du cas négatif déjà
        // couvert (Borda fixe → escalations == 0), via le bout-en-bout SDK→TopologyOutput→fold.
        let store = std::sync::Arc::new(InMemoryMemoryStore::new());
        let tenant = efe_nano_sdk::TenantId::system().to_string();
        let task = json!({ "text": "x", "expected": "x" });
        let judges = ["consistency", "j_non_empty"];
        let family = consensus_family(&task, "echo_generator", &judges);
        let swarm = Swarm::builder().memory(MemoryHandle::new(store.clone())).build().unwrap();

        let run = swarm
            .run(
                task.clone(),
                TopologySpec::Consensus {
                    generator: "echo_generator".into(),
                    judges: judges.iter().map(|s| s.to_string()).collect(),
                    k: 3.into(),
                    pattern: PatternSpec::Fixed(Pattern::Borda), // escalate_to
                    escalate: true,
                },
                Budget::cents(100_000),
            )
            .await
            .unwrap();
        assert!(run.is_clean());
        assert!(run.decided_pattern.is_some(), "un pattern est décidé + exposé");

        let snap = store.snapshot(&tenant, &family).await.unwrap();
        assert_eq!(snap.cells.pattern.total, 1, "un run de pattern observé");
        assert_eq!(snap.cells.pattern.escalations, 1, "l'escalade RÉELLE est comptée (≠ 0)");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn consensus_populates_per_variant_reputation_not_degenerate() {
        // read-back B (B1/B2) : un consensus propre replie une réputation PAR-VARIANTE — plusieurs
        // ids stables `generator_NNN`, un seul gagnant attribué. Plus l'agent métré unique dégénéré.
        let store = std::sync::Arc::new(InMemoryMemoryStore::new());
        let tenant = efe_nano_sdk::TenantId::system().to_string();
        let task = json!({ "text": "x", "expected": "x" });
        let judges = ["consistency", "length_band", "j_non_empty"];
        let family = consensus_family(&task, "echo_generator", &judges);
        let swarm = Swarm::builder().memory(MemoryHandle::new(store.clone())).build().unwrap();
        let run = swarm
            .run(task.clone(), TopologySpec::consensus("echo_generator", &judges, 3), Budget::cents(1000))
            .await
            .unwrap();
        assert!(run.is_clean());

        let snap = store.snapshot(&tenant, &family).await.unwrap();
        let rep = &snap.cells.reputation;
        assert!(
            rep.agents.len() >= 2,
            "réputation par-variante NON dégénérée : {:?}",
            rep.agents.keys().collect::<Vec<_>>()
        );
        assert!(rep.agents.contains_key("generator_000"), "id par-variante stable (I-REP1)");
        let winners: u64 = rep.agents.values().map(|a| a.won).sum();
        assert_eq!(winners, 1, "exactement UN gagnant attribué par run (I-REP5)");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn consensus_pattern_adaptive_is_reachable_and_clean() {
        // read-back A : le pattern ADAPTATIF (jadis code mort, inatteignable depuis la spec) est
        // désormais ATTEIGNABLE, tourne proprement, et décide un pattern selon l'accord des candidats.
        let run = Swarm::keyless()
            .run(
                json!({ "text": "x", "expected": "x" }),
                TopologySpec::consensus_pattern_adaptive("echo_generator", &["consistency", "j_non_empty"], 3),
                Budget::cents(1000),
            )
            .await
            .unwrap();
        assert!(run.is_clean(), "le consensus à pattern adaptatif tourne proprement");
        assert!(run.decided_pattern.is_some(), "le sélecteur adaptatif a décidé un pattern");
    }
}
