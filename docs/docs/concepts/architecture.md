# Architecture

Vue logique du système EFE, découpée en 12 modules.

## Vue d'ensemble

```
+--------------------------------------------------+
|              M10 API / gRPC                      |
|    tonic server        efe-client (Python)       |
+------------------------+-------------------------+
                         |
                         v
+--------------------------------------------------+
|            M1 Core Runtime (Rust)                |
|       SwarmController - orchestration            |
+------------+-------------+-----------------------+
             |             |              |
             v             v              v
       +-----------+  +----------+  +-------------+
       | M3        |  | M5       |  | M7          |
       | Genesis   |->| Fusion   |->| Dissolution |
       +-----+-----+  +----+-----+  +------+------+
             |             |               |
             v             v               v
      +----------------------------------------+
      |    M4 Espace Latent Partagé (ELS)      |
      |    LatentTensor BF16 [N,D] en HBM      |
      +-----------------+----------------------+
                        |
                        v
      +-------------------------------------+
      |   M2 Memory Pool + Streams CUDA     |
      +-------------------------------------+

Transverse :
  M6 Consensus   M8 Templates
  M9 Alignment   M11 Telemetry
  M12 Build / CI / Deploy
```

## Les 12 modules

| Module | Responsabilité | Langage |
|--------|----------------|---------|
| M1 Core Runtime | Types, erreurs, orchestration | Rust |
| M2 Memory Pool | Arenes HBM, stream pool | Rust + CUDA |
| M3 Genesis | Décomposition, routing, spawn | Rust + Python |
| M4 ELS | Espace latent, priors, projection | Rust + CUDA |
| M5 Fusion | Swarm Attention, Langevin | Triton + Rust |
| M6 Consensus | Détection de convergence | Rust |
| M7 Dissolution | Frechet mean, décodage | Rust + Python |
| M8 Templates | Nano-agents spécialisés | Python |
| M9 Alignment | Constitution, sandbox, budgets | Rust |
| M10 API | gRPC server, SDK Python | Rust + Python |
| M11 Telemetry | Logs, métriques, traces | Rust + Python |
| M12 Build/Deploy | Cargo, CMake, CI, Docker | Rust + YAML |

## Flux d'exécution (fin Sprint 7)

1. Client envoie `ExecuteRequest` via gRPC.
2. `SwarmController::execute(req)` — M1 orchestre.
3. `GenesisEngine::spawn` — M3 décompose et crée l'essaim dans l'ELS.
4. Boucle de rounds (1 à 8) :
   - `FusionStrategy::run_round` — M5 applique Swarm Attention + Langevin.
   - `ConsensusDetector::evaluate` — M6 décide si on continue.
5. `Dissolver::dissolve` — M7 extrait le centroïde, projette, décode.
6. Réponse gRPC + flush de la télémétrie — M10 + M11.

## Principes de couplage

- **P1** : Le Core Runtime ne connaît que des traits, jamais des implémentations.
- **P2** : Tout tenseur reste en HBM (jamais côté host sauf dissolution).
- **P3** : Alignment et Telemetry sont des cross-cutting concerns (middleware).
- **P4** : Rust ↔ Python via PyO3 uniquement, jamais JSON sur le hot path.

## En savoir plus

- [Glossaire](glossary.md) : définitions des termes techniques.
- [ADR-001](../adr/ADR-001-tech-stack.md) : choix de la stack.
- Documents internes : `EFE_Architecture_Ultra_Think.pdf`,
  `EFE_MVP_Sous_Modules.pdf`, `EFE_Coding_Roadmap.pdf`.
