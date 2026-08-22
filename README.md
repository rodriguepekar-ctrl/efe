# EFE — Ephemeral Fluid Swarm

> Inférence multi-agent par communication en espace latent partagé.

[![CI](https://github.com/efe-ai/efe/actions/workflows/ci.yml/badge.svg)](https://github.com/efe-ai/efe/actions)

EFE (Essaim Fluide Éphémère) remplace la chaîne d'agents LLM textuelle par
une population de **nano-agents éphémères** qui communiquent via un
**espace latent partagé** et convergent vers un consensus tensoriel en
millisecondes.

## Scella — le reçu souverain, en **une** commande

Sur une machine où rien n'est installé, Docker excepté :

```bash
docker compose up
```

Vous verrez d'abord la passerelle **annoncer ce qu'elle sert réellement** — son
rapport de démarrage nomme chaque capacité et ce que son absence coûte. Puis une
démonstration : un run, son reçu, son export, et une **troncature démasquée par
un simple comptage — sans aucune clé, sans cryptographie, sans accorder sa
confiance à qui que ce soit**.

Cette dernière propriété est la seule qui vaille d'être regardée. Le sceau d'une
chaîne a une faiblesse que rien, de l'intérieur, ne trahit : *le préfixe d'une
chaîne valide est lui-même valide*. Un bundle tronqué se re-prouve parfaitement.
Le seul témoin possible est **hors** du bundle — l'ancre publiée à l'export, qui
annonce le nombre de nœuds. La confronter est un comptage, que vous refaites
vous-même avec `grep -c` et `awk` sur les fichiers laissés dans le volume.

> **Ce que cette démonstration ne prouve PAS.** Aucune clé n'est fournie : la
> passerelle tourne sur la clé de **test publique**, connue de quiconque a lu ce
> dépôt. Le reçu produit démontre le **mécanisme**, jamais la **substance** — il
> n'oppose rien à personne. Pour cela il faut une clé que vous seul détenez :
> `EFE_NANO_AUDIT_HMAC_MASTER=$(openssl rand -hex 32) docker compose up`.
> Et ce qu'un reçu atteste, même avec une vraie clé, reste l'**intégrité** de ce
> qui a été enregistré — jamais la **correction** de la décision.

Le détail se lit dans [`compose.yaml`](compose.yaml),
[`profiles/docker.env`](profiles/docker.env) et
[`crates/scella-preuve`](crates/scella-preuve). Sans démon Docker, la charge
utile — passerelle, reçu, ancre, détection de troncature — se vérifie seule :
`bash scripts/m4-check.sh`.

---

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
