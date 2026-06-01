# `efe-multi-llm` — Phase 3 — Coordination multi-LLM

Crate Rust qui livre la **coordination multi-LLM réelle** par-dessus le
Coordinator Phase 2. 3 LLMs (Claude / GPT / Gemini) collaborent
coordonnés par les nano-robots Phase 1 + 2 nouveaux nano-robots
(`Decomposer`, `Aggregator`).

C'est la **réalisation littérale** de la mission utilisateur :

> "Nano-robots qui prémâchent le travail des agents LLM entre eux."

## Les 5 missions exposées

| Mission | Fonction | Cas d'usage |
|---|---|---|
| `run_vote_3llm` | Fan-out 3 LLMs + reformulation pré-vote + Borda/Condorcet | Question factuelle où la diversité réduit l'hallucination |
| `run_relay` | Claude planifie → GPT code → Gemini synthétise (séquentiel typé) | Tâche complexe nécessitant plusieurs expertises |
| `run_debate` | 2 LLMs débattent (rounds + rebut) + 1 LLM jury | Décisions à fort enjeu, design ou éthique |
| `run_specialist` | Decomposer split → routing par sub-task → Aggregator merge | Requête complexe avec sub-tasks de natures différentes |
| `run_cascade` | Drafter local (free) → GPT verifier → Claude validator (rare) | Optimisation latence : -41% vs Claude solo |

## Composants

| Module | Responsabilité |
|---|---|
| `canonical` | 5 schemas frozen v1.0.0 : plan, code, claim, argument, synthesis |
| `reformulator_ext` | Convertit dialecte LLM (XML/JSON/MD) → canonical schemas |
| `vote_ext` | Algos Borda + Condorcet (extension du Plurality Phase 2) |
| `budget` | `BudgetTracker` atomic avec try_charge + revert (invariant I13) |
| `tenant` | `TenantPheromoneBlackboard` isolation physique per-tenant (I12) |
| `cancellation` | `CancellationToken` pour early termination (I9) |
| `decompose` | `Decomposer` — découpe un prompt complexe en sub-tasks |
| `aggregate` | `Aggregator` — combine N canonical outputs en synthesis |

## Invariants Phase 3

- **I9** — Early termination : si vote décidé avec n-1 LLMs, le n-ième est cancelé.
- **I10** — Audit chain complète : `audit_chain_hash` toujours retourné.
- **I11** — Anti-monoculture : un LLM ne représente jamais > 60 % des sub-tasks.
- **I12** — Isolation tenant : phéromones tenant A invisibles à tenant B.
- **I13** — Budget hard-cap : try_charge atomic + revert si dépassement.

## Quick start

```rust
use std::sync::Arc;
use efe_coordinator::{Coordinator, MockLlmProvider, ProviderRegistry};
use efe_multi_llm::*;
use efe_nano_app_coordinator::types::router::TaskClass;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 1. Enregistre 3 LLMs (mock ou vrais).
    let reg = ProviderRegistry::new();
    reg.register(Arc::new(MockLlmProvider::new("claude").with_response("Paris.")));
    reg.register(Arc::new(MockLlmProvider::new("gpt").with_response("Paris.")));
    reg.register(Arc::new(MockLlmProvider::new("gemini").with_response("Paris.")));

    // 2. Coordinator Phase 2.
    let coord = Coordinator::builder().registry(reg).build()?;

    // 3. Mission Vote 3-LLM avec mode Borda.
    let req = Vote3LlmRequest {
        user_text: "Capital of France?".into(),
        task_class: TaskClass::FactualQa,
        llms: vec!["claude".into(), "gpt".into(), "gemini".into()],
        mode: VoteMode::Borda,
        quorum: Some(2),
        max_budget_cents: 100,
        max_total_latency: std::time::Duration::from_secs(5),
    };
    let resp = run_vote_3llm(&coord, req).await?;
    println!("Winner: {} → {}", resp.winner_llm, resp.final_text);
    Ok(())
}
```

## Canonical schemas v1.0.0

5 schemas JSON Schema Draft 2020-12 dans `assets/schemas/canonical/` :

- `plan_v1.json` — `{plan_id, title, steps: [{id, action, deps}], rationale}`
- `code_v1.json` — `{language, source, tests, deps, complexity_estimate}`
- `claim_v1.json` — `{claim, evidence: [...], confidence, domain}`
- `argument_v1.json` — `{thesis, reasoning, sources, stance: PRO|CONTRA|NEUTRAL|REBUTTAL}`
- `synthesis_v1.json` — `{summary, citations, uncertainty, sections}`

Le `CanonicalReformulator` parse Claude XML / GPT JSON / Gemini Markdown
et produit l'un de ces 5 types canoniques.

## Architecture multi-LLM

```
USER request
   ↓
┌─────────────────────────────────────────────────────────────────┐
│  Coordinator Phase 2 + workflows Phase 3                         │
└───────────────────────────┬─────────────────────────────────────┘
                            ▼
┌─────────────────────────────────────────────────────────────────┐
│  PRE-CHEW (nano-robots Phase 1)                                  │
│  Analyzer • Filter • Router • Compressor                         │
└───────────────────────────┬─────────────────────────────────────┘
                            ▼
┌─────────────────────────────────────────────────────────────────┐
│  3 LLMs en parallèle / séquentiel / cascade                      │
│  Claude (XML, raisonnement) ╱╱╱ GPT (JSON, code) ╱╱╱ Gemini      │
└───────────────────────────┬─────────────────────────────────────┘
                            ▼
┌─────────────────────────────────────────────────────────────────┐
│  CROSS-DIALECT NORMALISATION (Reformulator × 3)                  │
│  XML → canonical JSON  •  JSON → canonical  •  MD → canonical    │
└───────────────────────────┬─────────────────────────────────────┘
                            ▼
┌─────────────────────────────────────────────────────────────────┐
│  AGGREGATE (Vote/Relay/Debate/Specialist/Cascade)                │
│  VoteEngine (Borda/Condorcet) • Aggregator • Verifier            │
└───────────────────────────┬─────────────────────────────────────┘
                            ▼
                    Final response (SSE streamed)
```

## Tests

```bash
PYO3_PYTHON=$(which python3.12) cargo test -p efe-multi-llm
```

| Binaire | Tests | Focus |
|---|---|---|
| `lib` (inline) | 91 | unit per-module |
| `canonical_schemas` | 12 | présence + validation JSON Schema |
| `unit_vote_ext` | 7 | Borda + Condorcet algos |
| `unit_reformulator_ext` | 10 | parsing XML/JSON/MD |
| `unit_budget` | 8 | atomic try_charge + revert (I13) |
| `unit_tenant` | 5 | isolation physique (I12) |
| `unit_cancellation` | 6 | early termination tokens (I9) |
| `unit_decompose` | 8 | split + classify |
| `unit_aggregate` | 7 | merge + Jaccard uncertainty |
| `workflow_vote_3llm` | 7 | Plurality/Borda/Condorcet |
| `workflow_cascade` | 6 | drafter + verifier + validator |
| `workflow_relay` | 6 | Plan/Code/Synthesis sequence |
| `workflow_debate` | 7 | rounds + jury verdict |
| `workflow_specialist` | 5 | decompose + route + aggregate |
| `chaos_multi_llm` | 9 | 1 LLM down / 2 down / all down |
| `invariants_phase3` | 6 | I9–I13 transverses |

## Bench

```bash
PYO3_PYTHON=$(which python3.12) cargo bench -p efe-multi-llm
```

7 groupes Criterion : `vote`, `borda`, `condorcet`, `reformulator`,
`budget`, `decompose`, `cascade`.

## Stabilité

| Composant | Statut |
|---|---|
| 5 canonical schemas | **Frozen v1.0.0** |
| 5 fonctions `run_*` | **Stable** Phase 3 |
| `BudgetTracker` API | Stable |
| `vote_ext` Borda/Condorcet | Stable |
| `TenantPheromoneBlackboard` | Stable |
| `Decomposer` / `Aggregator` (heuristic) | Stable ; LLM-backed Beta |

## Roadmap

- Phase 1 (livrée) — Nano-robots SLM.
- Phase 2 (livrée) — Coordinator + DAG + blackboard + SSE.
- Phase 3 (**ce crate**) — Coordination multi-LLM.
- Phase 4 (à venir) — Frontend SSE.
- Phase 7 (à venir) — Durcissement prod (NATS, Postgres, cosign).

Voir `EFE_PROJET/03_a_construire/PHASE_3_MULTI_LLM.pdf` (22 pages) pour
l'architecture complète.
