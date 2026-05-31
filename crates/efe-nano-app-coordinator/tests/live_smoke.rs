// SPDX-License-Identifier: UNLICENSED
//! Tests live — frappent un vrai `llama-server`.
//!
//! Gates par env vars (skippés en CI par défaut).  Pour activer :
//!
//! ```bash
//! cd /Users/rodrigue.pekar/Downloads/EFE_PROJET/03_a_construire
//! ./scripts/start-nano-robots.sh    # démarre les 4 llama-server
//! cd "/Users/rodrigue.pekar/Downloads/vrai/jjjj copie/efe"
//! EFE_LIVE_SMOKE=1 \
//!   ANALYZER_URL=http://127.0.0.1:8090 \
//!   REFORMULATOR_URL=http://127.0.0.1:8091 \
//!   COMPRESSOR_URL=http://127.0.0.1:8092 \
//!   VERIFIER_URL=http://127.0.0.1:8093 \
//!   PYO3_PYTHON=$(which python3.12) \
//!   cargo test -p efe-nano-app-coordinator --test live_smoke -- --nocapture
//! ```
//!
//! Vérifie :
//! - Health check sur les 4 robots
//! - Round-trip JSON valide avec grammar enforcement
//! - Latence sous seuil acceptable

use std::sync::Arc;
use std::time::{Duration, Instant};

use efe_nano_app_coordinator::clients::{
    AnalyzerClient, CompressorClient, NanoRobotClient, ReformulatorClient, VerifierClient,
    RobotHealth,
};
use efe_nano_app_coordinator::context::{CallContext, InMemoryAuditWriter};
use efe_nano_app_coordinator::types::*;
use uuid::Uuid;

fn live_enabled() -> bool {
    std::env::var("EFE_LIVE_SMOKE").as_deref() == Ok("1")
}

fn env_url(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

#[tokio::test]
async fn live_analyzer_health() {
    if !live_enabled() {
        eprintln!("[skip] set EFE_LIVE_SMOKE=1 to enable");
        return;
    }
    let client = AnalyzerClient::from_assets(env_url("ANALYZER_URL", "http://127.0.0.1:8090"))
        .expect("from_assets OK");
    let health = client.health().await.expect("health call");
    assert_eq!(health, RobotHealth::Up, "analyzer must be UP");
}

#[tokio::test]
async fn live_reformulator_health() {
    if !live_enabled() {
        eprintln!("[skip] set EFE_LIVE_SMOKE=1 to enable");
        return;
    }
    let client = ReformulatorClient::from_assets(env_url("REFORMULATOR_URL", "http://127.0.0.1:8091"))
        .expect("from_assets OK");
    let health = client.health().await.expect("health call");
    assert_eq!(health, RobotHealth::Up);
}

#[tokio::test]
async fn live_compressor_health() {
    if !live_enabled() {
        eprintln!("[skip] set EFE_LIVE_SMOKE=1 to enable");
        return;
    }
    let client = CompressorClient::from_assets(env_url("COMPRESSOR_URL", "http://127.0.0.1:8092"))
        .expect("from_assets OK");
    let health = client.health().await.expect("health call");
    assert_eq!(health, RobotHealth::Up);
}

#[tokio::test]
async fn live_verifier_health() {
    if !live_enabled() {
        eprintln!("[skip] set EFE_LIVE_SMOKE=1 to enable");
        return;
    }
    let client = VerifierClient::from_assets(env_url("VERIFIER_URL", "http://127.0.0.1:8093"))
        .expect("from_assets OK");
    let health = client.health().await.expect("health call");
    assert_eq!(health, RobotHealth::Up);
}

#[tokio::test]
async fn live_analyzer_roundtrip_with_grammar() {
    if !live_enabled() {
        eprintln!("[skip] set EFE_LIVE_SMOKE=1 to enable");
        return;
    }
    let client = AnalyzerClient::from_assets(env_url("ANALYZER_URL", "http://127.0.0.1:8090"))
        .expect("from_assets OK");
    let writer = Arc::new(InMemoryAuditWriter::default());
    let ctx = CallContext::new(Uuid::now_v7(), writer.clone());

    let input = AnalyzerInput {
        text: "Apple reported $90B revenue in Q3 2026.".into(),
        max_entities: 20,
        source_agent: Some("live_test".into()),
    };
    let started = Instant::now();
    let out = client.invoke(input, &ctx).await;
    let elapsed = started.elapsed();
    eprintln!("live analyzer roundtrip: {:?}", elapsed);

    let out = out.expect("analyzer must return Ok");
    eprintln!("entities={}, intent={:?}, language={:?}, summary={:?}",
              out.entities.len(), out.intent, out.language, out.one_line_summary);

    // Validations basiques.
    assert!(out.confidence >= 0.0 && out.confidence <= 1.0);
    assert!(out.sentiment >= -1.0 && out.sentiment <= 1.0);
    assert!(out.entities.len() <= 20);

    // Latence raisonnable (cible Phase 1 : p99 30 ms sur prompt court,
    // mais en live avec model load on accepte 2s).
    assert!(elapsed < Duration::from_secs(2), "too slow: {elapsed:?}");

    // Audit chain : exactement 1 entry.
    let entries = {
        use efe_nano_app_coordinator::AuditWriter as _;
        writer.entries().await
    };
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].robot_id, "analyzer");
}

#[tokio::test]
async fn live_compressor_with_grammar() {
    if !live_enabled() {
        eprintln!("[skip] set EFE_LIVE_SMOKE=1 to enable");
        return;
    }
    let client = CompressorClient::from_assets(env_url("COMPRESSOR_URL", "http://127.0.0.1:8092"))
        .expect("from_assets OK");
    let ctx = CallContext::mock();
    let input = CompressorInput {
        text: "La Réserve fédérale américaine a annoncé mercredi qu'elle maintenait son taux directeur dans la fourchette de 5,25 à 5,50 %. Cette décision marque la cinquième pause consécutive. Le président de la Fed, Jerome Powell, a indiqué lors de la conférence de presse que des baisses de taux restent possibles plus tard en 2026 si l'inflation continue de ralentir vers la cible de 2 %.".into(),
        target_words: 50,
        must_preserve: vec!["Réserve fédérale".into(), "Jerome Powell".into()],
        style: CompressionStyle::Extractive,
    };
    let started = Instant::now();
    let out = client.invoke(input, &ctx).await.expect("invoke OK");
    let elapsed = started.elapsed();
    eprintln!("live compressor: {:?} ratio={:.2}", elapsed, out.compression_ratio);
    eprintln!("summary: {}", out.summary);
    assert!(out.compression_ratio >= 1.0);
    assert!(elapsed < Duration::from_secs(5));
}

#[tokio::test]
async fn live_verifier_with_grammar() {
    if !live_enabled() {
        eprintln!("[skip] set EFE_LIVE_SMOKE=1 to enable");
        return;
    }
    let client = VerifierClient::from_assets(env_url("VERIFIER_URL", "http://127.0.0.1:8093"))
        .expect("from_assets OK");
    let ctx = CallContext::mock();
    let input = VerifierInput {
        claim: "The Fed maintained rates at 5.25-5.50%".into(),
        candidate_sources: vec![CandidateSource {
            id: "src-fed-2026-03-20".into(),
            text: "The FOMC voted unanimously to maintain the federal funds rate target range at 5.25 to 5.50 percent.".into(),
        }],
        strictness: VerifierStrictness::Standard,
    };
    let started = Instant::now();
    let out = client.invoke(input, &ctx).await.expect("invoke OK");
    let elapsed = started.elapsed();
    eprintln!("live verifier: {:?} verdict={:?} similarity={:.2}",
              elapsed, out.verdict, out.similarity);
    assert!(out.similarity >= 0.0);
    assert!(elapsed < Duration::from_secs(5));
    // Le verdict devrait être SUPPORTED ou INSUFFICIENT_EVIDENCE — pas
    // UNSUPPORTED / CONTRADICTED puisque la source dit exactement la même chose.
    assert!(matches!(
        out.verdict,
        Verdict::Supported | Verdict::InsufficientEvidence
    ));
}

#[tokio::test]
async fn live_all_four_robots_in_parallel() {
    if !live_enabled() {
        eprintln!("[skip] set EFE_LIVE_SMOKE=1 to enable");
        return;
    }
    let analyzer =
        AnalyzerClient::from_assets(env_url("ANALYZER_URL", "http://127.0.0.1:8090"))
            .expect("analyzer");
    let reformulator =
        ReformulatorClient::from_assets(env_url("REFORMULATOR_URL", "http://127.0.0.1:8091"))
            .expect("reformulator");
    let compressor =
        CompressorClient::from_assets(env_url("COMPRESSOR_URL", "http://127.0.0.1:8092"))
            .expect("compressor");
    let verifier =
        VerifierClient::from_assets(env_url("VERIFIER_URL", "http://127.0.0.1:8093"))
            .expect("verifier");

    let started = Instant::now();
    let (h1, h2, h3, h4) = tokio::join!(
        analyzer.health(),
        reformulator.health(),
        compressor.health(),
        verifier.health(),
    );
    eprintln!("4-robot parallel health: {:?}", started.elapsed());
    assert_eq!(h1.unwrap(), RobotHealth::Up);
    assert_eq!(h2.unwrap(), RobotHealth::Up);
    assert_eq!(h3.unwrap(), RobotHealth::Up);
    assert_eq!(h4.unwrap(), RobotHealth::Up);
    // 4 robots en parallèle doivent répondre en < 500ms.
    assert!(started.elapsed() < Duration::from_millis(500));
}
