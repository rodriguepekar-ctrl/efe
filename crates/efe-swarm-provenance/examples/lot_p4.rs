// SPDX-License-Identifier: UNLICENSED
//! # `lot_p4` — le **GATE** de Phase 4 : la provenance par-agent paie-t-elle son coût ?
//!
//! Question (§7) : un enregistrement par-agent atteint-il **100 % de couverture** +
//! **rejeu déterministe** + **vérification tierce**, à **overhead < budget** ?
//!
//! Harnais HONNÊTE (même doctrine que Lot S2/S3) : **crypto RÉELLE** (BLAKE3 + SHA-256 +
//! HMAC + HKDF) + **topologie RÉELLE** (`ConsensusTopology`) ; seuls les **agents** sont
//! simulés. On compare deux bras — **sans** provenance vs **avec** — et on mesure :
//! - overhead de latence (ns/run et ns/agent ; relatif %, avec sa mise en garde) ;
//! - octets par enregistrement ;
//! - `traced_pct`, déterminisme du `provenance_root`, `verify_swarm`, falsification.
//!
//! GO ⇔ tracé 100 % ∧ déterministe ∧ vérif OK ∧ falsification détectée ∧ overhead sous budget.
//!
//! Run :  cargo run --release --example lot_p4 -p efe-swarm-provenance

use std::fs;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use efe_blackboard::WriteSink;
use efe_nano_sdk::{AgentId, Blake3Hash, SwarmId, TenantId};
use efe_swarm_provenance::{
    replay_root, verify_swarm, HmacKeySource, ProvenanceMetrics, ProvenanceRecorder,
    ProvenanceSink, ProvenancedAgent, SealedProvenance,
};
use efe_swarm_topology::{ConsensusTopology, Generator, Judge, Pattern, SwarmContext, Topology};
use serde_json::{json, Value};

const RUNS: usize = 200;
const WARMUP: usize = 20;
const K: usize = 7;
const JN: usize = 5;
// budgets figés AU LANCEMENT (§7).
const OVERHEAD_NS_PER_AGENT_BUDGET: f64 = 100_000.0; // 100 µs / agent
const BYTES_PER_RECORD_BUDGET: f64 = 2048.0; // 2 Ko / enregistrement

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

fn mean_std(xs: &[f64]) -> (f64, f64) {
    let n = xs.len().max(1) as f64;
    let mean = xs.iter().sum::<f64>() / n;
    let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
    (mean, var.sqrt())
}

/// Un run de la VRAIE ConsensusTopology, SANS provenance. Renvoie la latence (ns).
async fn run_bare(task: &Value, ctx: &SwarmContext) -> f64 {
    let gen: Arc<dyn Generator> = Arc::new(SimGen);
    let judges: Vec<Arc<dyn Judge>> =
        (0..JN).map(|j| Arc::new(EqJudge(format!("judge_{j:03}"))) as Arc<dyn Judge>).collect();
    let topo = ConsensusTopology::new(gen, K, judges).with_pattern(Pattern::Borda);
    let t0 = Instant::now();
    let _ = topo.run(task, ctx).await;
    t0.elapsed().as_nanos() as f64
}

/// Un run AVEC provenance (décorateur + sink + scellement). Renvoie (latence ns, sealed).
async fn run_prov(task: &Value, ctx: &SwarmContext, key: &[u8]) -> (f64, SealedProvenance) {
    let recorder = Arc::new(ProvenanceRecorder::new(SwarmId::new(), TenantId::system()));
    let sink: Arc<dyn WriteSink> = Arc::new(ProvenanceSink::new(recorder.clone()));
    let gen: Arc<dyn Generator> =
        Arc::new(ProvenancedAgent::mode_a(SimGen, recorder.clone(), AgentId::new("generator").unwrap()));
    let judges: Vec<Arc<dyn Judge>> = (0..JN)
        .map(|j| {
            Arc::new(ProvenancedAgent::mode_c(
                EqJudge(format!("judge_{j:03}")),
                recorder.clone(),
                AgentId::new(format!("judge_{j:03}")).unwrap(),
            )) as Arc<dyn Judge>
        })
        .collect();
    let topo = ConsensusTopology::new(gen, K, judges).with_pattern(Pattern::Borda).with_write_sink(sink);

    let t0 = Instant::now();
    let _ = topo.run(task, ctx).await;
    let sealed = recorder.seal_with_key(key.to_vec());
    let elapsed = t0.elapsed().as_nanos() as f64;
    (elapsed, sealed)
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    let ctx = SwarmContext::standard();
    let task = json!({ "expected": "tech", "n_correct": 3, "difficulty": 0.7 });
    let tenant = TenantId::system();
    let key = HmacKeySource::test_key().derive(tenant).unwrap();
    let metrics = ProvenanceMetrics::new();

    println!("══════════════════════════════════════════════════════════════════════════════");
    println!(" LOT P4 — gate Phase 4 : la provenance par-agent paie-t-elle son coût ?");
    println!(" crypto + topologie RÉELLES · agents simulés · K={K} J={JN} · {RUNS} runs/bras");
    println!("══════════════════════════════════════════════════════════════════════════════");

    // warmup (chauffe caches + JIT du runtime).
    for _ in 0..WARMUP {
        let _ = run_bare(&task, &ctx).await;
        let _ = run_prov(&task, &ctx, &key).await;
    }

    // mesure.
    let mut bare_ns = Vec::with_capacity(RUNS);
    let mut prov_ns = Vec::with_capacity(RUNS);
    let mut last: Option<SealedProvenance> = None;
    for _ in 0..RUNS {
        bare_ns.push(run_bare(&task, &ctx).await);
        let (e, sealed) = run_prov(&task, &ctx, &key).await;
        prov_ns.push(e);
        metrics.observe_sealed(&sealed);
        for r in &sealed.records {
            metrics.observe_bytes(serde_json::to_string(r).unwrap().len());
        }
        last = Some(sealed);
    }

    let (mean_bare, std_bare) = mean_std(&bare_ns);
    let (mean_prov, std_prov) = mean_std(&prov_ns);
    let overhead_run = (mean_prov - mean_bare).max(0.0);
    let overhead_agent = overhead_run / (K + JN) as f64;
    let relative_pct = if mean_bare > 0.0 { 100.0 * overhead_run / mean_bare } else { 0.0 };
    metrics.observe_overhead_ns(overhead_run as u64);
    let mean_bytes = metrics.mean_bytes();

    // intégrité sur le dernier run : tracé + déterminisme + vérif + falsification.
    let sealed = last.expect("au moins un run");
    let report = verify_swarm(&sealed.records, &key);
    metrics.observe_verify(&report);
    let traced_pct = report.traced_pct;
    let deterministic = sealed.root.is_some() && replay_root(&sealed.records, &key) == sealed.root;

    let mut tampered = sealed.records.clone();
    tampered[0].output_hash = Blake3Hash::from_bytes([0xff; 32]);
    let tamper = verify_swarm(&tampered, &key);
    let falsification_detected = !tamper.ok() && tamper.first_break.is_some();

    let go = (traced_pct - 1.0).abs() < 1e-9
        && deterministic
        && report.ok()
        && falsification_detected
        && overhead_agent < OVERHEAD_NS_PER_AGENT_BUDGET
        && mean_bytes < BYTES_PER_RECORD_BUDGET;

    // ── rapport ─────────────────────────────────────────────────────────────────
    let us = |ns: f64| ns / 1000.0;
    println!("\n LATENCE (µs/run) :");
    println!("   sans provenance : {:.1} ± {:.1}", us(mean_bare), us(std_bare));
    println!("   avec provenance : {:.1} ± {:.1}", us(mean_prov), us(std_prov));
    println!("   overhead/run    : {:.1} µs  ·  overhead/agent : {:.2} µs  (budget {:.0} µs)", us(overhead_run), us(overhead_agent), us(OVERHEAD_NS_PER_AGENT_BUDGET));
    println!("   relatif         : {relative_pct:.0} %  ⚠ surévalué : agents simulés ≈ travail nul ;");
    println!("                     le chiffre RÉEL est l'ABSOLU ({:.2} µs/agent), négligeable vs", us(overhead_agent));
    println!("                     tout agent réel (Mode A travail réel, ou Mode C en ms).");
    println!("\n COUVERTURE & INTÉGRITÉ :");
    println!("   octets/enregistrement : {mean_bytes:.0} o   (budget {BYTES_PER_RECORD_BUDGET:.0} o)");
    println!("   traced_pct            : {:.0} %", 100.0 * traced_pct);
    println!("   rejeu déterministe    : {deterministic}");
    println!("   verify_swarm (sain)   : ok={} · chaîne={} · contenu={} · DAG={}", report.ok(), report.chain_intact, report.content_intact, report.dag_consistent);
    println!("   falsification détectée: {falsification_detected}");

    let verdict = if go {
        format!(
            "GO — provenance par-agent : 100 % tracé, déterministe, re-prouvée ; overhead {:.2} µs/agent (< {:.0} µs) et {mean_bytes:.0} o/record (< {BYTES_PER_RECORD_BUDGET:.0}). Le moat paie son coût.",
            us(overhead_agent), us(OVERHEAD_NS_PER_AGENT_BUDGET)
        )
    } else {
        "NO-GO — un critère sous le seuil ; escalades : sink async, échantillonnage Mode A, compaction Merkle (cf. ADR-SWARM-006).".to_string()
    };
    println!("\n────────────────────────── VERDICT LOT P4 ──────────────────────────");
    println!(" {verdict}");
    println!("─────────────────────────────────────────────────────────────────────");

    println!("\n [extrait Prometheus]");
    for line in metrics.render_prometheus().lines().filter(|l| !l.starts_with('#')) {
        println!("   {line}");
    }

    // ── persiste les résultats (format des Lots S2/S3). ──────────────────────────
    let payload = json!({
        "_note": "Agents simulés ; crypto (BLAKE3+SHA256+HMAC+HKDF) + ConsensusTopology RÉELLES.",
        "runs": RUNS, "k": K, "j": JN,
        "latency_us": {
            "bare":  [us(mean_bare), us(std_bare)],
            "prov":  [us(mean_prov), us(std_prov)],
            "overhead_per_run": us(overhead_run),
            "overhead_per_agent": us(overhead_agent),
            "relative_pct": relative_pct,
            "_caveat": "relatif surévalué (agents simulés ≈ travail nul) ; l'ABSOLU/agent est la mesure réelle"
        },
        "bytes_per_record": mean_bytes,
        "traced_pct": traced_pct,
        "deterministic": deterministic,
        "verify_ok": report.ok(),
        "falsification_detected": falsification_detected,
        "budgets": { "overhead_us_per_agent": us(OVERHEAD_NS_PER_AGENT_BUDGET), "bytes_per_record": BYTES_PER_RECORD_BUDGET },
        "metrics": metrics.snapshot(),
        "verdict": verdict,
    });
    let _ = fs::create_dir_all("scripts/swarm");
    let path = "scripts/swarm/EFE_SWARM_P4_2026-06-09.json";
    let _ = fs::write(path, serde_json::to_string_pretty(&payload).unwrap());
    println!("\n✅ Résultats : {path}");
}
