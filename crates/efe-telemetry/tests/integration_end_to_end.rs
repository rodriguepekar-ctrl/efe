//! End-to-end integration test for `efe-telemetry`:
//! - init the stack with an in-memory config,
//! - bind the `/metrics` HTTP sidecar on an ephemeral port,
//! - feed a synthetic `RegistryEvent` through a `RegistrySink`,
//! - drive a `TracingTelemetryCollector` through a full swarm lifecycle,
//! - scrape the endpoint and assert the expected metrics are present.

use std::sync::Arc;
use std::time::Duration;

use efe_core::alignment::{BreachContext, BreachReport, ConeKey, RegistryEvent};
use efe_core::fusion::FusionStats;
use efe_core::types::{AlignmentCone, LatentDim, PopulationMix, SwarmSpec, TemplateCode};
use efe_core::{Request, SwarmId, TelemetryCollector};
use tokio::sync::mpsc;

use efe_telemetry::logs::audit::MemoryAuditSink;
use efe_telemetry::{init, RegistrySink, TelemetryConfig, TracingTelemetryCollector};

fn sample_request() -> Request {
    let spec = SwarmSpec::builder()
        .problem_embedding(vec![0.0; 4])
        .dim_latent(LatentDim::new(4).expect("valid"))
        .population(PopulationMix::new(vec![(TemplateCode::NaSem, 4)]).expect("valid"))
        .max_rounds(3)
        .expect("valid")
        .alignment_cone(AlignmentCone::new(vec![vec![0.1; 4]], 0.15).expect("valid"))
        .build()
        .expect("valid");
    Request::for_tests(spec)
}

async fn http_get(url: &str) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let uri = url.parse::<http::Uri>().unwrap();
    let authority = uri.authority().unwrap().as_str();
    let path = uri.path();
    let mut stream = tokio::net::TcpStream::connect(authority).await.unwrap();
    let req = format!("GET {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf).to_string();
    let status = text
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let body = text
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_string())
        .unwrap_or_default();
    (status, body)
}

#[tokio::test]
async fn full_pipeline_emits_expected_signals() {
    // 1. Init telemetry stack with metrics HTTP on ephemeral port.
    let mut cfg = TelemetryConfig::silent();
    cfg.metrics_addr = Some("127.0.0.1:0".parse().unwrap());
    let mut handle = init(cfg).expect("init ok");
    handle.ensure_metrics_bound().await.expect("bind ok");
    let metrics_url = format!(
        "http://{}/metrics",
        handle.metrics_local_addr().expect("bound")
    );

    // 2. Attach an audit sink and spawn a RegistrySink in the background.
    let audit = Arc::new(MemoryAuditSink::new());
    let sink = RegistrySink::new(Some(
        audit.clone() as Arc<dyn efe_telemetry::logs::audit::AuditSink>
    ));
    let (tx, rx) = mpsc::unbounded_channel();
    let sink_task = tokio::spawn(sink.run(rx));

    // 3. Simulate a swarm lifecycle through the TelemetryCollector.
    let collector = TracingTelemetryCollector::new();
    let req = sample_request();
    let mut guard = collector.begin_swarm(SwarmId::new(), &req);
    let stats = FusionStats::synthetic_with(1, 4, 0.01, 0.05);
    guard.record_round(1, &stats);
    guard.record_round(2, &stats);
    guard.finalize(12.3, &stats.with_confidence(0.92));

    // 4. Emit M9 events through the sink.
    tx.send(RegistryEvent::Registered {
        key: ConeKey::new("acme", "qa", 1).unwrap(),
        at_nanos: 1,
    })
    .unwrap();
    tx.send(RegistryEvent::BreachReported {
        report: BreachReport::new(
            ConeKey::new("acme", "qa", 1).unwrap(),
            SwarmId::new(),
            0.9,
            0.5,
            2,
            BreachContext::Dissolution,
        ),
    })
    .unwrap();
    tx.send(RegistryEvent::PolicyRejected {
        attempted_key: None,
        reason: "policy violation: I5: duplicate anchor".to_string(),
    })
    .unwrap();
    drop(tx);
    let processed = sink_task.await.unwrap();
    assert_eq!(processed, 3);

    // Give Prometheus a tick to flush observations (atomic, but be safe).
    tokio::time::sleep(Duration::from_millis(50)).await;

    // 5. Scrape /metrics.
    let (status, body) = http_get(&metrics_url).await;
    assert_eq!(status, 200);

    // Swarm lifecycle signals.
    assert!(
        body.contains("efe_swarm_rounds_total"),
        "missing swarm_rounds_total; body = {body}"
    );
    assert!(body.contains("efe_swarm_duration_seconds"));
    assert!(body.contains("efe_swarm_confidence"));

    // Registry sink signals.
    assert!(body.contains("efe_cone_registrations_total"));
    assert!(body.contains("efe_breaches_total"));
    assert!(body.contains("efe_policy_rejections_total"));
    assert!(body.contains("efe_breach_magnitude"));

    // Audit entries recorded.
    let audit_entries = audit.entries();
    assert_eq!(audit_entries.len(), 3);
    let events: Vec<&str> = audit_entries
        .iter()
        .map(|e| e["event"].as_str().unwrap_or(""))
        .collect();
    assert!(events.contains(&"cone.registered"));
    assert!(events.contains(&"alignment.breach"));
    assert!(events.contains(&"policy.rejected"));

    handle.shutdown().await;
}

#[tokio::test]
async fn metrics_endpoint_content_type_is_prometheus_text() {
    let mut cfg = TelemetryConfig::silent();
    cfg.metrics_addr = Some("127.0.0.1:0".parse().unwrap());
    let mut handle = init(cfg).expect("init ok");
    handle.ensure_metrics_bound().await.expect("bind ok");
    let url = format!(
        "http://{}/metrics",
        handle.metrics_local_addr().expect("bound")
    );

    // Retrieve raw headers to assert content-type.
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let uri = url.parse::<http::Uri>().unwrap();
    let mut stream = tokio::net::TcpStream::connect(uri.authority().unwrap().as_str())
        .await
        .unwrap();
    let req = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
        uri.path(),
        uri.authority().unwrap().as_str()
    );
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8_lossy(&buf).to_string();
    assert!(
        text.to_lowercase().contains("content-type: text/plain"),
        "expected prometheus content-type, got: {}",
        text.lines().take(10).collect::<Vec<_>>().join("\n")
    );
    assert!(text.contains("version=0.0.4"));
    handle.shutdown().await;
}
