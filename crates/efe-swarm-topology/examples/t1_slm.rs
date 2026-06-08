// SPDX-License-Identifier: UNLICENSED
//! # `t1_slm` — T1 Generate-Verify avec générateurs Mode C réels (`LlmClient`).
//!
//! Démontre la topologie [`GenerateVerify`] productisée, branchée sur un VRAI
//! `LlmClient` (Mode C, keyless `BuiltinLlmClient`) : K générateurs produisent des
//! classifications, un vérificateur déterministe (oracle = plafond OR) sélectionne
//! a-posteriori. On compare T1 au VOTE majoritaire (le mécanisme du gate Lot S2).
//!
//! Diversité injectée (fallibilité simulée par variant) pour le run keyless ; en
//! réel elle vient de l'échantillonnage du modèle. Le vérificateur « oracle » =
//! ce qu'atteint un vérificateur DÉTERMINISTE sur une vraie tâche vérifiable
//! (check intrinsèque : tests/schéma), pas le label.
//!
//! Run :
//! ```text
//! cargo run --example t1_slm -p efe-swarm-topology            # K=7
//! cargo run --example t1_slm -p efe-swarm-topology -- 9
//! ```

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use efe_nano::runtime::llm_client::{BuiltinLlmClient, LlmCallRequest, LlmClient};
use efe_nano_sdk::TenantId;
use efe_swarm_topology::{GenerateVerify, Generator, Verifier};
use serde_json::{json, Value};

const TOPICS: &[&str] = &["technology", "finance", "health", "legal", "politics", "sports"];

/// Corpus étiqueté (texte, topic attendu) — tâche « vérifiable » par oracle.
const CORPUS: &[(&str, &str)] = &[
    ("the new ai software ships faster code to the internet", "technology"),
    ("the stock market and trading drive investment returns", "finance"),
    ("doctor and patient discuss the medical treatment plan", "health"),
    ("the court judge reviewed the contract under the law", "legal"),
    ("an election and government policy split the vote", "politics"),
    ("the team player scored in the championship match", "sports"),
    ("software engineers write code for the ai computer", "technology"),
    ("bond yields and the market shape trading strategy", "finance"),
];

/// Générateur Mode C : appelle le SLM, avec fallibilité simulée par variant (keyless).
struct LlmGenerator {
    client: Arc<dyn LlmClient>,
    tenant: TenantId,
}

#[async_trait]
impl Generator for LlmGenerator {
    async fn generate(&self, task: &Value, variant: usize) -> Value {
        let text = task.get("text").and_then(|v| v.as_str()).unwrap_or("");
        let req = LlmCallRequest::builder(self.tenant.clone(), "m50.classify_topic.v1")
            .template_inputs(json!({ "text": text }))
            .max_tokens(48)
            .build();
        let model_topic = match self.client.call(req).await {
            Ok(r) => r
                .content
                .get("topic")
                .and_then(|v| v.as_str())
                .unwrap_or("technology")
                .to_string(),
            Err(_) => "technology".to_string(),
        };
        // Diversité keyless : une MAJORITÉ de variants se trompe, sur le MÊME leurre
        // (corrélé) → le leurre devient majoritaire → le vote échoue, T1 gagne.
        let topic = if is_wrong(variant) {
            lure_topic(&model_topic, text)
        } else {
            model_topic
        };
        json!({ "topic": topic })
    }
}

/// Vérificateur ORACLE (= plafond OR) : passe ssi le topic == attendu.
struct OracleVerifier;
impl Verifier for OracleVerifier {
    fn score(&self, task: &Value, candidate: &Value) -> f64 {
        let exp = task.get("expected").and_then(|v| v.as_str()).unwrap_or("");
        let got = candidate.get("topic").and_then(|v| v.as_str()).unwrap_or("");
        if got == exp {
            1.0
        } else {
            0.0
        }
    }
    fn passes(&self, task: &Value, candidate: &Value) -> bool {
        self.score(task, candidate) >= 0.5
    }
}

/// MAJORITÉ des variants (sauf le 0) se trompent → leurre majoritaire (vote KO).
/// Pour K=7 : variants 1-4 faux (4), variants 0,5,6 corrects (3) → vote→leurre.
fn is_wrong(variant: usize) -> bool {
    let v = variant % 7;
    v >= 1 && v <= 4
}

/// Un seul leurre COMMUN par texte (corrélé entre variants) → peut former majorité.
fn lure_topic(correct: &str, text: &str) -> String {
    let h = text
        .bytes()
        .fold(0u64, |a, b| (a ^ b as u64).wrapping_mul(0x100000001B3));
    let alt: Vec<&&str> = TOPICS.iter().filter(|t| **t != correct).collect();
    alt[(h as usize) % alt.len()].to_string()
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let k: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(7);

    let gen: Arc<dyn Generator> = Arc::new(LlmGenerator {
        client: Arc::new(BuiltinLlmClient::new()),
        tenant: TenantId::system(),
    });
    let ver = OracleVerifier;
    let t1 = GenerateVerify::new(k, 4);

    println!("══════════════════════════════════════════════════════════════════");
    println!(" T1 Generate-Verify · K={k} générateurs Mode C (LlmClient) · vérificateur oracle");
    println!("══════════════════════════════════════════════════════════════════");

    let (mut t1_ok, mut vote_ok) = (0usize, 0usize);
    for (text, expected) in CORPUS {
        let task = json!({ "text": text, "expected": expected });
        let r = t1.run(&task, gen.clone(), &ver).await;
        let sel = r.selected.as_ref().and_then(|c| c["topic"].as_str()).unwrap_or("?");

        // vote majoritaire (contraste).
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for c in &r.candidates {
            if let Some(t) = c["topic"].as_str() {
                *counts.entry(t).or_default() += 1;
            }
        }
        let vote = counts.iter().max_by_key(|(_, n)| **n).map(|(t, _)| *t).unwrap_or("?");

        if sel == *expected {
            t1_ok += 1;
        }
        if vote == *expected {
            vote_ok += 1;
        }
        println!(
            " attendu={:<11} T1={:<11} vote={:<11} (passants {}/{})",
            expected, sel, vote, r.passed, r.candidates.len()
        );
    }
    let n = CORPUS.len();
    println!("──────────────────────────────────────────────────────────────────");
    println!(" T1 (verifier) : {t1_ok}/{n}   ·   vote majoritaire : {vote_ok}/{n}");
    println!("──────────────────────────────────────────────────────────────────");
    println!("→ T1 extrait la bonne réponse via le vérificateur a-posteriori là où le");
    println!("  vote échoue (leurres majoritaires). Topologie productisée (crate dédié).");
    println!("══════════════════════════════════════════════════════════════════");
}
