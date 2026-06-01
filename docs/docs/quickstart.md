# Quickstart

## Prérequis

- macOS 14+ ou Ubuntu 22.04+
- Rust 1.85+ (installé par `rustup`)
- Python 3.12
- CMake 3.28+
- (Optionnel) GPU NVIDIA avec CUDA 12.4

## Installation

```bash
git clone git@github.com:efe-ai/efe.git
cd efe
just bootstrap
```

La commande `just bootstrap` installe :

1. Les dépendances système manquantes (via `brew` ou `apt`).
2. La toolchain Rust 1.85 avec `rustfmt`, `clippy`, `rust-analyzer`.
3. Cargo tools (`nextest`, `audit`, `taplo`).
4. L'environnement Python via `uv` (incluant extras dev).
5. Les hooks `pre-commit`.

Durée : 5–10 minutes selon la connexion.

## Vérification

```bash
just test-smoke
```

Doit afficher :

```
cargo check --workspace
...Checking efe-core v0.0.1 ...
...Finished `dev` profile
uv run python -c "import efe; print(f'efe {efe.__version__}')"
efe 0.0.1
Smoke OK
```

## Build complet

```bash
just build          # Rust + Python
just build-cuda     # + kernels CUDA (requiert GPU)
```

## Lancer les services locaux

```bash
just docker-up
```

Expose :

- `http://localhost:9090` — Prometheus
- `http://localhost:16686` — Jaeger UI
- `localhost:4317` — OTLP gRPC endpoint

## Premier essaim (Sprint 1+)

*Section disponible à partir du Sprint 1 (mid-juillet 2026).*

## Dépannage

| Problème | Solution |
|----------|----------|
| `rustc` introuvable | `source ~/.cargo/env` ou relancer le terminal |
| `uv` non trouvé | `curl -LsSf https://astral.sh/uv/install.sh \| sh` |
| CUDA build fail | Skip `build-cuda`, CPU-only fonctionne pour le dev |
| `pre-commit` échoue sur format | `just fmt` puis retenter |
