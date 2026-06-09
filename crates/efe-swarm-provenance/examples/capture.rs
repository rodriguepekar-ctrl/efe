// SPDX-License-Identifier: UNLICENSED
//! # `capture` — les 2 seams en action, SANS réécrire les agents (Phase 4, Inc.3).
//!
//! - **Seam B (décorateur)** : on enveloppe un générateur Mode A et 3 juges Mode C
//!   avec [`ProvenancedAgent`] — ils restent des `Generator`/`Judge` ordinaires.
//! - **Seam A (sink)** : un [`ProvenanceSink`] branché sur le `Blackboard` ancre
//!   **chaque** écriture (`/cand/*`, `/ballot/*`).
//! - Le [`ProvenanceRecorder`] **fusionne** les deux → chaîne HMAC + `provenance_root`.
//!
//! Run :  cargo run --example capture -p efe-swarm-provenance

use std::sync::Arc;

use async_trait::async_trait;
use efe_blackboard::{Blackboard, WriteSink};
use efe_nano_sdk::{AgentId, SwarmId, TenantId};
use efe_swarm_provenance::{
    verify_chain, HmacKeySource, ProvenanceRecorder, ProvenanceSink, ProvenancedAgent,
};
use efe_swarm_topology::{Generator, Judge};
use serde_json::{json, Value};
use uuid::Uuid;

/// Générateur Mode A : variants 0..n_correct corrects, le reste un leurre.
struct SimGen;
#[async_trait]
impl Generator for SimGen {
    async fn generate(&self, task: &Value, variant: usize) -> Value {
        let n = task.get("n_correct").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        if variant < n {
            json!({ "value": task.get("expected").cloned().unwrap_or(Value::Null) })
        } else {
            json!({ "value": "__lure__" })
        }
    }
}

/// Juge Mode C (simulé) : 1.0 si le candidat == attendu, sinon 0.0.
struct EqJudge(String);
#[async_trait]
impl Judge for EqJudge {
    async fn assess(&self, task: &Value, candidates: &[Value]) -> Vec<f64> {
        let exp = task.get("expected").cloned().unwrap_or(Value::Null);
        candidates.iter().map(|c| if c.get("value") == Some(&exp) { 1.0 } else { 0.0 }).collect()
    }
    fn name(&self) -> &str {
        &self.0
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let tenant = TenantId::system();
    let swarm = SwarmId::from_uuid(Uuid::from_u128(0x2222));
    let recorder = Arc::new(ProvenanceRecorder::new(swarm, tenant));

    // ── Seam A : sink branché sur le Blackboard (ancre 100 % des écritures). ─────
    let bb = Blackboard::new(swarm);
    bb.set_write_sink(Arc::new(ProvenanceSink::new(recorder.clone())) as Arc<dyn WriteSink>);

    // ── Seam B : agents Mode A/C enveloppés — restent de simples Generator/Judge. ─
    let gen = ProvenancedAgent::mode_a(SimGen, recorder.clone(), AgentId::new("generator").unwrap());
    let judges: Vec<ProvenancedAgent<EqJudge>> = (0..3)
        .map(|j| {
            ProvenancedAgent::mode_c(
                EqJudge(format!("judge_{j:03}")),
                recorder.clone(),
                AgentId::new(format!("judge_{j:03}")).unwrap(),
            )
        })
        .collect();

    println!("══════════════════════════════════════════════════════════════════════════");
    println!(" CAPTURE PAR 2 SEAMS — décorateur (entrée→sortie) + sink (ancrage 100 %)");
    println!("══════════════════════════════════════════════════════════════════════════");

    // ── Un mini-run : K=5 générations (séquentielles → versions déterministes). ──
    let task = json!({ "expected": "tech", "n_correct": 3, "difficulty": 0.7 });
    let mut candidates = Vec::new();
    for variant in 0..5 {
        let cand = gen.generate(&task, variant).await; // seam B capte l'appel
        bb.write(&format!("/cand/{variant}"), cand.clone(), AgentId::new("generator").unwrap())
            .unwrap(); // seam A ancre l'écriture
        candidates.push(cand);
    }
    // ── 3 juges notent les candidats, écrivent leurs ballots. ───────────────────
    for (j, judge) in judges.iter().enumerate() {
        let scores = judge.assess(&task, &candidates).await; // seam B
        bb.write(&format!("/ballot/{j}"), json!({ "scores": scores }), AgentId::new(format!("judge_{j:03}")).unwrap())
            .unwrap(); // seam A
    }

    println!("\n seam B (décorateur) : {} appels d'agent captés", recorder.captured_calls());
    println!(" seam A (sink)       : {} écritures ancrées", recorder.anchored_writes());

    // ── Fusion + scellement → chaîne HMAC + provenance_root. ────────────────────
    let sealed = recorder.seal(&HmacKeySource::test_key()).expect("seal");
    println!("\n fusion → {} enregistrements (ordonnés par version d'écriture) :", sealed.len());
    for r in &sealed.records {
        println!(
            "   seq {} · {:<11} · {:<9} · {:<7} · writes={:?}",
            r.seq,
            r.agent_id,
            r.agent_role.as_str(),
            r.agent_kind.as_str(),
            r.writes.iter().map(|w| format!("{}@{}", w.pointer, w.version)).collect::<Vec<_>>(),
        );
    }
    println!("\n ► provenance_root : {}", sealed.root_hex().unwrap());
    println!(
        " couverture : {}/{} enregistrements ancrés à une cellule ({:.0}%)",
        sealed.anchored_records,
        sealed.len(),
        100.0 * sealed.anchored_record_ratio()
    );

    // ── Vérification indépendante. ──────────────────────────────────────────────
    let key = HmacKeySource::test_key().derive(tenant).unwrap();
    println!("\n vérification de la chaîne scellée : ok={}", verify_chain(&sealed.records, &key).ok);

    println!("\n──────────────────────────────────────────────────────────────────────────");
    println!(" → Aucun agent n'a été réécrit : SimGen/EqJudge ignorent la provenance. Le");
    println!("   décorateur capte entrée→sortie, le sink ancre 100 % des écritures, le");
    println!("   recorder fusionne en une chaîne dont le root prouve tout le run.");
    println!("══════════════════════════════════════════════════════════════════════════");
}
