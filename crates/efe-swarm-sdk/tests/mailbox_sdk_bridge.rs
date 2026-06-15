// SPDX-License-Identifier: UNLICENSED
//! **ADR-015 × SDK — le contrat inter-crate du DRAINAGE MÉTRÉ, prouvé de bout en bout.**
//!
//! L'architecture §5.1 affirme : « le drainage métré (la lettre entre dans la TÂCHE,
//! jamais à côté) est la convention d'intégration SDK, gardée par test ». Les autres
//! gardiens couvrent les MORCEAUX — la forme d'`attach_to_task` (mailbox.rs), le
//! mécanisme `declared_reads` sur le gossip (iterative_keyless.rs), la couche
//! provenance par réplication manuelle (mailbox_provenance.rs). Ce test ferme le
//! couplage LITTÉRAL qu'aucun autre ne touche : une `Letter` de boîte, attachée à la
//! tâche d'un agent, passe par le **vrai Meter du SDK** (`run_traced` → `declared_reads`)
//! et sa lecture est **scellée** dans le record — puis le run se re-prouve.
//!
//! Pourquoi c'est nécessaire : si un futur changement faisait diverger la SORTIE
//! d'`attach_to_task` (efe-blackboard) de ce que `declared_reads` (efe-swarm-sdk) sait
//! parser, tous les tests par-crate resteraient verts et la lettre serait silencieusement
//! NON tracée. Ce test inter-crate fait casser la CI à ce moment-là.

use efe_blackboard::{Blackboard, Mailboxes, MsgKind, TopicRouter};
use efe_nano_sdk::{AgentId, SwarmId};
use efe_swarm_sdk::{Budget, Swarm, TopologySpec};
use serde_json::json;

fn aid(s: &str) -> AgentId {
    AgentId::new(s).unwrap()
}

/// Drainage métré : une lettre adressée → `attach_to_task` → run SDK réel → la lecture
/// de la cellule de courrier est liftée par le Meter et scellée dans CHAQUE record
/// générateur ; le run se re-prouve (la lecture est « externe » au run — tolérée).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mailbox_letter_read_is_lifted_by_the_real_sdk_meter() {
    // ── 1) bob reçoit une lettre d'alice (boîte sur la vérité, drainée). ──────────
    let bb = Blackboard::new(SwarmId::new());
    let router = TopicRouter::new(64);
    let boxes = Mailboxes::new();
    let (alice, bob) = (aid("alice"), aid("bob"));
    boxes
        .send(&router, &bb, &alice, &bob, MsgKind::Request, None, json!({ "q": "process this" }))
        .unwrap();
    let mut bob_box = boxes.reader(&router, &bob);
    let letters = bob_box.drain(&bb);
    assert_eq!(letters.len(), 1);
    let letter = &letters[0];
    let pointer = letter.pointer.clone();
    assert_eq!(letter.version, 1, "write-once");

    // ── 2) la lettre ENTRE DANS LA TÂCHE de l'agent (drainage métré). ─────────────
    let task = letter.attach_to_task(&json!({ "text": "work", "expected": "work" }));
    assert_eq!(task["_reads"][0]["pointer"], json!(pointer), "la déclaration est dans la tâche");

    // ── 3) run à travers le VRAI SDK : le Meter (declared_reads) lifte la lecture. ─
    let swarm = Swarm::keyless();
    let out = swarm
        .run_traced(
            task,
            TopologySpec::GenerateVerify {
                generator: "echo_generator".into(),
                verifier: "non_empty_text".into(),
                k: 2.into(),
            },
            Budget::cents(100),
        )
        .await
        .expect("run keyless");

    // ── 4) la lecture de la cellule de courrier est SCELLÉE — par le Meter, pas à la
    //       main — dans chaque record générateur, et le run se re-prouve. ──────────
    assert!(out.run.provenance.verified, "verify_swarm re-prouve");
    let sealed = out.sealed.as_ref().expect("scellé");
    let gen_records: Vec<_> =
        sealed.records.iter().filter(|r| r.agent_id.as_str() == "t1_generator").collect();
    assert_eq!(gen_records.len(), 2, "k=2 générateurs");
    for rec in &gen_records {
        assert!(
            rec.reads.iter().any(|c| c.pointer == pointer && c.version == 1),
            "le VRAI Meter a lifté la lecture de la lettre — attach_to_task ↔ declared_reads tient : {:?}",
            rec.reads
        );
    }
}

/// Contrôle (anti-faux-positif) : SANS `attach_to_task`, le record ne porte AUCUNE
/// lecture — la lecture scellée du test ci-dessus vient bien de la lettre, pas d'un
/// artefact du chemin générateur.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn without_attach_no_mail_read_is_sealed() {
    let swarm = Swarm::keyless();
    let out = swarm
        .run_traced(
            json!({ "text": "work", "expected": "work" }), // tâche NUE, sans _reads
            TopologySpec::GenerateVerify {
                generator: "echo_generator".into(),
                verifier: "non_empty_text".into(),
                k: 2.into(),
            },
            Budget::cents(100),
        )
        .await
        .expect("run keyless");
    let sealed = out.sealed.as_ref().expect("scellé");
    assert!(
        sealed.records.iter().all(|r| r.reads.is_empty()),
        "aucune lecture déclarée sans attach_to_task (la lecture vient de la lettre, pas du chemin)"
    );
}
