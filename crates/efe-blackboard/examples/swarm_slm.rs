// SPDX-License-Identifier: UNLICENSED
//! # `swarm_slm` — l'essaim M50, mais chaque agent fait une VRAIE inférence SLM.
//!
//! Variante de `swarm_relay` où le travail trivial (comptage de mots) est
//! remplacé par un appel au **trait `LlmClient` réel** de `efe-nano` (le chemin
//! Mode C). Backend par défaut : `BuiltinLlmClient` — le « substitut de
//! production » que le repo ship lui-même : déterministe, **keyless, sans
//! serveur**, exécutable partout. Le seam pour brancher un vrai SLM réseau
//! (llama-server / `LlmRouterClient`) est documenté plus bas.
//!
//! Run :
//! ```text
//! cargo run --example swarm_slm -p efe-blackboard            # 24 classifieurs ∥ 4 tallies
//! cargo run --example swarm_slm -p efe-blackboard -- 80 6    # 80 classifieurs ∥ 6 tallies
//! ```
//!
//! ## Ce que ça prouve EN PLUS de `swarm_relay`
//!
//! - **Petits agents = vrais SLM** : chaque classifieur appelle
//!   `LlmClient::call(m50.classify_topic.v1)` → `{topic, confidence}`. C'est le
//!   chemin exact qu'utilise `LlmAugmentedRuntime` (Mode C).
//! - **L'info qui circule entre agents est une SORTIE STRUCTURÉE de modèle**
//!   (verdict + confiance), pas un comptage. Le passage se fait toujours par
//!   `Blackboard` + `EventBus`.
//! - **Consensus émergent pondéré** : aucun agent ne décide du topic global ;
//!   il émerge de la somme des confiances, agrégée par work-stealing (CAS).
//! - **Comptabilité SLM réelle** : coût (cents) + tokens, sommés sur l'essaim.
//!
//! ## Brancher un VRAI SLM réseau (llama-server)
//!
//! Le backend est choisi par variable d'environnement :
//! ```text
//! EFE_SWARM_LLM=builtin                                   # défaut : keyless, sans serveur
//! EFE_SWARM_LLM=llama  EFE_SWARM_LLM_URL=http://127.0.0.1:8090   # vrai SLM réseau
//! ```
//! Le backend `llama` parle le **vrai contrat `/completion`** de llama-server
//! (b9297+) : `{prompt, n_predict, temperature, cache_prompt, stop, grammar}` →
//! `{content, tokens_evaluated, tokens_predicted}`, avec une **grammaire GBNF**
//! qui force la sortie `{"topic","confidence"}`. Calqué sur le `LlamaServerClient`
//! réel de `efe-nano-app-coordinator`. Pas de clé (serveur local).
//!
//! Si `llama` est demandé mais le serveur est injoignable (`/health` KO), la
//! démo **bascule automatiquement sur `builtin`** (même esprit keyless/real que
//! le PoC) pour rester exécutable. Le reste de l'essaim est STRICTEMENT inchangé.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use efe_blackboard::{Blackboard, BlackboardError, EventBus, SwarmEvent};
use efe_nano::error::LlmCallError;
use efe_nano::runtime::llm_client::{
    BuiltinLlmClient, LlmCallRequest, LlmCallResponse, LlmClient,
};
use efe_nano_sdk::{AgentId, SwarmId, TenantId};
use serde_json::json;

// ─────────────────────────────────────────────────────────────────────────────
//  Backend RÉSEAU : implémente le trait `LlmClient` par-dessus llama-server.
//  Calqué sur le contrat /completion réel (efe-nano-app-coordinator).
// ─────────────────────────────────────────────────────────────────────────────

/// GBNF : force la réponse en JSON `{"topic": <enum>, "confidence": <0..1>}`.
const TOPIC_GRAMMAR: &str = r#"root   ::= "{" ws "\"topic\":" ws "\"" topic "\"" ws "," ws "\"confidence\":" ws conf ws "}"
topic  ::= "technology" | "finance" | "health" | "legal" | "politics" | "sports" | "travel" | "general"
conf   ::= "0." [0-9] [0-9]? | "1.0" | "0" | "1"
ws     ::= [ \t\n]*"#;

const TOPIC_SYSTEM: &str = "You classify the user's text into exactly one topic among: \
technology, finance, health, legal, politics, sports, travel, general. \
Reply ONLY with JSON: {\"topic\": <one topic>, \"confidence\": <0..1>}.";

/// Client SLM réseau : `POST {url}/completion` vers llama-server.
struct LlamaSwarmClient {
    http: reqwest::Client,
    url: String,
    max_tokens: u32,
    temperature: f32,
}

impl LlamaSwarmClient {
    fn new(url: String) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(20))
                .pool_max_idle_per_host(16)
                .build()
                .expect("build reqwest client"),
            url,
            max_tokens: 48,
            temperature: 0.0, // greedy = déterministe (cf. Correctif A)
        }
    }

    /// Probe `/health` : true si llama-server répond 2xx.
    async fn healthy(&self) -> bool {
        let u = format!("{}/health", self.url);
        matches!(
            tokio::time::timeout(Duration::from_secs(2), self.http.get(&u).send()).await,
            Ok(Ok(r)) if r.status().is_success()
        )
    }
}

#[async_trait]
impl LlmClient for LlamaSwarmClient {
    async fn call(&self, request: LlmCallRequest) -> Result<LlmCallResponse, LlmCallError> {
        // Cette démo ne câble que le template de classification.
        if request.prompt_template_id != "m50.classify_topic.v1" {
            return Err(LlmCallError::PromptNotFound(request.prompt_template_id));
        }
        let text = request
            .template_inputs
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let prompt = format!("{TOPIC_SYSTEM}\n\nText: {text}\nJSON:");

        let body = json!({
            "prompt": prompt,
            "n_predict": self.max_tokens,
            "temperature": self.temperature,
            "cache_prompt": true,
            "stop": ["\n\n\n"],
            "grammar": TOPIC_GRAMMAR,
        });

        let url = format!("{}/completion", self.url);
        let resp = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmCallError::Provider(format!("transport: {e}")))?;
        if !resp.status().is_success() {
            return Err(LlmCallError::Provider(format!("http {}", resp.status())));
        }
        let raw: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| LlmCallError::Provider(format!("decode: {e}")))?;
        let content = raw.get("content").and_then(|v| v.as_str()).unwrap_or("");
        let parsed: serde_json::Value = serde_json::from_str(content.trim())
            .map_err(|e| LlmCallError::Provider(format!("schema: {e} (raw={content:?})")))?;

        let tokens_in = raw.get("tokens_evaluated").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let tokens_out = raw.get("tokens_predicted").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        Ok(LlmCallResponse {
            content: parsed,
            tokens_in,
            tokens_out,
            // Coût indicatif d'un SLM local (≈ électricité) : 0.02 cent/appel.
            cost_cents: 0.02,
            provider_name: "llama-server".to_string(),
            model_name: "local-slm".to_string(),
            cache_hit: false,
            duration_ms: 0,
            retry_attempts: 0,
        })
    }

    fn estimate_cost_cents(&self, _request: &LlmCallRequest) -> f64 {
        0.02
    }
}

/// Choisit le backend SLM selon l'env, avec bascule auto si llama-server est KO.
async fn build_client() -> (Arc<dyn LlmClient>, String) {
    let backend = std::env::var("EFE_SWARM_LLM").unwrap_or_else(|_| "builtin".to_string());
    if backend == "llama" {
        let url = std::env::var("EFE_SWARM_LLM_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8090".to_string());
        let client = LlamaSwarmClient::new(url.clone());
        if client.healthy().await {
            return (Arc::new(client), format!("llama-server @ {url} [réseau, greedy]"));
        }
        eprintln!(
            "⚠  EFE_SWARM_LLM=llama mais {url}/health injoignable → bascule sur builtin.\n   \
             (Démarrez llama-server : ./scripts/slm/download_models.sh puis le serveur sur :8090.)"
        );
    }
    (Arc::new(BuiltinLlmClient::new()), "builtin [keyless, sans serveur]".to_string())
}

/// Corpus volontairement *biaisé vers la technologie* : on veut qu'un consensus
/// clair ÉMERGE de l'essaim (et pas une égalité). Les mots-clés correspondent
/// au classifieur builtin (technology / finance / health / legal / politics …).
const CORPUS: &[&str] = &[
    "the new ai software ships faster code to the internet",        // technology
    "a computer running ai code on the internet",                   // technology
    "software engineers write code for the ai computer",            // technology
    "ai and software power the modern internet computer",           // technology
    "the stock market and trading drive investment returns",        // finance
    "doctor and patient discuss the medical treatment",             // health
    "the court judge reviewed the contract under law",              // legal
    "ai software code computer internet machine learning",          // technology
    "an election and government policy split the vote",             // politics
    "the team player scored in the championship match game",        // sports
    "internet ai software code computer cloud",                     // technology
    "ai computer software internet code deploy",                    // technology
];

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let n_class: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(24);
    let n_tally: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(4);

    let swarm_id = SwarmId::new();
    let bb = Arc::new(Blackboard::new(swarm_id));
    let bus = Arc::new(EventBus::with_capacity((n_class * 4).max(256)));

    // ── LE SLM : trait `LlmClient` réel. Backend choisi par env (builtin/llama). ─
    let (client, backend_desc) = build_client().await;
    let tenant = TenantId::system();

    println!("══════════════════════════════════════════════════════════════");
    println!(" ESSAIM SLM M50 — {n_class} classifieurs ∥ {n_tally} tallies ∥ aggregator ∥ tracer");
    println!(" swarm_id = {swarm_id}");
    println!(" agent SLM = LlmClient::call(m50.classify_topic.v1)");
    println!(" backend   = {backend_desc}");
    println!(" comm      = EventBus (broadcast) + Blackboard (CAS versionné)");
    println!("══════════════════════════════════════════════════════════════\n");

    // ── TRACER ───────────────────────────────────────────────────────────────
    let tracer = {
        let mut sub = bus.subscribe();
        tokio::spawn(async move {
            let mut seen = 0u64;
            while let Some(ev) = sub.recv().await {
                seen += 1;
                if ev.topic == "swarm_complete" {
                    break;
                }
                if ev.topic != "classified" || seen <= 12 || seen % 13 == 0 {
                    println!(
                        "   ┊ [{:>4}] {:<13} ⇒ {:<10} {}",
                        seen,
                        ev.published_by,
                        ev.topic,
                        compact(&ev.payload),
                    );
                }
            }
            seen
        })
    };

    // ── AGGREGATOR : détecte la fin, libère les tallies, fusionne le consensus ─
    let aggregator = {
        let mut sub = bus.subscribe();
        let bus = bus.clone();
        let bb = bb.clone();
        let agg = AgentId::new("aggregator").unwrap();
        tokio::spawn(async move {
            use std::collections::BTreeSet;
            let mut done: BTreeSet<String> = BTreeSet::new();
            while let Some(ev) = sub.recv().await {
                if ev.topic == "classified" {
                    if let Some(a) = ev.payload.get("agent").and_then(|v| v.as_str()) {
                        done.insert(a.to_string());
                    }
                    if done.len() == n_class {
                        break;
                    }
                }
            }
            let _ = bus.publish(mk(swarm_id, &agg, "shutdown", json!({})));

            // Attend les R partiels /tally/<r> (poll coopératif).
            loop {
                let n = bb
                    .snapshot()
                    .fields
                    .keys()
                    .filter(|k| k.starts_with("/tally/"))
                    .count();
                if n >= n_tally {
                    break;
                }
                tokio::task::yield_now().await;
            }

            // CONSENSUS PONDÉRÉ ÉMERGENT : somme des confiances par topic.
            let mut weighted: BTreeMap<String, f64> = BTreeMap::new();
            for (k, v) in bb.snapshot().fields {
                if !k.starts_with("/tally/") {
                    continue;
                }
                if let Some(obj) = v.value.as_object() {
                    for (topic, w) in obj {
                        *weighted.entry(topic.clone()).or_default() += w.as_f64().unwrap_or(0.0);
                    }
                }
            }
            weighted
        })
    };

    // ── TALLIES (reducers) : work-stealing CAS sur les verdicts SLM ──────────
    let mut tally_tasks = Vec::new();
    for r in 0..n_tally {
        let mut sub = bus.subscribe();
        let bb = bb.clone();
        let rid = AgentId::new(format!("tally_{r}")).unwrap();
        tally_tasks.push(tokio::spawn(async move {
            let mut local: BTreeMap<String, f64> = BTreeMap::new();
            let mut won = 0u64;
            let mut lost = 0u64;
            while let Some(ev) = sub.recv().await {
                match ev.topic.as_str() {
                    "classified" => {
                        let Some(agent) = ev.payload.get("agent").and_then(|v| v.as_str()) else {
                            continue;
                        };
                        match bb.write_if(&format!("/claim/{agent}"), 0, json!(rid.as_str()), rid.clone()) {
                            Ok(_) => {
                                won += 1;
                                // Lit le verdict SLM publié par le classifieur.
                                if let Some(bv) = bb.read(&format!("/verdict/{agent}")) {
                                    let topic = bv.value.get("topic").and_then(|v| v.as_str()).unwrap_or("general");
                                    let conf = bv.value.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0);
                                    *local.entry(topic.to_string()).or_default() += conf;
                                }
                            }
                            Err(BlackboardError::Conflict { .. }) => lost += 1,
                            Err(_) => {}
                        }
                    }
                    "shutdown" => break,
                    _ => {}
                }
            }
            let _ = bb.write(&format!("/tally/{r}"), json!(local), rid.clone());
            println!(
                "   ✓ {:<13} a agrégé {won:>3} verdicts SLM (perdu {lost:>3} courses CAS)",
                rid.to_string()
            );
            (won, lost)
        }));
    }

    // ── CLASSIFIEURS (mappers SLM) : le gros de l'essaim, tous en parallèle ──
    let mut class_tasks = Vec::new();
    for m in 0..n_class {
        let bb = bb.clone();
        let bus = bus.clone();
        let client = client.clone();
        let id = AgentId::new(format!("classifier_{m:02}")).unwrap();
        let text = CORPUS[m % CORPUS.len()].to_string();
        class_tasks.push(tokio::spawn(async move {
            // VRAIE INFÉRENCE via le trait LlmClient (chemin Mode C).
            let req = LlmCallRequest::builder(tenant, "m50.classify_topic.v1")
                .template_inputs(json!({ "text": text }))
                .max_tokens(64)
                .build();
            let (topic, conf, cost, tok_out) = match client.call(req).await {
                Ok(resp) => (
                    resp.content.get("topic").and_then(|v| v.as_str()).unwrap_or("general").to_string(),
                    resp.content.get("confidence").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    resp.cost_cents,
                    resp.tokens_out as u64,
                ),
                Err(_) => ("general".to_string(), 0.0, 0.0, 0),
            };
            // Passe le verdict structuré à l'essaim : blackboard + bus.
            let _ = bb.write(
                &format!("/verdict/{id}"),
                json!({ "topic": topic, "confidence": conf }),
                id.clone(),
            );
            let _ = bus.publish(mk(
                swarm_id,
                &id,
                "classified",
                json!({ "agent": id.as_str(), "topic": topic, "confidence": conf }),
            ));
            (cost, tok_out)
        }));
    }

    // ── Join ──────────────────────────────────────────────────────────────────
    let mut total_cost = 0.0f64;
    let mut total_tokens = 0u64;
    for t in class_tasks {
        let (c, tok) = t.await.unwrap_or((0.0, 0));
        total_cost += c;
        total_tokens += tok;
    }
    let mut total_won = 0u64;
    let mut total_lost = 0u64;
    for t in tally_tasks {
        let (w, l) = t.await.unwrap_or((0, 0));
        total_won += w;
        total_lost += l;
    }
    let weighted = aggregator.await.unwrap_or_default();

    let closer = AgentId::new("aggregator").unwrap();
    let _ = bus.publish(mk(swarm_id, &closer, "swarm_complete", json!({})));
    let traced = tracer.await.unwrap_or(0);

    // ── Rapport ────────────────────────────────────────────────────────────────
    let mut ranked: Vec<(String, f64)> = weighted.into_iter().collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    println!("\n──────────────────── CONSENSUS SLM ÉMERGENT ────────────────────");
    if let Some((winner, score)) = ranked.first() {
        println!(" ▶ TOPIC GAGNANT (émergé de l'essaim) : {winner}  (poids {score:.2})");
    }
    println!(" Distribution pondérée (somme des confiances) :");
    for (topic, w) in ranked.iter().take(8) {
        let bar = "█".repeat(((*w) * 2.0).round() as usize);
        println!("   {topic:<12} {w:>6.2}  {bar}");
    }

    println!("\n──────────────────────── PREUVE DE TRAFIC ──────────────────────");
    println!(" agents totaux         : {}", n_class + n_tally + 2);
    println!(" inférences SLM        : {n_class}  (1 par classifieur, en parallèle)");
    println!(" verdicts agrégés (CAS): {total_won}  ·  courses CAS perdues : {total_lost}");
    println!(" coût SLM total        : {total_cost:.4} cents  ·  tokens_out cumulés : {total_tokens}");
    println!(" événements publiés    : {}", bus.published_total());
    println!(" événements vus/tracer : {traced}");
    println!(" écritures blackboard  : {}", bb.write_count());
    println!("══════════════════════════════════════════════════════════════════");
    println!(
        "\n→ Chaque petit agent a fait une vraie inférence (trait LlmClient, Mode C).\n  \
         Les verdicts ont circulé agent→agent via bus + blackboard ; le topic\n  \
         consensus a ÉMERGÉ de la somme pondérée, sans chef. Essaim de SLM."
    );
}

// ─────────────────────────── helpers ────────────────────────────────────────

fn mk(swarm_id: SwarmId, by: &AgentId, topic: &str, payload: serde_json::Value) -> SwarmEvent {
    SwarmEvent {
        swarm_id,
        published_by: by.clone(),
        topic: topic.to_string(),
        payload,
        published_at: SystemTime::now(),
    }
}

fn compact(v: &serde_json::Value) -> String {
    let s = v.to_string();
    if s.len() > 52 {
        format!("{}…", &s[..51])
    } else {
        s
    }
}
