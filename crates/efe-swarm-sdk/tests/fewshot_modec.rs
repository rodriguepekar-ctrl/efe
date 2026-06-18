// SPDX-License-Identifier: UNLICENSED
//! **ADR-019 (MEM-2.b) — le few-shot Mode C, mesuré sur un VRAI modèle (gate M-§B…F).**
//!
//! La structure du few-shot est déjà gardée en CI par des tests **purs** sur `gen_prompt`
//! (`llm.rs::tests` : froid = forme historique I-MEM8 ; chaud = bloc borné ; câblage
//! resolver→prompt). Reste l'unique mesure que l'on ne peut PAS faire sans un vrai modèle :
//! **injecter des exemplaires change-t-il, de façon mesurable et bornée, ce que le modèle
//! produit ?** On ne simule pas un LLM (ce serait inventer le résultat qu'on prétend mesurer) —
//! donc ce harnais est **réel mais gardé** : `#[ignore]`, lancé à la demande sur Ollama local
//! (`cargo test -p efe-swarm-sdk --test fewshot_modec -- --ignored`).
//!
//! Ce qu'il prouve, sur un vrai modèle (architecture du Mode C réel, §11) :
//! - **M-§B influence mesurée** : sur une famille où des gagnants passés aident la *forme* de
//!   réponse, le run **chaud** (exemplaires injectés) est **au moins aussi conforme** que le
//!   **froid** — un delta rapporté, absorbant la stochasticité sur plusieurs tâches ;
//! - **M-§D N17 cohérent** : chaque appel Mode C attesté et lié (`all_consistent`,
//!   `unlinked==0`, `broken==0`) ;
//! - **M-§E R0 re-prouve** : la chaîne d'essaim du vrai run LLM re-prouve (`verify_swarm`) —
//!   auditable même non rejouable ;
//! - **M-§F souveraineté** : backend **local** (loopback), aucun egress, aucune clé.
//!
//! La **logique de mesure** (conformité, agrégation, GO) est, elle, gardée en CI par un test
//! PUR ci-dessous (`conformance_scoring_is_pure_and_correct`) — backend synthétique étiqueté,
//! zéro prétention sur le LLM : il prouve que le harnais *détecterait* un effet réel, pas qu'un
//! modèle l'a. La division est nette : la structure se garde en CI, l'effet se mesure à la demande.

use serde_json::{json, Value};

const ENDPOINT: &str = "http://127.0.0.1:11434";
const MODEL: &str = "gemma3:12b";
/// Le vocabulaire de gagnants attendu — une réponse « conforme » est EXACTEMENT l'une de ces
/// étiquettes. Le few-shot doit ENSEIGNER ce vocabulaire : l'instruction ne le donne PAS (sinon le
/// modèle le devine seul et le few-shot n'a aucune marge — cf. la première mesure, Δ=0). Sans
/// exemples, le modèle emploie ses propres synonymes (« computing », « medicine », « markets »…),
/// hors vocabulaire ⇒ non conforme ; avec, il s'aligne sur la forme des gagnants passés.
const LABELS: [&str; 3] = ["technology", "finance", "health"];
/// L'instruction du générateur Mode C — VAGUE À DESSEIN (ni vocabulaire, ni casse imposés) : c'est
/// au few-shot d'enseigner la forme. En SelfHost, le « nom » de l'agent EST l'instruction LLM.
const INSTRUCTION: &str = "Identify the single most relevant topic word for this text.";
const JUDGE: &str = "Judge how well the one-word topic label fits the text.";

// ─────────────────────────────────────────────────────────────────────────────
//  Logique de mesure — PURE, gardée en CI (sans backend)
// ─────────────────────────────────────────────────────────────────────────────

/// La sortie est-elle une réponse **propre** : exactement une étiquette valide, à la casse / aux
/// espaces / au point final près ? C'est le signal du few-shot — « voici de bonnes réponses
/// passées, produis quelque chose de la même *forme* ». Verbeux ou hors-vocabulaire ⇒ non conforme.
fn is_conformant(output: &str, labels: &[&str]) -> bool {
    let o = output.trim().trim_end_matches('.').trim().to_lowercase();
    labels.contains(&o.as_str())
}

/// Taux de conformité d'un lot de sorties (milli, `[0,1000]`).
fn conformance_milli(outputs: &[String], labels: &[&str]) -> u32 {
    if outputs.is_empty() {
        return 0;
    }
    let ok = outputs.iter().filter(|o| is_conformant(o, labels)).count();
    (ok * 1000 / outputs.len()) as u32
}

#[test]
fn conformance_scoring_is_pure_and_correct() {
    // Garde-fou CI de la LOGIQUE de mesure (pas du LLM) : une étiquette propre est conforme ;
    // tolérance casse/espaces/point ; verbeux ou hors-vocabulaire = non conforme ; le taux agrège.
    assert!(is_conformant("technology", &LABELS));
    assert!(is_conformant("  Finance. ", &LABELS), "trim + casse + point final tolérés");
    assert!(!is_conformant("The topic is technology.", &LABELS), "verbeux ⇒ non conforme");
    assert!(!is_conformant("computing", &LABELS), "synonyme hors vocabulaire ⇒ non conforme");
    assert!(!is_conformant("", &LABELS), "vide ⇒ non conforme");
    let outs = vec![
        "technology".to_string(),            // ✓
        "The topic is finance.".to_string(), // ✗ (verbeux)
        "Health.".to_string(),               // ✓
        "medicine".to_string(),              // ✗ (synonyme hors vocabulaire)
    ];
    assert_eq!(conformance_milli(&outs, &LABELS), 500, "2/4 conformes");
    assert_eq!(conformance_milli(&[], &LABELS), 0, "lot vide ⇒ 0");
}

// ─────────────────────────────────────────────────────────────────────────────
//  Le harnais RÉEL — #[ignore], Ollama local
// ─────────────────────────────────────────────────────────────────────────────

/// La famille de tâches : des textes clairement mono-topique (la bonne réponse est connue), de
/// MÊME forme structurelle (un seul `{text}`) ⇒ même `FamilyKey` (FamilyMode::Structural) ⇒ une
/// seule mémoire de famille. Le few-shot doit aider la *forme* (un mot, minuscule), pas le savoir.
fn tasks() -> Vec<(Value, &'static str)> {
    // Textes prône aux SYNONYMES à froid (« hardware », « monetary policy », « medicine »…) — c'est
    // l'écart que les exemplaires resserrent vers le vocabulaire des gagnants {technology, finance, health}.
    vec![
        (json!({ "text": "The new GPU ships with 24 gigabytes of VRAM and PCIe 5.0 connectivity." }), "technology"),
        (json!({ "text": "The startup migrated its database to a distributed Kubernetes cluster." }), "technology"),
        (json!({ "text": "The central bank raised its benchmark interest rate by fifty basis points." }), "finance"),
        (json!({ "text": "Quarterly earnings beat analyst estimates, lifting the share price sharply." }), "finance"),
        (json!({ "text": "Clinical trials showed the vaccine markedly reduced hospitalizations." }), "health"),
        (json!({ "text": "The hospital rolled out a new bedside protocol for early sepsis screening." }), "health"),
    ]
}

/// Extrait l'étiquette d'une sortie de consensus (`{value: "<label>"}` ou nu).
fn winner_label(output: &Value) -> String {
    output
        .get("value")
        .and_then(|v| v.as_str())
        .map(String::from)
        .unwrap_or_else(|| output.as_str().map(String::from).unwrap_or_else(|| output.to_string()))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "nécessite un Ollama local (ollama serve + gemma3:12b)"]
async fn fewshot_influence_on_real_ollama() {
    use std::sync::Arc;

    use efe_nano_sdk::TenantId;
    use efe_swarm_memory::{
        family_key, Exemplar, FamilyMode, InMemoryMemoryStore, MemoryStore, MemoryUpdate, RunSummary,
    };
    use efe_swarm_sdk::{Budget, ClientMode, MemoryHandle, Swarm, TopologySpec};
    use uuid::Uuid;

    let tenant = TenantId::system().to_string();
    let budget = Budget::cents(100_000);
    let spec = || TopologySpec::consensus(INSTRUCTION, &[JUDGE], 3);

    // FROID : SelfHost SANS mémoire ⇒ prompt historique (aucun exemplaire, I-MEM8).
    let cold = Swarm::builder()
        .mode(ClientMode::SelfHost { endpoint: ENDPOINT.into(), model: MODEL.into() })
        .require_healthy_backend(true)
        .build()
        .expect("backend Ollama (froid)");

    // CHAUD : SelfHost AVEC une mémoire SEMÉE d'exemplaires « propres » (Mode A étiqueté : ce sont
    // des aperçus de gagnants passés à la forme idéale — un mot, minuscule). recall_only : pas de
    // repli pendant la mesure (les exemplaires restent fixes → mesure contrôlée).
    let store = Arc::new(InMemoryMemoryStore::new());
    let exemplars: Vec<Exemplar> = ["technology", "finance", "health"]
        .iter()
        .enumerate()
        .map(|(i, lbl)| Exemplar { content_hash: format!("{i:064x}"), preview: (*lbl).to_string(), quality_milli: 1000 })
        .collect();
    let warm = Swarm::builder()
        .mode(ClientMode::SelfHost { endpoint: ENDPOINT.into(), model: MODEL.into() })
        .require_healthy_backend(true)
        .memory(MemoryHandle::with_config(store.clone(), efe_swarm_sdk::MemoryConfig::recall_only()))
        .build()
        .expect("backend Ollama (chaud)");

    let (kind, agents) = spec().family_signature();
    let mut cold_outs = Vec::new();
    let mut warm_outs = Vec::new();
    let mut all_n17_consistent = true;
    let mut all_verified = true;
    let mut local_endpoint = true;
    // LIVENESS du few-shot (revue adversariale) : sans ça, un câblage cassé (exemplaires jamais
    // recall-és) donnerait warm == cold et passerait l'assertion warm ≥ cold VACUEUSEMENT. On exige
    // donc que CHAQUE run chaud ait LU un instantané semé (recalled ∧ version > 0) — l'entrée du
    // few-shot était bien vivante. (Le câblage exemplaires→prompt est gardé en CI, llm.rs::tests.)
    let mut warm_recalled = true;

    for (n, (task, _expected)) in tasks().into_iter().enumerate() {
        // FROID
        let c = cold.run_traced(task.clone(), spec(), budget).await.expect("run froid");
        cold_outs.push(winner_label(&c.run.output));

        // SEED la famille de CETTE tâche (robuste au partage structurel) puis CHAUD.
        let family = family_key(&task, &kind, &agents, FamilyMode::Structural);
        // ISOLATION du few-shot : on ne sème QUE des exemplaires. `winning_pattern: None` (pas de
        // PatternPrior) et `agents: vide` (pas de Réputation) ⇒ aucune autre lecture-retour ne peut
        // confondre la mesure. (Le spec est de toute façon Fixed(Borda), donc le biais de pattern ne
        // s'appliquerait pas — mais on rend l'isolation EXPLICITE plutôt que dépendante de ce détail.)
        let summary = RunSummary {
            clean: true,
            k_used: 3,
            winning_pattern: None,
            escalated: false,
            exemplars: exemplars.clone(),
            agents: Vec::new(),
            failed: false,
            failure_key: None,
        };
        store
            .fold(&tenant, MemoryUpdate { run_id: Uuid::from_u128(0xFE_00 + n as u128), family, summary })
            .await
            .expect("seed exemplaires");
        let w = warm.run_traced(task, spec(), budget).await.expect("run chaud");
        warm_outs.push(winner_label(&w.run.output));

        // LIVENESS : le run chaud a-t-il LU l'instantané semé (recall actif, version non-genèse) ?
        warm_recalled &= w.run.memory.as_ref().map(|m| m.recalled && m.version > 0).unwrap_or(false);

        // M-§D/E/F sur le run chaud (Mode C réel).
        if let Some(n17) = &w.run.n17 {
            all_n17_consistent &= n17.all_consistent && n17.unlinked == 0 && n17.broken == 0;
        } else {
            all_n17_consistent = false; // un run Mode C DOIT avoir un résumé N17
        }
        all_verified &= w.run.provenance.verified;
        local_endpoint &= w.run.backend.as_ref().map(|b| {
            b.endpoint.contains("127.") || b.endpoint.contains("localhost") || b.endpoint.contains("::1")
        }).unwrap_or(false);
    }

    let cold_rate = conformance_milli(&cold_outs, &LABELS);
    let warm_rate = conformance_milli(&warm_outs, &LABELS);

    // M-§B : le few-shot aide la forme OU est neutre (jamais nuit) — claim honnête, borné — ET son
    // entrée était vivante (warm_recalled), sinon l'assertion serait vacueuse (revue adversariale).
    let b_ok = warm_rate >= cold_rate && warm_recalled;
    let d_ok = all_n17_consistent; // M-§D
    let e_ok = all_verified; // M-§E
    let f_ok = local_endpoint; // M-§F
    let go = b_ok && d_ok && e_ok && f_ok;

    println!("\n M-§B INFLUENCE   : conformité chaud {warm_rate}‰ ≥ froid {cold_rate}‰ (Δ {}‰) · recall vivant={warm_recalled}  → B_ok={b_ok}", warm_rate as i32 - cold_rate as i32);
    println!("   froid : {cold_outs:?}");
    println!("   chaud : {warm_outs:?}");
    println!(" M-§D N17         : tous cohérents + liés (unlinked=0, broken=0)  → D_ok={d_ok}");
    println!(" M-§E R0          : chaîne re-prouvée (verify_swarm)  → E_ok={e_ok}");
    println!(" M-§F SOUVERAIN   : backend local (loopback), zéro clé  → F_ok={f_ok}");
    let verdict = if go {
        "GO ✅ — few-shot ≥ froid (forme) · N17 cohérent · R0 re-prouve · souverain local"
    } else {
        "NO-GO ❌"
    };
    println!("\n──────────────── VERDICT FEW-SHOT MODE C (M-§B…F, Ollama réel) ────────────────");
    println!(" {verdict}");
    println!("───────────────────────────────────────────────────────────────────────────────");

    let payload = json!({
        "_note": "Few-shot Mode C mesuré sur un VRAI modèle (Ollama gemma3:12b local). Froid (sans mémoire) vs chaud (exemplaires propres semés via le vrai fold, Mode A étiqueté). Conformité = sortie EXACTEMENT une étiquette valide (forme propre). Claim honnête et borné : warm ≥ cold (le few-shot aide la forme ou est neutre, jamais ne nuit) ; la stochasticité est absorbée sur plusieurs tâches. N17/R0/souveraineté pris des runs chauds. On ne SIMULE pas le LLM : la logique de mesure est gardée en CI (test pur), l'effet est mesuré ici à la demande.",
        "model": MODEL, "endpoint": ENDPOINT, "labels": LABELS, "tasks": cold_outs.len(),
        "M_B_influence": { "cold_conformance_milli": cold_rate, "warm_conformance_milli": warm_rate, "delta_milli": warm_rate as i32 - cold_rate as i32, "warm_recall_live": warm_recalled, "cold_outputs": cold_outs, "warm_outputs": warm_outs, "ok": b_ok },
        "M_D_n17_consistent": d_ok,
        "M_E_r0_reproves": e_ok,
        "M_F_sovereign_local": f_ok,
        "verdict": verdict, "go": go
    });
    // En `cargo test`, le CWD est le dossier du CRATE (pas la racine workspace, contrairement aux
    // exemples `cargo run`) : on vise la racine via CARGO_MANIFEST_DIR pour ranger l'artefact au même
    // endroit que les autres gates (`scripts/swarm/`).
    let dir = format!("{}/../../scripts/swarm", env!("CARGO_MANIFEST_DIR"));
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(format!("{dir}/EFE_SWARM_FEWSHOT_MODEC_2026-06-18.json"), serde_json::to_string_pretty(&payload).unwrap());

    assert!(go, "GATE Mode C few-shot (M-§B…F) : few-shot ≥ froid, N17 cohérent, R0 re-prouve, souverain");
}
