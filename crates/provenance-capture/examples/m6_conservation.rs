// SPDX-License-Identifier: UNLICENSED
//! # M6, rendu visible — `cargo run -p provenance-capture --example m6_conservation`
//!
//! Le critère de M6 (« un cas expiré disparaît, et tout le reste se re-prouve encore ») est prouvé
//! par des tests. Un test qui passe ne montre rien : cet exemple imprime **le manifest-root avant et
//! après le broyage**, pour qu'un lecteur constate lui-même qu'il ne bouge pas — et le contenu
//! disparaître à la ligne d'à côté.
//!
//! Il n'y a rien à croire ici : les deux racines sont à l'écran, et ce sont les mêmes.

use provenance_capture::conservation::{lire, Conservation};
use provenance_capture::{CaptureStore, LlmCall};
use provenance_dag::shred::{CaseKey, EtatCas, Lecture, Trousseau};
use provenance_dag::{Node, TenantKey};

/// Trois dossiers : deux ouverts il y a longtemps, un récent.
const DOSSIERS: [(&str, u64, &str); 3] =
    [("SUP-2023", 0, "COMPLIANT"), ("SUP-2024", 100, "PASS"), ("SUP-2026", 900, "ESCALADE")];

/// L'horizon de conservation, et l'heure qu'il est. 500 s d'horizon à t=1000 : les deux premiers
/// dossiers ont expiré.
const HORIZON_S: u64 = 500;
const MAINTENANT: u64 = 1000;

fn main() {
    // ⚠ En production ces clés sont ALÉATOIRES. Ici elles sont fabriquées pour que la sortie soit
    // reproductible ; dériver en production annulerait le broyage (cf. `provenance_dag::shred`).
    let cle = |cas: &str| {
        let mut b = [0u8; 32];
        for (i, o) in cas.bytes().enumerate().take(32) {
            b[i] = o;
        }
        CaseKey::from_bytes(b)
    };

    let mut store = CaptureStore::new(TenantKey::derive(b"racine-de-demonstration", "acme"));
    store.avec_conservation(Trousseau::new());

    println!("┌─ M6 — la conservation limitée ─────────────────────────────────────────────┐");
    println!("│ horizon : {HORIZON_S} s · il est : {MAINTENANT}");
    println!("└────────────────────────────────────────────────────────────────────────────┘\n");

    for (cas, ouvert_a, verdict) in DOSSIERS {
        store.ouvrir_cas(cas, cle(cas), ouvert_a);
        store
            .record_llm(&LlmCall {
                case_id: cas,
                step: "verdict",
                agent_role: "verdict_aggregate",
                inputs: b"pieces-du-dossier",
                model: "phi4",
                version: "1",
                output: verdict,
                evidence_steps: &[],
                timestamp: ouvert_a,
            })
            .expect("ingestion");
    }

    let racine_avant = store.reprove().manifest_root.clone().expect("racine");
    println!("AVANT le broyage");
    println!("  manifest-root : {racine_avant}");
    etat_des_dossiers(&store);

    // Ce qui est publié ne contient jamais le clair — on le montre plutôt que de l'affirmer.
    let Node::AgentAct { output, .. } = &store.records()[0].node else { unreachable!() };
    println!("  ce qui est SCELLÉ pour SUP-2023 : {}…", &output[..output.len().min(38)]);

    let balayage = Conservation::nouvelle(HORIZON_S)
        .appliquer(store.conservation_mut().expect("conservation active"), MAINTENANT);

    println!("\nLA POLITIQUE PASSE");
    println!(
        "  broyés : {}  ·  vivants restants : {}  ·  broyés en tout : {}",
        balayage.broyes.join(", "),
        balayage.vivants,
        balayage.total_broyes
    );

    let apres = store.reprove();
    println!("\nAPRÈS le broyage");
    println!("  manifest-root : {}", apres.manifest_root.clone().unwrap_or_default());
    etat_des_dossiers(&store);

    println!("\n────────────────────────────────────────────────────────────────────────────");
    let identique = apres.manifest_root.as_ref() == Some(&racine_avant);
    println!(
        "  la chaîne se re-prouve : {}   ·   la racine est la MÊME : {}",
        if apres.ok { "oui" } else { "NON" },
        if identique { "oui" } else { "NON" }
    );
    println!(
        "\n  Ce qui reste d'un dossier broyé : qu'un acte a eu lieu, son rôle, sa date, ses arêtes."
    );
    println!("  Ce qui part : ce qu'il disait. C'est la limitation de conservation, et rien de plus.");
    println!("  Le nœud n'est PAS supprimé — c'est ce qui permet à la chaîne de tenir.\n");

    assert!(apres.ok && identique, "le critère de M6 doit tenir dans cette démonstration");
}

fn etat_des_dossiers(store: &CaptureStore) {
    let t = store.conservation().expect("conservation active");
    for (i, (cas, ..)) in DOSSIERS.iter().enumerate() {
        let vue = lire(&store.records()[i].node, t);
        let quoi = match &vue["output"] {
            Lecture::Ouvert(v) => format!("« {v} »"),
            Lecture::Broye { le } => format!("— détruit à t={le} —"),
            Lecture::CleAbsente => "— clé absente —".into(),
            Lecture::EnClair(v) => format!("« {v} » (jamais chiffré)"),
            Lecture::Illisible => "— illisible —".into(),
        };
        let etat = match t.etat(cas) {
            EtatCas::Vivant { depuis } => format!("vivant depuis t={depuis}"),
            EtatCas::Broye { le } => format!("BROYÉ à t={le}"),
            EtatCas::Inconnu => "inconnu".into(),
        };
        println!("  {cas:<10} {etat:<22} verdict : {quoi}");
    }
}
