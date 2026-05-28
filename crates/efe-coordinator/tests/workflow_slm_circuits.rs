// SPDX-License-Identifier: UNLICENSED
//! **R3.S2 (fix L2)** — Tests intégration `SlmCircuitBreaker` wiring dans
//! `run_mission_a` / `run_mission_b`.
//!
//! Couvre les chemins critiques du wiring R3.S2 :
//! - Skip analyzer/compressor/verifier quand circuit pré-Open.
//! - `FailureMode::SlmDown(robot)` correctement tracé.
//! - `ChewingMetrics.failed_components` push.
//! - Backward-compat : sans `slm_circuits`, comportement S8 préservé.
//! - Composite quand plusieurs SLM down dans la même mission.
//! - Recovery temporel (Open → HalfOpen → Closed).
//!
//! Stratégie : pré-ouvrir le circuit AVANT mission → le workflow voit
//! `is_open() == true` et skip l'invoke (sans avoir besoin de mock fail
//! d'un SLM client, qui est complexe à construire offline).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use efe_coordinator::failure::FailureMode;
use efe_coordinator::{
    run_mission_a, run_mission_b, Coordinator, MissionARequest, MissionBRequest,
    MockLlmProvider, ProviderRegistry,
};
use efe_nano_app_coordinator::clients::llama_server::LlamaServerClient;
use efe_nano_app_coordinator::clients::{
    AnalyzerClient, CompressorClient, SlmCircuitBreaker, SlmCircuitState, VerifierClient,
};
use efe_nano_app_coordinator::types::router::TaskClass;
use efe_nano_app_coordinator::types::{Budget, LlmMetadata};

// =====================================================================
// Helpers de construction (LLM mock + nano-clients factices)
// =====================================================================

/// LlmMetadata minimal pour les tests R3.S2 (cost arbitraire — circuit
/// indépendant de la composante cost).
fn meta() -> LlmMetadata {
    LlmMetadata {
        cost_input_per_million: 500,
        cost_output_per_million: 1500,
        cost_input_per_million_microcents: 0,
        cost_output_per_million_microcents: 0,
        p50_latency_ms: 500,
        p99_latency_ms: 2000,
        success_rate: 0.99,
        capability_scores: HashMap::new(),
        circuit_open: false,
    }
}

/// **R3.S2** — Construit un `AnalyzerClient` factice via `from_helper` qui
/// pointe vers une URL inaccessible (`127.0.0.1:1`) + timeout 5ms.  Si
/// jamais invoqué (= circuit Closed), il fail vite (en ~5ms). Avec le
/// circuit pré-Open, il ne sera jamais invoqué donc l'URL importe peu.
fn fail_fast_analyzer() -> AnalyzerClient {
    let inner = LlamaServerClient {
        id: "analyzer",
        schema_version: "1.0.0",
        endpoint: "http://127.0.0.1:1".into(), // inaccessible
        grammar: String::new(),
        system_prompt: String::new(),
        max_tokens: 16,
        temperature: 0.0,
        timeout: Duration::from_millis(5),
        http: reqwest::Client::new(),
    };
    AnalyzerClient::from_helper(inner)
}

fn fail_fast_compressor() -> CompressorClient {
    let inner = LlamaServerClient {
        id: "compressor",
        schema_version: "1.0.0",
        endpoint: "http://127.0.0.1:1".into(),
        grammar: String::new(),
        system_prompt: String::new(),
        max_tokens: 16,
        temperature: 0.0,
        timeout: Duration::from_millis(5),
        http: reqwest::Client::new(),
    };
    CompressorClient::from_helper(inner)
}

fn fail_fast_verifier() -> VerifierClient {
    let inner = LlamaServerClient {
        id: "verifier",
        schema_version: "1.0.0",
        endpoint: "http://127.0.0.1:1".into(),
        grammar: String::new(),
        system_prompt: String::new(),
        max_tokens: 16,
        temperature: 0.0,
        timeout: Duration::from_millis(5),
        http: reqwest::Client::new(),
    };
    VerifierClient::from_helper(inner)
}

/// Helper : construit un Coordinator avec mock LLM "claude" + clients SLM
/// fail-fast UNIQUEMENT pour les robots listés dans `circuit_states`.
///
/// `circuit_states[i]` = (robot_name, is_pre_open).
///
/// Cette construction sélective évite les effets de bord : un robot non
/// listé n'a NI client NI circuit → workflow ne tente même pas
/// `if let Some(robot)`. Permet d'isoler le comportement R3.S2 par robot.
fn make_coord_with_slm_circuits(
    circuit_states: &[(&str, bool)],
) -> Coordinator {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(
        MockLlmProvider::new("claude_mock")
            .with_response("OK answer")
            .with_cost_microcents(45),
    ));

    let mut circuits: HashMap<String, SlmCircuitBreaker> = HashMap::new();
    let mut clients = efe_coordinator::CoordinatorClients::default();
    for (robot, pre_open) in circuit_states {
        let cb = SlmCircuitBreaker::new(1, Duration::from_secs(10));
        if *pre_open {
            cb.record_failure(); // → Open (threshold=1).
            assert_eq!(cb.state(), SlmCircuitState::Open);
        }
        circuits.insert((*robot).to_string(), cb);
        // N'attache QUE le client correspondant au circuit demandé.
        match *robot {
            "analyzer" => clients.analyzer = Some(Arc::new(fail_fast_analyzer())),
            "compressor" => clients.compressor = Some(Arc::new(fail_fast_compressor())),
            "verifier" => clients.verifier = Some(Arc::new(fail_fast_verifier())),
            _ => panic!("Unknown SLM robot in test: {robot}"),
        }
    }
    clients.slm_circuits = circuits;

    Coordinator::builder()
        .registry(reg)
        .clients(clients)
        .build()
        .expect("Coordinator builder must succeed")
}

fn make_mission_a_request() -> MissionARequest {
    let mut metas = HashMap::new();
    metas.insert("claude_mock".to_string(), meta());
    MissionARequest {
        user_text: "Bonjour".into(),
        task_class: TaskClass::FactualQa,
        available_llms: vec!["claude_mock".into()],
        llm_metadata: metas,
        budget: Budget {
            max_cost_cents: 1_000,
            max_cost_microcents: 0,
            max_latency_ms: 5_000,
            min_quality_score: 0.0,
        },
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(2),
        task_class_scoring_table: None,
    }
}

fn make_mission_b_request() -> MissionBRequest {
    let mut metas = HashMap::new();
    for id in ["claude", "gpt", "gemini"] {
        metas.insert(id.to_string(), meta());
    }
    MissionBRequest {
        user_text: "Bonjour".into(),
        task_class: TaskClass::FactualQa,
        llms: vec!["claude".into(), "gpt".into(), "gemini".into()],
        llm_metadata: metas,
        vote_weights: HashMap::new(),
        quorum: Some(2),
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(2),
    }
}

// =====================================================================
// Tests R3.S2 — Mission A
// =====================================================================

/// **R3.S2 Test 1** : circuit `analyzer` pré-Open → mission_a doit skipper
/// l'analyzer + tracer `FailureMode::SlmDown("analyzer")`.
#[tokio::test]
async fn r3s2_mission_a_skips_analyzer_when_circuit_pre_open() {
    let coord = make_coord_with_slm_circuits(&[("analyzer", true)]);
    let resp = run_mission_a(&coord, make_mission_a_request())
        .await
        .expect("mission_a doit fonctionner même avec analyzer skippé");

    assert!(
        matches!(resp.failure_mode, FailureMode::SlmDown(ref r) if r == "analyzer"),
        "FailureMode doit être SlmDown(\"analyzer\"), got {:?}",
        resp.failure_mode
    );
    let cm = resp.chewing_metrics.as_ref().expect("chewing_metrics");
    assert!(
        cm.failed_components.contains(&"analyzer".to_string()),
        "failed_components doit contenir analyzer"
    );
    // L'analyzer ayant été skippé, sa latence reste 0.
    assert_eq!(
        cm.analyzer_latency_ms, 0,
        "analyzer_latency_ms doit être 0 quand circuit Open (skip <1ms)"
    );
}

/// **R3.S2 Test 2** : circuit `compressor` pré-Open → mission_a skip
/// compressor → utilise raw user_text (pas de compression).
#[tokio::test]
async fn r3s2_mission_a_skips_compressor_when_circuit_pre_open() {
    let coord = make_coord_with_slm_circuits(&[("compressor", true)]);
    // user_text > 500 chars pour déclencher le compressor.
    let mut req = make_mission_a_request();
    req.user_text = "x".repeat(600);

    let resp = run_mission_a(&coord, req)
        .await
        .expect("mission_a doit fonctionner avec compressor skippé");

    assert!(
        matches!(resp.failure_mode, FailureMode::SlmDown(ref r) if r == "compressor"),
        "FailureMode doit être SlmDown(\"compressor\"), got {:?}",
        resp.failure_mode
    );
    let cm = resp.chewing_metrics.as_ref().expect("chewing_metrics");
    assert!(cm.failed_components.contains(&"compressor".to_string()));
    assert_eq!(cm.compressor_latency_ms, 0);
}

/// **R3.S2 Test 3** : circuit `verifier` pré-Open + candidate_sources non
/// vides → skip verifier, pas de verdicts.
#[tokio::test]
async fn r3s2_mission_a_skips_verifier_when_circuit_pre_open() {
    use efe_coordinator::CandidateSourceRef;
    let coord = make_coord_with_slm_circuits(&[("verifier", true)]);
    let mut req = make_mission_a_request();
    req.candidate_sources = vec![CandidateSourceRef {
        id: "s1".into(),
        text: "Bonjour means hello in French".into(),
    }];

    let resp = run_mission_a(&coord, req)
        .await
        .expect("mission_a doit fonctionner avec verifier skippé");

    assert!(
        matches!(resp.failure_mode, FailureMode::SlmDown(ref r) if r == "verifier"),
        "FailureMode doit être SlmDown(\"verifier\"), got {:?}",
        resp.failure_mode
    );
    assert!(
        resp.verdicts.is_empty(),
        "verifier skipped → aucun verdict produit"
    );
}

/// **R3.S2 Test 4** : circuits multiples pré-Open → `FailureMode::Composite`
/// avec tous les SlmDown.
#[tokio::test]
async fn r3s2_mission_a_multiple_circuits_open_produces_composite() {
    use efe_coordinator::CandidateSourceRef;
    let coord = make_coord_with_slm_circuits(&[
        ("analyzer", true),
        ("verifier", true),
    ]);
    let mut req = make_mission_a_request();
    req.candidate_sources = vec![CandidateSourceRef {
        id: "s1".into(),
        text: "x".into(),
    }];

    let resp = run_mission_a(&coord, req).await.expect("mission_a OK");

    if let FailureMode::Composite(modes) = &resp.failure_mode {
        assert_eq!(modes.len(), 2, "2 circuits Open → Composite à 2 modes");
        assert!(modes.contains(&FailureMode::SlmDown("analyzer".into())));
        assert!(modes.contains(&FailureMode::SlmDown("verifier".into())));
    } else {
        panic!(
            "attendu Composite([analyzer, verifier]), got {:?}",
            resp.failure_mode
        );
    }
    let cm = resp.chewing_metrics.as_ref().expect("chewing_metrics");
    assert_eq!(cm.degradations_count, 2);
    assert!(cm.failed_components.contains(&"analyzer".to_string()));
    assert!(cm.failed_components.contains(&"verifier".to_string()));
}

/// **R3.S2 Test 5** : backward-compat — Coordinator sans `slm_circuits`
/// (HashMap empty) → comportement S8 préservé (pas d'effet R3.S2).
///
/// Note : les SLM clients fail-fast vont fail le invoke (URL :1 + 5ms
/// timeout), mais c'est le comportement S8 attendu : on continue sans
/// circuit, juste un `RobotFinished error` dans le bus.  `failure_mode`
/// reste `None` (parce que SANS circuit, le path de tracking R3.S2 n'est
/// pas activé — c'est la définition même de backward-compat).
#[tokio::test]
async fn r3s2_mission_a_no_circuits_falls_back_to_s8_behavior() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(
        MockLlmProvider::new("claude_mock").with_response("ok"),
    ));
    let coord = Coordinator::builder()
        .registry(reg)
        .clients(efe_coordinator::CoordinatorClients {
            analyzer: Some(Arc::new(fail_fast_analyzer())),
            // PAS de slm_circuits : HashMap empty.
            ..efe_coordinator::CoordinatorClients::default()
        })
        .build()
        .expect("Coordinator OK");

    let resp = run_mission_a(&coord, make_mission_a_request())
        .await
        .expect("mission_a OK même si analyzer fail (optionnel)");

    // R3.S2 backward-compat : pas de SlmDown tracé (pas de guard activé).
    // L'analyzer va fail (URL inaccessible), mais on a aucun circuit pour
    // tracer ça en FailureMode::SlmDown.
    // NB : la mission complète quand même (analyzer optionnel).
    assert_eq!(
        resp.failure_mode,
        FailureMode::None,
        "Sans slm_circuits, R3.S2 n'a pas d'effet → failure_mode reste None (S8 compat)"
    );
}

/// **R3.S2 Test 6** : helper `CoordinatorClients::default_slm_circuits()`
/// crée 3 circuits préconfigurés (analyzer/compressor/verifier).
#[tokio::test]
async fn r3s2_default_slm_circuits_helper_creates_3_robots() {
    use efe_coordinator::CoordinatorClients;
    let circuits = CoordinatorClients::default_slm_circuits();
    assert_eq!(circuits.len(), 3);
    assert!(circuits.contains_key("analyzer"));
    assert!(circuits.contains_key("compressor"));
    assert!(circuits.contains_key("verifier"));
    // Tous Closed initial.
    for (robot, cb) in &circuits {
        assert_eq!(
            cb.state(),
            SlmCircuitState::Closed,
            "circuit {robot} doit être Closed initial"
        );
        assert_eq!(cb.threshold(), 3, "threshold default = 3");
    }
}

/// **R3.S2 Test 7** : builder helper `.with_default_slm_circuits()` active
/// les 3 circuits par défaut.
#[tokio::test]
async fn r3s2_builder_with_default_slm_circuits_activates_circuits() {
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("c").with_response("ok")));
    let coord = Coordinator::builder()
        .registry(reg)
        .with_default_slm_circuits()
        .build()
        .expect("OK");
    assert_eq!(coord.clients.slm_circuits.len(), 3);
    assert!(coord.clients.slm_circuit("analyzer").is_some());
    assert!(coord.clients.slm_circuit("compressor").is_some());
    assert!(coord.clients.slm_circuit("verifier").is_some());
    assert!(coord.clients.slm_circuit("unknown").is_none());
}

// =====================================================================
// Tests R3.S2 — Mission B
// =====================================================================

/// **R3.S2 Test 8** : Mission B avec circuit `analyzer` pré-Open → skip
/// analyzer mais continue avec fan-out N LLMs.
#[tokio::test]
async fn r3s2_mission_b_skips_analyzer_when_circuit_pre_open() {
    let reg = ProviderRegistry::new();
    for id in ["claude", "gpt", "gemini"] {
        reg.register(Arc::new(
            MockLlmProvider::new(id).with_response("Paris is capital"),
        ));
    }
    let mut circuits = HashMap::new();
    let cb = SlmCircuitBreaker::new(1, Duration::from_secs(10));
    cb.record_failure();
    circuits.insert("analyzer".to_string(), cb);

    let coord = Coordinator::builder()
        .registry(reg)
        .clients(efe_coordinator::CoordinatorClients {
            analyzer: Some(Arc::new(fail_fast_analyzer())),
            slm_circuits: circuits,
            ..efe_coordinator::CoordinatorClients::default()
        })
        .build()
        .expect("OK");

    let resp = run_mission_b(&coord, make_mission_b_request())
        .await
        .expect("Mission B doit fonctionner avec analyzer skippé");

    let cm = resp.chewing_metrics.as_ref().expect("chewing_metrics");
    assert!(
        cm.failed_components.contains(&"analyzer".to_string()),
        "Mission B doit tracer analyzer dans failed_components"
    );
    assert_eq!(cm.analyzer_latency_ms, 0);
    // Mission B a quand même tourné (3 LLMs ont voté).
    assert_eq!(resp.per_llm_scores.len(), 3);
}

/// **R3.S2 Test 9** : Mission B avec circuit `compressor` pré-Open
/// (mutualisé, 1× pour N LLMs) → skip compressor → utilise raw text.
#[tokio::test]
async fn r3s2_mission_b_skips_compressor_when_circuit_pre_open() {
    let reg = ProviderRegistry::new();
    for id in ["claude", "gpt", "gemini"] {
        reg.register(Arc::new(MockLlmProvider::new(id).with_response("Paris")));
    }
    let mut circuits = HashMap::new();
    let cb = SlmCircuitBreaker::new(1, Duration::from_secs(10));
    cb.record_failure();
    circuits.insert("compressor".to_string(), cb);

    let coord = Coordinator::builder()
        .registry(reg)
        .clients(efe_coordinator::CoordinatorClients {
            compressor: Some(Arc::new(fail_fast_compressor())),
            slm_circuits: circuits,
            ..efe_coordinator::CoordinatorClients::default()
        })
        .build()
        .expect("OK");

    let mut req = make_mission_b_request();
    req.user_text = "x".repeat(600); // > 500 → compressor activé.

    let resp = run_mission_b(&coord, req).await.expect("Mission B OK");
    let cm = resp.chewing_metrics.as_ref().expect("chewing_metrics");
    assert!(cm.failed_components.contains(&"compressor".to_string()));
    assert_eq!(cm.compressor_latency_ms, 0);
}

// =====================================================================
// Tests R3.S2 — Recovery temporel (Open → HalfOpen → Closed)
// =====================================================================

/// **R3.S2 Test 10** : recovery temporel — circuit Open, après `reset_timeout`
/// passe en HalfOpen, succès lors du probe → Closed.
///
/// Test purement sur le `SlmCircuitBreaker` (intégration workflow non
/// requise pour valider la mécanique de recovery — c'est le rôle des
/// tests unitaires `slm_circuit.rs` mais on duplique au niveau R3.S2 pour
/// valider que `slm_guard_record_success` reset bien le circuit).
#[tokio::test]
async fn r3s2_circuit_recovers_open_to_halfopen_to_closed() {
    let cb = SlmCircuitBreaker::new(2, Duration::from_millis(80));
    cb.record_failure();
    cb.record_failure();
    assert_eq!(cb.state(), SlmCircuitState::Open);

    // Wait reset_timeout → HalfOpen.
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert_eq!(cb.state(), SlmCircuitState::HalfOpen);

    // Probe success (via slm_guard_record_success) → Closed.
    // Note : slm_guard_record_success est pub(crate), donc on appelle
    // directement record_success ici (équivalent).
    cb.record_success();
    assert_eq!(cb.state(), SlmCircuitState::Closed);
    assert_eq!(cb.failure_count(), 0);
}

// =====================================================================
// Tests R3.S2 — Bus event SlmCircuitOpened
// =====================================================================

/// **R3.S2 Test 11** : émission bus event `SlmCircuitOpened` quand un
/// invoke fail provoque la transition Closed → Open.
///
/// Setup : circuit threshold=1, AnalyzerClient fail-fast (URL :1) qui
/// fail au invoke → record_failure → transition → 1 event émis.
#[tokio::test]
async fn r3s2_mission_a_emits_slm_circuit_opened_event_on_transition() {
    use efe_coordinator::StreamEvent;

    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("claude").with_response("ok")));

    let mut circuits = HashMap::new();
    // threshold=1 : la 1ère failure ouvre directement.
    circuits.insert(
        "analyzer".to_string(),
        SlmCircuitBreaker::new(1, Duration::from_secs(10)),
    );

    let coord = Coordinator::builder()
        .registry(reg)
        .clients(efe_coordinator::CoordinatorClients {
            analyzer: Some(Arc::new(fail_fast_analyzer())),
            slm_circuits: circuits,
            ..efe_coordinator::CoordinatorClients::default()
        })
        .build()
        .expect("OK");
    let (_t, mut rx) = coord.subscribe(Some("test-circuit-event"));

    // 1ère mission : analyzer fail (URL inaccessible 5ms timeout) →
    // record_failure → threshold 1 atteint → circuit Open → emit event.
    let mut metas = HashMap::new();
    metas.insert("claude".to_string(), meta());
    let req = MissionARequest {
        user_text: "test".into(),
        task_class: TaskClass::FactualQa,
        available_llms: vec!["claude".into()],
        llm_metadata: metas,
        budget: Budget {
            max_cost_cents: 1000,
            max_cost_microcents: 0,
            max_latency_ms: 10_000,
            min_quality_score: 0.0,
        },
        candidate_sources: vec![],
        max_total_latency: Duration::from_secs(2),
        task_class_scoring_table: None,
    };
    let _resp = run_mission_a(&coord, req).await.expect("OK");

    // Drain le bus avec tokio::time::timeout (recv() bloque, on borne 50ms).
    let mut event_count = 0;
    while let Ok(Some(ev)) =
        tokio::time::timeout(Duration::from_millis(100), rx.recv()).await
    {
        if let StreamEvent::SlmCircuitOpened { robot, failures, .. } = ev {
            assert_eq!(robot, "analyzer");
            assert!(failures >= 1, "failures count must be ≥ threshold");
            event_count += 1;
        }
    }
    assert_eq!(
        event_count, 1,
        "exactement 1 event SlmCircuitOpened par transition Closed→Open (got {event_count})"
    );
    // Le circuit doit être Open après ce passage.
    assert!(coord.clients.slm_circuit("analyzer").unwrap().is_open());
}

// =====================================================================
// Tests R3.S2 — Mission Speculative N'EST PAS impactée
// =====================================================================

/// **R3.S2 Test 12** : Mission Speculative n'utilise pas les SLM circuits
/// (drafter/verifier sont des LLM cloud, pas des SLM Phase 1). Vérifie
/// que `slm_circuits` configuré n'a aucun effet sur Speculative.
#[tokio::test]
async fn r3s2_speculative_unaffected_by_slm_circuits() {
    use efe_coordinator::{run_speculative, SpeculativeRequest};
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(
        MockLlmProvider::new("drafter").with_response("Paris"),
    ));
    reg.register(Arc::new(
        MockLlmProvider::new("verifier_llm").with_response("Paris is capital"),
    ));

    // Tous les circuits SLM pré-Open : devrait n'avoir AUCUN effet.
    // Note : make_coord_with_slm_circuits utilise "claude_mock" → on
    // construit un nouveau coord ci-dessous pour avoir drafter + verifier_llm
    // dans le registry.  Le `_unused_coord` ci-dessous reste pour vérifier
    // que la helper compile, mais n'est PAS utilisé dans la mission.
    let _unused_coord = make_coord_with_slm_circuits(&[
        ("analyzer", true),
        ("compressor", true),
        ("verifier", true),
    ]);
    let mut circuits = HashMap::new();
    for robot in ["analyzer", "compressor", "verifier"] {
        let cb = SlmCircuitBreaker::new(1, Duration::from_secs(10));
        cb.record_failure();
        circuits.insert(robot.to_string(), cb);
    }
    let _unused_coord2 = Coordinator::builder()
        .registry(reg)
        .clients(efe_coordinator::CoordinatorClients {
            slm_circuits: circuits,
            ..efe_coordinator::CoordinatorClients::default()
        })
        .build()
        .expect("OK");

    let req = SpeculativeRequest {
        user_text: "test".into(),
        task_class: TaskClass::FactualQa,
        verifier_llm: "verifier_llm".into(),
        verifier_metadata: meta(),
        budget: Budget {
            max_cost_cents: 1000,
            max_cost_microcents: 0,
            max_latency_ms: 5000,
            min_quality_score: 0.5,
        },
        max_draft_tokens: 64,
        max_total_latency: Duration::from_secs(2),
        drafter_llm: "drafter".to_string(),
        accept_threshold: 0.7,
        max_words_for_quick_accept: 10, // hot path activé
    };
    // Mission Speculative doit fonctionner SANS être affectée par les
    // circuits SLM (qui ne s'appliquent pas à elle).
    let coord2 = {
        let reg = ProviderRegistry::new();
        reg.register(Arc::new(
            MockLlmProvider::new("drafter").with_response("Paris"),
        ));
        reg.register(Arc::new(
            MockLlmProvider::new("verifier_llm")
                .with_response("Paris is capital"),
        ));
        let mut circuits = HashMap::new();
        for robot in ["analyzer", "compressor", "verifier"] {
            let cb = SlmCircuitBreaker::new(1, Duration::from_secs(10));
            cb.record_failure();
            circuits.insert(robot.to_string(), cb);
        }
        Coordinator::builder()
            .registry(reg)
            .clients(efe_coordinator::CoordinatorClients {
                slm_circuits: circuits,
                ..efe_coordinator::CoordinatorClients::default()
            })
            .build()
            .expect("OK")
    };
    let resp = run_speculative(&coord2, req).await.expect("Speculative OK");
    // Hot path actif → verifier_skipped == true (drafter Paris ≤ 10 mots).
    assert!(resp.verifier_skipped, "hot path doit bypasser verifier_llm");
    // failure_mode NONE car aucun guard SLM activé en Speculative.
    // Note : SpeculativeResponse n'a pas de field failure_mode direct,
    // mais chewing_metrics.failure_mode doit rester None.
    if let Some(cm) = &resp.chewing_metrics {
        assert_eq!(
            cm.failure_mode,
            FailureMode::None,
            "Speculative ne doit PAS être affecté par les SLM circuits"
        );
        assert!(
            cm.failed_components.is_empty(),
            "Speculative ne push rien dans failed_components"
        );
    }
}
