# EFE — Ephemeral Fluid Swarm

> Inférence multi-agent par communication en espace latent partagé.

[![CI](https://github.com/efe-ai/efe/actions/workflows/ci.yml/badge.svg)](https://github.com/efe-ai/efe/actions)

EFE (Essaim Fluide Éphémère) remplace la chaîne d'agents LLM textuelle par
une population de **nano-agents éphémères** qui communiquent via un
**espace latent partagé** et convergent vers un consensus tensoriel en
millisecondes.

## Quickstart (5 minutes)

```bash
git clone git@github.com:efe-ai/efe.git
cd efe
just bootstrap     # installe toolchains, deps, hooks (5-10 min)
just test-smoke    # vérification rapide
```

Pour builder tout le workspace :

```bash
just build            # Rust + Python (CUDA optionnel)
just build-cuda       # inclut les kernels CUDA (requiert GPU)
```

Pour lancer les services de dev (OTel, Prometheus, Jaeger) :

```bash
just docker-up
```

## Structure du repo

```
crates/          Rust workspace
  efe-core/      types, erreurs, orchestration (M1, M3-M7, M9, M11)
  efe-kernels/   bindings CUDA + kernels (Sprint 1+)
  efe-ffi/       bindings PyO3 pour Python
  efe-server/    serveur gRPC (M10)
python/efe/      SDK Python + templates ML (M8)
protos/          Définitions gRPC
configs/         Configurations par environnement
docker/          Images Docker dev + prod
docs/            Documentation mkdocs (https://docs.efe.ai)
tests/           Tests d'intégration
scripts/         Scripts utilitaires (hooks, release)
```

## Documentation

- [Quickstart](docs/docs/quickstart.md)
- [Architecture](docs/docs/concepts/architecture.md)
- [Glossaire](docs/docs/concepts/glossary.md)
- [Contributing](CONTRIBUTING.md)
- [ADRs](docs/docs/adr/index.md)

## Commandes principales (`just`)

| Recipe | Rôle |
|--------|------|
| `just bootstrap` | Installe tout (rustup, uv, pre-commit, etc.) |
| `just build` | Build Rust + Python |
| `just build-cuda` | Build kernels CUDA (GPU requis) |
| `just test` | Exécute fast + unit tests |
| `just test-smoke` | Vérification rapide (<10s) |
| `just bench` | Benchmarks |
| `just fmt` | Formate tout |
| `just lint` | Linters (fail si warning) |
| `just clean` | Purge les artefacts |
| `just docker-up` | Démarre dépendances locales |
| `just docs-serve` | Serve docs localement sur :8000 |

## Versions supportées

- Rust 1.85 (pinned via `rust-toolchain.toml`)
- Python 3.12 (géré par `uv`)
- CUDA 12.4 (optionnel pour dev CPU-only)

## License

Proprietary — see [LICENSE](LICENSE).
