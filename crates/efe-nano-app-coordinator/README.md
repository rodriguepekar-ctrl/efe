# efe-nano-app-coordinator

> **Phase 1 — JSON Grammar enforcement + clients Rust pour les 6 nano-robots SLM.**

Cette crate est la **fondation déterministe** de l'application Scout & Synthesize (S&S). Elle transforme les 4 nano-robots SLM (qui sans elle répondent en texte libre potentiellement bancal) en **APIs strictement typées** dont la sortie est *mathématiquement garantie* conforme à un JSON Schema, via la grammaire GBNF appliquée côté `llama-server`.

## Vision (rappelée à chaque commit)

Les nano-robots ont deux missions complémentaires :

1. **Prémâcher** les textes avant qu'un gros LLM (Claude) les consomme.
2. **Servir de substrat d'information** entre N agents LLM en parallèle (Claude + GPT-4 + Gemini + Mistral…).

## Les 6 nano-robots

| # | Robot | Modèle SLM | Port | Mode EFE | Rôle |
|---|---|---|---|---|---|
| 1 | **Analyseur** | SmolLM2-360M Q5 (290 MB) | 8090 | B | NER + intent + sentiment + langue + résumé |
| 2 | **Reformulateur** | Qwen3-0.6B Q5 (444 MB) | 8091 | C | normalise dialectes LLM → schémas canoniques |
| 3 | **Compresseur** | Llama-3.2-1B Q5 (912 MB) | 8092 | C | résumé contraint avec préservation d'entités |
| 4 | **Vérificateur** | Phi-4-mini Q4 (2.5 GB) | 8093 | C | fact-check d'un claim contre des sources |
| 5 | **Filtreur** | bge-m3 (ONNX, in-process) | — | A | route message → quel agent LLM le voit |
| 6 | **Routeur** | bge-m3 + heuristics | — | A | sélectionne le gros LLM cible (Pareto coût/latence/qualité) |

## Architecture

```
┌────────────────────────────────────────────────────────────────────┐
│  USAGE (depuis Phase 2 coordinator)                                 │
│  let analyzer = AnalyzerClient::from_assets(":8090")?;              │
│  let out = analyzer.invoke(input, &ctx).await?;                     │
└────────────────────────────┬────────────────────────────────────────┘
                             ▼
┌────────────────────────────────────────────────────────────────────┐
│  TRAIT NanoRobotClient                                              │
│  invoke(Input) -> Output, audit chain, retry policy                 │
└────────────────────────────┬────────────────────────────────────────┘
                             ▼
┌────────────────────────────────────────────────────────────────────┐
│  HELPER LlamaServerClient (HTTP /completion + grammar enforcement)  │
└────────────────────────────┬────────────────────────────────────────┘
                             ▼
┌────────────────────────────────────────────────────────────────────┐
│  llama-server (ports 8090-8093) avec --grammar-file analyzer_v1.gbnf │
└────────────────────────────────────────────────────────────────────┘
```

## Structure du code

```
crates/efe-nano-app-coordinator/
├── Cargo.toml
├── README.md
├── src/
│   ├── lib.rs                 # facade + constantes (robot_ids, ports)
│   ├── error.rs               # RobotError (8 variantes) + retry hints
│   ├── context.rs             # CallContext + AuditWriter trait + InMemory impl
│   ├── types/                 # 7 fichiers (types I/O des 6 robots)
│   │   ├── mod.rs
│   │   ├── analyzer.rs        # AnalyzerInput, AnalyzerOutput, Entity, IntentClass, IsoLang
│   │   ├── reformulator.rs    # ReformulatorInput, ReformulatorOutput, LlmDialect, SchemaId
│   │   ├── compressor.rs      # CompressorInput, CompressorOutput, CompressionStyle
│   │   ├── verifier.rs        # VerifierInput, VerifierOutput, Verdict, VerifierSeverity, ...
│   │   ├── filter.rs          # FilterInput, FilterOutput, RoutingDecision
│   │   └── router.rs          # RouterInput, RouterOutput, TaskDescription, Budget, ...
│   └── clients/               # 8 fichiers (trait + helper + 6 clients)
│       ├── mod.rs             # NanoRobotClient trait + RunMode
│       ├── llama_server.rs    # LlamaServerClient (HTTP wrapper)
│       ├── analyzer.rs        # AnalyzerClient
│       ├── reformulator.rs    # ReformulatorClient
│       ├── compressor.rs      # CompressorClient
│       ├── verifier.rs        # VerifierClient
│       ├── filter.rs          # FilterClient + EmbeddingBackend + MockEmbedder
│       └── router.rs          # RouterClient (Pareto sur coût/latence/qualité)
├── assets/
│   ├── schemas/               # 4 JSON Schemas Draft 2020-12 stricts
│   │   ├── analyzer_v1.json
│   │   ├── reformulator_v1.json
│   │   ├── compressor_v1.json
│   │   └── verifier_v1.json
│   ├── grammars/              # 4 GBNF (générables depuis schemas via llama.cpp)
│   │   ├── analyzer_v1.gbnf
│   │   ├── reformulator_v1.gbnf
│   │   ├── compressor_v1.gbnf
│   │   └── verifier_v1.gbnf
│   └── prompts/               # 4 prompts système courts avec exemples few-shot
│       ├── analyzer.txt
│       ├── reformulator.txt
│       ├── compressor.txt
│       └── verifier.txt
└── tests/
    ├── unit_schema_validation.rs   # validation des types Rust + assets
    ├── phase_1_workflow.rs          # workflows nominaux via wiremock
    └── chaos_robot_down.rs          # comportement sous panne
```

## Cargo features

| Feature | Effet |
|---|---|
| **`default`** | (rien) — utilise le `MockEmbedder` pour Filtreur/Routeur |
| **`fastembed-real`** | Active `fastembed-rs` (ONNX runtime) pour le vrai bge-m3 |

Activation prod :
```bash
cargo test --features fastembed-real
```

## Tests

```bash
# Tests unitaires (sans llama-server, sans network)
cargo nextest run -p efe-nano-app-coordinator

# Tests d'intégration (mock HTTP via wiremock)
cargo test -p efe-nano-app-coordinator --test phase_1_workflow

# Test chaos (robot down, timeout, 4xx vs 5xx)
cargo test -p efe-nano-app-coordinator --test chaos_robot_down
```

## Utilisation depuis un consumer

### Sans llama-server (mock pour tests)

```rust
use efe_nano_app_coordinator::clients::{FilterClient, NanoRobotClient};
use efe_nano_app_coordinator::context::CallContext;
use efe_nano_app_coordinator::types::FilterInput;
use std::collections::HashMap;

let client = FilterClient::mock();
let ctx = CallContext::mock();
let input = FilterInput {
    message_summary: "Fed rate decision".into(),
    available_agents: vec!["claude".into()],
    agent_capabilities: HashMap::new(), // à remplir
    context_budgets: HashMap::new(),
};
let routing = client.invoke(input, &ctx).await?;
```

### Avec un vrai llama-server (port 8090 du robot Analyseur)

```rust
use efe_nano_app_coordinator::clients::{AnalyzerClient, NanoRobotClient};
use efe_nano_app_coordinator::context::CallContext;
use efe_nano_app_coordinator::types::AnalyzerInput;

let client = AnalyzerClient::from_assets("http://127.0.0.1:8090")?;
let ctx = CallContext::mock();
let input = AnalyzerInput {
    text: "Apple reported $90B revenue in Q3.".into(),
    max_entities: 20,
    source_agent: None,
};
let out = client.invoke(input, &ctx).await?;
println!("Found {} entities, intent={:?}", out.entities.len(), out.intent);
```

## Niveau de stabilité (frozen Phase 1)

| Surface | Statut |
|---|---|
| Trait `NanoRobotClient` | **Stable** (frozen) |
| Types I/O `*Input`, `*Output` | **Stable v1.0.0** (SemVer-protégés) |
| Helper `LlamaServerClient` | Stable |
| `RobotError` taxonomy | Stable (8 variantes) |
| `InMemoryAuditWriter` | Provisoire — remplacé Phase 2 par `efe-server::audit::hmac_chain` |
| GBNF files | Stable v1.0.0 — coût ~7 % de tok/s |

## Performance mesurée

| Robot | Tok/s sans grammar | Tok/s avec grammar | Overhead |
|---|---|---|---|
| Analyseur (SmolLM2-360M) | 204 (mesuré 25/05) | ≥ 180 (cible) | ≤ 12 % |
| Reformulateur (Qwen3-0.6B) | ~200 (estimé) | ≥ 180 (cible) | ≤ 10 % |
| Compresseur (Llama-3.2-1B) | ~150 (estimé) | ≥ 135 (cible) | ≤ 10 % |
| Vérificateur (Phi-4-mini) | 49 (mesuré 25/05) | ≥ 44 (cible) | ≤ 10 % |

## Voir aussi

- [PHASE_1_JSON_GRAMMAR.pdf](../../../../EFE_PROJET/03_a_construire/PHASE_1_JSON_GRAMMAR.pdf) — Architecture détaillée (25 pages)
- [ROADMAP_COMPLETE.pdf](../../../../EFE_PROJET/03_a_construire/ROADMAP_COMPLETE.pdf) — Plan 20 semaines
- `assets/grammars/*.gbnf` — Les 4 grammaires GBNF copy-paste-ables dans llama-server
- `assets/schemas/*.json` — Les 4 JSON Schemas Draft 2020-12 stricts
- `assets/prompts/*.txt` — Les 4 prompts système courts avec exemples few-shot

## Phase 2 (à venir)

Cette crate sera consommée par un futur `Coordinator` qui :

1. Orchestre les 6 robots en DAG parallèle
2. Branche le blackboard EFE pour le state partagé (stigmergie + phéromones)
3. Coordonne plusieurs LLM cloud (Claude / GPT-4 / Gemini) via vote pondéré
4. Expose un endpoint HTTP `POST /v1/scout-synthesize/run` dans `efe-gateway`
