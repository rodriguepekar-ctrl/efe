# `efe-coordinator` — Phase 2 du swarm nano-robots

Crate Rust qui livre la **pièce centrale** du système nano-robots EFE :
le **Coordinator**.  Il transforme les 6 clients Phase 1 isolés
(`analyzer`, `reformulator`, `compressor`, `verifier`, `filter`, `router`)
en une plateforme cohérente capable de :

- orchestrer les nano-robots en **DAG parallèle** ;
- coordonner **plusieurs gros LLMs** (Claude, GPT-4, Gemini) en parallèle ;
- agréger les réponses via **vote pondéré** (Symphony) ;
- exécuter le pattern **Speculative RAG** (drafter local + verifier gros LLM) ;
- exposer tout cela en **streaming SSE typé** à un client UI.

## Quick start

```rust
use std::sync::Arc;
use efe_coordinator::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Enregistrer les providers LLM (mock ou vrais).
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("claude").with_response("hello")));

    // 2. Construire le coordinator.
    let coord = Coordinator::builder().registry(reg).build()?;

    // 3. (UI) Souscrire au bus SSE.
    let (_token, mut rx) = coord.subscribe(Some("ui"));
    tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            println!("[{}] {}", ev.tag(), ev.to_sse_frame());
        }
    });

    // 4. Lancer une Mission A.
    let req = MissionARequest {
        user_text: "What is the capital of France?".into(),
        task_class: efe_nano_app_coordinator::types::router::TaskClass::FactualQa,
        available_llms: vec!["claude".into()],
        llm_metadata: Default::default(),
        budget: efe_nano_app_coordinator::types::Budget {
            max_cost_cents: 100,
            max_latency_ms: 5_000,
            min_quality_score: 0.5,
        },
        candidate_sources: vec![],
        max_total_latency: std::time::Duration::from_secs(5),
    };
    let resp = run_mission_a(&coord, req).await?;
    println!("FINAL: {}", resp.final_text);
    Ok(())
}
```

## Architecture

```
                    ┌──────────────────────┐
                    │      Coordinator     │
                    └──────────┬───────────┘
                               │
       ┌───────────────────────┼────────────────────────┐
       │                       │                        │
┌──────▼───────┐    ┌──────────▼────────┐   ┌───────────▼─────────┐
│  DagExecutor │    │ PheromoneBlackbd  │   │     EventBus        │
│ (parallel)   │    │  (256 KB bounded) │   │ (SSE pub/sub)       │
└──────┬───────┘    └─────────┬─────────┘   └──────────┬──────────┘
       │                      │                        │
       └─── consume ──┬───────┴──── publish ───────────┘
                     │
       ┌─────────────┴─────────────────┐
       │                               │
┌──────▼──────────────┐      ┌─────────▼────────────────┐
│ NanoRobotClient × 6 │      │  MultiLlmBridge          │
│ (Phase 1 crate)     │      │   ├ Claude               │
│ analyzer/filter/…   │      │   ├ GPT-4                │
│                     │      │   ├ Gemini               │
│                     │      │   └ local SLM (drafter)  │
└─────────────────────┘      └──────────────────────────┘
                                       │
                              ┌────────┴────────┐
                              │  VoteEngine     │  ← Symphony
                              │  Speculator     │  ← Speculative RAG
                              └─────────────────┘
```

## Les 3 missions exposées

| Mission | Fonction | Cas d'usage |
|---|---|---|
| **Mission A** | `run_mission_a` | Pré-mâcher pour 1 LLM (cost + qualité) |
| **Mission B** | `run_mission_b` | N LLMs en parallèle + vote consensus |
| **Speculative** | `run_speculative` | Drafter local + verifier gros LLM (latence ↓) |

## Composants

| Module | Type | Stabilité |
|---|---|---|
| `coordinator::Coordinator` | API publique | **Stable** |
| `dag::DagExecutor` | Orchestrateur DAG | Stable |
| `blackboard::PheromoneBlackboard` | Mémoire partagée 256 KB | Stable |
| `events::EventBus` + `InMemoryBus` | Pub/sub | Stable |
| `events::NatsBus` (feature `nats-bus`) | JetStream prod | Beta (Phase 7) |
| `llm::MultiLlmBridge` | Fan-out N providers | Stable |
| `vote::VoteEngine` | Symphony pondéré | Stable |
| `speculate::Speculator` | Drafter/verifier | Stable |
| `events::StreamEvent` | Protocole SSE v1.0.0 | **Frozen** |

## Invariants (testés)

1. **I1** — chaque invocation de robot produit exactement 1 audit entry.
2. **I2** — blackboard ≤ 256 KB (`MAX_BLACKBOARD_BYTES`).
3. **I3** — tout `StreamEvent` est JSON roundtrip-stable.
4. **I4** — workflows idempotents pour un `swarm_id` donné.
5. **I5** — vote sans quorum → fallback déterministe sur `success_rate`.
6. **I6** — Mission B exige ≥ 2 LLMs (sinon erreur typée).
7. **I7** — phéromones monotones non-croissantes hors dépôt.
8. **I8** — DagExecutor borné par `max_concurrency` (défaut 8).

## Tests

```bash
PYO3_PYTHON=$(which python3.12) cargo test -p efe-coordinator
# → 90 lib + 70 intégration = 160 tests
```

Subdivision :

| Binaire | Tests | Focus |
|---|---|---|
| `lib` (inline) | 90 | unit per-module |
| `tests/unit_blackboard.rs` | 9 | I2/I7 + LRU |
| `tests/unit_bridge.rs` | 6 | fan-out, registry |
| `tests/unit_bus.rs` | 7 | sub/unsub, backpressure |
| `tests/unit_dag.rs` | 8 | cycle, parallélisme I8 |
| `tests/unit_speculator.rs` | 5 | drafter/verifier |
| `tests/unit_vote.rs` | 7 | quorum, fallback I5 |
| `tests/workflow_mission_a.rs` | 4 | E2E A |
| `tests/workflow_mission_b.rs` | 5 | E2E B + vote |
| `tests/workflow_speculative.rs` | 5 | E2E spéculation |
| `tests/sse_stream.rs` | 8 | protocole SSE v1 |
| `tests/invariants.rs` | 6 | I1–I8 transverses |

## Benchmarks

```bash
PYO3_PYTHON=$(which python3.12) cargo bench -p efe-coordinator
# Résultats HTML : target/criterion/
```

8 groupes Criterion : `blackboard`, `vote`, `dag`, `bridge`, `workflow`.

## Features cargo

| Flag | Effet |
|---|---|
| (none) | Bus in-process uniquement (default) |
| `nats-bus` | Active le `NatsBus` JetStream (prod) — **stub Phase 7** |

## Roadmap

- Phase 1 (livrée) — `efe-nano-app-coordinator` : 6 clients Rust + JSON Grammar.
- Phase 2 (**ce crate**) — Coordinator + DAG + blackboard + SSE.
- Phase 3 (à venir) — Frontend SSE.
- Phase 7 (à venir) — Durcissement multi-tenant prod (NATS, Postgres audit, cosign).

Voir `EFE_PROJET/03_a_construire/PHASE_2_COORDINATOR.pdf` (26 pages)
pour l'architecture complète.
