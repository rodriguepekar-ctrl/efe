// SPDX-License-Identifier: UNLICENSED
//! `GET /v1/swarm/replay/:id` (R0) + `POST /v1/swarm/replay/:id` (R1) — **rejeu déterministe**
//! (ADR-SWARM-017). Là où `verify/:id` **re-prouve** l'intégrité, `replay/:id` **re-dérive** la
//! reproductibilité : « si je relance, j'obtiens la même chose ? ».
//!
//! - **R0 (`GET`, zéro-input, hors-ligne, gratuit)** : re-scelle les records scellés en ordre
//!   canonique (ADR-007, [`efe_swarm_provenance::replay`]) → la racine recomposée doit égaler la
//!   racine stockée ; localise la 1ʳᵉ rupture de contenu ; classe chaque record par
//!   [`DeterminismLevel`] (l'inventaire de rejouabilité) ; re-vérifie la chaîne N17 **sans la
//!   recomputer** (I-RP7).
//! - **R1 (`POST { task, topology, … }`, re-exécution)** : la task n'étant **pas** persistée,
//!   le caller la re-fournit ; on re-exécute l'essaim **Mode A** sur un substrat vierge et on
//!   compare la racine fraîche + les records à ceux scellés (I-RP1 bit-à-bit). Le Mode C
//!   (re-appel provider) est hors-périmètre (§10) : ses appels sont **honnêtement classés**,
//!   jamais re-exécutés en aveugle.

use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::Json;
use efe_swarm_provenance::{deterministic_order, replay, verify_run_offline, AgentCallRecord, AgentKind, AgentRole};
use efe_swarm_sdk::LlmProvenanceRecord;
use uuid::Uuid;

use crate::auth::Caller;
use crate::dto::{
    DeterminismLevel, DivergenceView, ExecutionReplayView, MatchOutcome, N17VerifyView, ReplayDeterminismSummary,
    ReplayOpView, ReplayReport, RunRequest,
};
use crate::error::{GatewayError, GatewayJson};
use crate::state::SwarmGatewayState;
use crate::store::StoredRun;

/// **Empreinte de calcul IDENTITÉ-LIBRE** d'un record : tout ce que l'agent a *calculé*
/// (agent, rôle, mode, variant, hash d'entrée/sortie, cellules lues/écrites), **sans** les
/// champs d'identité **par-run** (`record_id`, `swarm_id`, `causal_parents`, `llm_provenance_id`)
/// — ceux qui lient le run et rendent la `provenance_root` non reproductible par re-exécution.
/// Deux runs « computationnellement équivalents » ont le même multiset d'empreintes.
fn compute_fp(r: &AgentCallRecord) -> String {
    let cells = |cs: &[efe_swarm_provenance::CellRef]| {
        let mut v: Vec<String> = cs.iter().map(|c| format!("{}@{}", c.pointer, c.version)).collect();
        v.sort();
        v.join(",")
    };
    format!(
        "{}|{}|{}|{:?}|{}|{}|r[{}]|w[{}]",
        r.agent_id.as_str(),
        r.agent_role.as_str(),
        r.agent_kind.as_str(),
        r.variant,
        r.input_hash.to_hex(),
        r.output_hash.to_hex(),
        cells(&r.reads),
        cells(&r.writes),
    )
}

/// Classe la rejouabilité d'un record d'essaim depuis son mode + (Mode C) la forensique liée.
fn classify(rec: &AgentCallRecord, llm_by_id: &HashMap<Uuid, &LlmProvenanceRecord>) -> DeterminismLevel {
    match rec.agent_kind {
        AgentKind::ModeA => DeterminismLevel::FullDeterministic,
        AgentKind::ModeC => match rec.llm_provenance_id.and_then(|id| llm_by_id.get(&id.as_uuid())) {
            Some(l) if l.is_cache_hit() || l.is_replay() => DeterminismLevel::LlmReplayable,
            Some(l) if l.sampling.seed.is_some() => DeterminismLevel::Seeded,
            _ => DeterminismLevel::LlmEphemeral,
        },
    }
}

/// Construit le rapport **R0** (re-sceller depuis les records scellés) — la base commune à
/// R0 et R1. Aucune exécution d'agent ; lecture-seule sur le store (I-RP4).
fn build_r0(id: Uuid, stored: &StoredRun, key: &[u8], tier: &'static str) -> ReplayReport {
    // (1) re-scellement déterministe (ADR-007) : la racine se reproduit-elle ?
    let rep = replay(&stored.records, key);
    let reproduced_root = rep.root_hex();
    let root_reproduced = reproduced_root == stored.root;

    // (2) localisation d'une rupture de contenu (ordre canonique).
    let ordered = deterministic_order(&stored.records);
    let first_divergence = rep.first_content_break.and_then(|pos| {
        ordered.get(pos).map(|r| DivergenceView {
            position: pos,
            record_id: r.record_id.to_string(),
            agent_id: r.agent_id.as_str().to_string(),
            reason: "content_break".into(),
        })
    });

    // (3) inventaire de rejouabilité : classe chaque record.
    let llm_by_id: HashMap<Uuid, &LlmProvenanceRecord> =
        stored.llm_records.iter().map(|l| (l.provenance_id.as_uuid(), l)).collect();
    let mut determinism = ReplayDeterminismSummary::default();
    let per_op: Vec<ReplayOpView> = stored
        .records
        .iter()
        .map(|r| {
            let level = classify(r, &llm_by_id);
            match level {
                DeterminismLevel::FullDeterministic => determinism.full_deterministic += 1,
                DeterminismLevel::Seeded => determinism.seeded += 1,
                DeterminismLevel::LlmReplayable => determinism.llm_replayable += 1,
                DeterminismLevel::LlmEphemeral => determinism.llm_ephemeral += 1,
            }
            ReplayOpView {
                record_id: r.record_id.to_string(),
                agent_id: r.agent_id.as_str().to_string(),
                kind: r.agent_kind.as_str().to_string(),
                level,
                outcome: MatchOutcome::NotReexecuted, // R0 ne re-exécute pas ; R1 raffinera
            }
        })
        .collect();

    // (4) N17 RE-VÉRIFIÉE (pas re-produite — I-RP7) : on relit les feuilles scellées.
    let v = verify_run_offline(
        &stored.records,
        &stored.llm_records,
        &stored.n17_leaf_bytes(),
        stored.n17_root_bytes(),
        key,
    );
    let n17 = N17VerifyView {
        records: stored.llm_records.len(),
        chain_intact: v.n17_chain.intact,
        root: v.n17_chain.root_hex(),
        bridge_consistent: v.bridge.all_mode_c_consistent(),
        mode_c_consistent: v.bridge.consistent,
        mode_c_unlinked: v.bridge.unlinked,
        mode_c_broken: v.bridge.broken,
        first_break: v.n17_chain.first_break.map(|p| p.as_uuid().to_string()),
    };

    ReplayReport {
        run_id: id,
        tier,
        root_reproduced,
        reproduced_root,
        stored_root: stored.root.clone(),
        content_intact: rep.content_intact,
        records: stored.records.len(),
        first_divergence,
        determinism,
        per_op,
        n17,
        execution: None,
    }
}

/// **R0** — `GET /v1/swarm/replay/:id`. Re-sceller depuis les records scellés (hors-ligne,
/// zéro-input). Re-prouve que l'**ensemble** des records commet la racine annoncée + inventaire
/// de rejouabilité. Lecture-seule (I-RP4), tenant-scopé (I-RP5).
pub async fn get_replay(
    State(s): State<SwarmGatewayState>,
    caller: Caller,
    Path(id): Path<Uuid>,
) -> Result<Json<ReplayReport>, GatewayError> {
    caller.require_any_role(&["viewer", "operator", "admin", "auditor"])?;
    let stored = s.store.get(id, &caller.tenant).await?.ok_or(GatewayError::NotFound)?;
    let key = s.derive_key(&caller.tenant)?;
    Ok(Json(build_r0(id, &stored, &key, "R0")))
}

/// **R1** — `POST /v1/swarm/replay/:id` avec un `RunRequest` (le caller **re-fournit la task**,
/// non persistée). Re-exécute l'essaim **Mode A** sur un substrat vierge et compare la racine
/// fraîche + les records aux scellés (I-RP1). Le store n'est **jamais** muté (I-RP4) : la
/// re-exécution ne persiste rien. Mode C : re-exécution provider hors-périmètre (§10) → classé.
pub async fn post_replay(
    State(s): State<SwarmGatewayState>,
    caller: Caller,
    Path(id): Path<Uuid>,
    GatewayJson(req): GatewayJson<RunRequest>,
) -> Result<Json<ReplayReport>, GatewayError> {
    caller.require_any_role(&["operator", "admin"])?; // re-exécution = compute → rôle opérateur
    let stored = s.store.get(id, &caller.tenant).await?.ok_or(GatewayError::NotFound)?;
    let key = s.derive_key(&caller.tenant)?;
    let mut report = build_r0(id, &stored, &key, "R1");

    // Run Mode C : la re-exécution exigerait le provider (hors-périmètre, ADR-017 §10). On NE
    // re-exécute PAS en aveugle ; on classe honnêtement chaque appel (I-RP6).
    if stored.has_n17() {
        for op in &mut report.per_op {
            op.outcome = match op.level {
                DeterminismLevel::LlmReplayable => MatchOutcome::Match, // re-dérivable par identité (cache/replay_of)
                DeterminismLevel::Seeded | DeterminismLevel::LlmEphemeral => MatchOutcome::SkippedNonDeterministic,
                DeterminismLevel::FullDeterministic => MatchOutcome::NotReexecuted, // Mode A intercalé : non re-exécuté ici
            };
        }
        report.execution = Some(ExecutionReplayView {
            ran: false,
            outputs_reproduced: None,
            task_mismatch: false,
            mode_a_records: report.determinism.full_deterministic,
            mode_c_records: stored.llm_records.len(),
            fresh_root: None,
            note: "mode_c : re-exécution provider hors-périmètre (ADR-017 §10) — appels classés par DeterminismLevel (cache/replay_of → match ; éphémère → skipped). Re-preuve N17 dans le bloc n17.".into(),
        });
        return Ok(Json(report));
    }

    // Run Mode A (keyless) : re-exécuter sur la task re-fournie, comparer (mirroir de /run, SANS persister).
    let width = req.width;
    let (task, spec, mut budget, mode) = req.into_parts()?;
    let (mode, allow_remote, price_override) = s.resolve_mode(mode)?;
    if let Some(price) = price_override {
        budget.price_per_call_cents.get_or_insert(price);
    }
    let execution = match s.swarm_for(&caller.tenant, mode, allow_remote, width) {
        Ok(swarm) => match swarm.run_traced(task, spec, budget).await {
            Ok(outcome) => {
                let fresh = outcome.records();
                let fresh_root = replay(fresh, &key).root_hex();

                // ── GARDE ANTI-CONFUSION (§5.1) : la task re-fournie correspond-elle aux entrées
                //    scellées ? Les générateurs ont input_hash = hash(task, variant) ; si le
                //    multiset de leurs input_hash diverge, le caller a re-soumis une AUTRE task
                //    → on N'ÉMET PAS un faux `outputs_reproduced: false` (qui ferait croire le
                //    run non reproductible) ; on signale une erreur d'opérateur (task_mismatch).
                let gen_inputs = |recs: &[AgentCallRecord]| -> std::collections::BTreeMap<String, usize> {
                    let mut m = std::collections::BTreeMap::new();
                    for r in recs.iter().filter(|r| matches!(r.agent_role, AgentRole::Generator)) {
                        *m.entry(r.input_hash.to_hex()).or_insert(0) += 1;
                    }
                    m
                };
                if gen_inputs(&stored.records) != gen_inputs(fresh) {
                    ExecutionReplayView {
                        ran: true,
                        outputs_reproduced: None,
                        task_mismatch: true,
                        mode_a_records: fresh.len(),
                        mode_c_records: 0,
                        fresh_root,
                        note: "task_mismatch : la task re-fournie ne correspond PAS aux entrées scellées du run (input_hash des générateurs divergents) — re-fournissez la task d'origine. Aucun verdict de reproductibilité émis (≠ run non reproductible).".into(),
                    }
                } else {
                    // ── La COMPUTATION se reproduit-elle ? On compare les empreintes de calcul
                    //    IDENTITÉ-LIBRES (output_hash + input_hash + cellules + variant + agent),
                    //    PAS la racine : la racine lie swarm_id/record_id/parents (frais par-run),
                    //    donc elle DIFFÈRE forcément — re-provable par R0, pas reproductible ici.
                    //    Comparaison par multiset (robuste au ré-ordonnancement concurrent).
                    let mut stored_fps: HashMap<String, usize> = HashMap::new();
                    for r in &stored.records {
                        *stored_fps.entry(compute_fp(r)).or_insert(0) += 1;
                    }
                    let mut fresh_fps: HashMap<String, usize> = HashMap::new();
                    for r in fresh {
                        *fresh_fps.entry(compute_fp(r)).or_insert(0) += 1;
                    }
                    let outputs_reproduced = stored_fps == fresh_fps;

                    // localisation : 1ʳᵉ empreinte scellée (ordre canonique) absente côté frais.
                    if !outputs_reproduced {
                        let mut remaining = fresh_fps.clone();
                        for (pos, r) in deterministic_order(&stored.records).iter().enumerate() {
                            let fp = compute_fp(r);
                            match remaining.get_mut(&fp) {
                                Some(n) if *n > 0 => *n -= 1,
                                _ => {
                                    report.first_divergence = Some(DivergenceView {
                                        position: pos,
                                        record_id: r.record_id.to_string(),
                                        agent_id: r.agent_id.as_str().to_string(),
                                        reason: "output_mismatch".into(),
                                    });
                                    break;
                                }
                            }
                        }
                    }
                    // outcomes par-op (Mode A) : Match si son empreinte se re-dérive, sinon Mismatch.
                    for op in &mut report.per_op {
                        if op.level == DeterminismLevel::FullDeterministic {
                            op.outcome = if outputs_reproduced { MatchOutcome::Match } else { MatchOutcome::Mismatch };
                        }
                    }
                    ExecutionReplayView {
                        ran: true,
                        outputs_reproduced: Some(outputs_reproduced),
                        task_mismatch: false,
                        mode_a_records: fresh.len(),
                        mode_c_records: 0,
                        fresh_root,
                        note: if outputs_reproduced {
                            "mode_a : sorties + flux de données re-dérivés à l'identique (I-RP1). La racine fraîche diffère par CONCEPTION (identité par-run) ; la racine scellée est re-prouvée par R0.".into()
                        } else {
                            "mode_a : la re-exécution NE re-dérive PAS les mêmes sorties (non-reproductibilité réelle) — 1ʳᵉ divergence localisée.".into()
                        },
                    }
                }
            }
            Err(e) => ExecutionReplayView {
                ran: false,
                outputs_reproduced: None,
                task_mismatch: false,
                mode_a_records: report.determinism.full_deterministic,
                mode_c_records: 0,
                fresh_root: None,
                note: format!("re-exécution échouée : {e}"),
            },
        },
        Err(e) => ExecutionReplayView {
            ran: false,
            outputs_reproduced: None,
            task_mismatch: false,
            mode_a_records: report.determinism.full_deterministic,
            mode_c_records: 0,
            fresh_root: None,
            note: format!("instanciation de l'essaim échouée : {e}"),
        },
    };
    report.execution = Some(execution);
    Ok(Json(report))
}
