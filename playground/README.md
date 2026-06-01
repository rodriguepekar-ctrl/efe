# EFE Playground

> **Try EFE 1.0 GA locally in your browser. No K8s required.**

[![Status: Stable](https://img.shields.io/badge/Status-Stable_1.0_GA-success)](#)

The EFE Playground is a self-contained environment for trying EFE's
multi-tenant semantic classification API. It bundles a Python Flask backend
+ vanilla-JS frontend + 4 ready-to-use sample catalogs, runnable in 60 seconds.

---

## ✨ What you can do

- 🔐 **Multi-tenant** : sign in as 3 demo tenants (`acme` / `beta-corp` / `demo-fr`)
- 📚 **Catalog management** : create + view + delete catalogs through the web UI
- 🎯 **Classify** : real-time text classification with confidence bars
- 🧬 **Encode** : visualize 384-dim embedding vectors
- 📦 **Samples** : 4 ready-to-use catalogs (support routing, content moderation, intents, multilingual)
- 🔍 **Audit log** : tamper-evident HMAC chain (D14-bis pattern) — verify integrity in one click
- 📊 **Dashboard** : live metrics — catalogs / events / chain status / action breakdown
- 📖 **5 tutorials** : 30 min total to learn EFE end-to-end

---

## 🚀 Quick start (60 seconds)

```bash
# From the repo root :
bash playground/scripts/quickstart.sh
```

Then open : **http://localhost:8765/**

The script :
1. Creates a Python venv at `playground/.venv/`
2. Installs Flask + Flask-CORS + numpy
3. Starts the backend on port 8765
4. Auto-detects whether `sentence-transformers` is installed (fallback: hash simulator)

---

## 🐳 Docker (alternative)

```bash
docker compose -f playground/docker-compose.yml up
```

Open : http://localhost:8765/

For real semantic embeddings (~500 MiB image with `sentence-transformers`) :
```bash
INSTALL_REAL_EMBEDDER=yes docker compose -f playground/docker-compose.yml up --build
```

---

## 🧠 Two embedder backends

The playground auto-selects the best embedder available :

| Backend | Use case | Quality | Setup |
|---|---|---|---|
| **`sentence-transformers`** | Production-like demo | ✅ Real semantic | `pip install sentence-transformers torch` (~500 MiB) |
| **`hash` (fallback)** | Offline / no-deps | ⚠ Pseudo (deterministic hash) | None — works out of the box |

Both produce 384-dim L2-normalized vectors so the cosine-similarity API surface
is identical. Real backend is much more accurate ; fallback is for testing the
UI when you can't install heavy deps.

To force a backend :
```bash
EFE_PLAYGROUND_BACKEND=hash bash playground/scripts/quickstart.sh
EFE_PLAYGROUND_BACKEND=sentence-transformers bash playground/scripts/quickstart.sh
```

---

## 📚 Tutorials

Read in order :

1. [00 — Introduction](tutorials/00_introduction.md) (5 min)
2. [01 — Your first catalog](tutorials/01_first_catalog.md) (10 min)
3. [02 — Multi-tenant isolation](tutorials/02_multi_tenant.md) (10 min)
4. [03 — Drift detection](tutorials/03_drift_detection.md) (15 min)
5. [04 — Production deployment](tutorials/04_production_deploy.md) (30 min)

---

## 🏗️ Architecture

```
playground/
├── backend/                # Flask Python (~1 200 LOC)
│   ├── app.py              # REST endpoints + CORS
│   ├── embedder.py         # sentence-transformers OR hash fallback
│   ├── catalog.py          # Per-tenant in-memory catalog + versioning
│   ├── audit.py            # HMAC-SHA-256 chain (D14-bis mirror)
│   ├── auth.py             # Mock OIDC tokens (NOT cryptographic)
│   └── requirements.txt
├── frontend/               # SPA (~1 600 LOC)
│   ├── index.html          # Single-page app shell
│   ├── style.css           # Modern dark theme, no framework
│   ├── api.js              # fetch-based API client
│   └── app.js              # View routing + UI logic
├── samples/                # 4 ready-to-use catalogs
│   ├── support_routing.json
│   ├── content_moderation.json
│   ├── intent_recognition.json
│   └── multilingual_intents.json
├── scripts/
│   └── quickstart.sh       # One-command start
├── tutorials/              # 5 markdown walkthroughs
├── tests/                  # E2E smoke tests
├── Dockerfile              # Single-container all-in-one
└── docker-compose.yml      # Compose with security hardening
```

---

## 🆚 Playground vs production EFE

The playground **mirrors** production EFE patterns but is simplified for
learning + offline use :

| Concern | Playground | Production EFE 1.0 GA |
|---|---|---|
| Backend | Flask in-memory | Tonic gRPC server (Rust, `crates/efe-server/`) |
| Storage | dict | Postgres (D14) + connection pool |
| Auth | mock OIDC tokens | Real OIDC + JWT + JWKS rotation (D15) |
| Embedder | sentence-transformers / hash | Bundle-based ONNX runtime (M13) |
| HA | single process | HPA 3-30 replicas + leader election (D9 + D18) |
| TLS | none | TLS 1.3 + mTLS (M12 D22) |
| Network policy | none | strict default-deny (M12 D21) |
| Audit log | in-memory ring (1k events) | HMAC chain + Postgres + Object Lock 7-yr |
| Backup | none | D17 + D40 quarterly drill |
| Compliance | dev only | SOC 2 + ISO 27001 + GDPR + AI Act + NIST |

**Tutorial 04 walks you through the migration** from playground to a real K8s deployment.

---

## 🧪 Smoke tests

```bash
# Run from repo root
python3 -m pytest playground/tests/ -v
```

Or via the parent test suite :
```bash
python3 -m pytest scripts/tests/test_m15_*.py -v
```

---

## 🔒 Security note

The playground uses :
- **Mock OIDC tokens** : NOT cryptographically signed ; trivial to forge.
- **Demo HMAC key** : `playground-dev-audit-key-do-not-use-in-prod` (override via env).
- **In-memory storage** : data lost on restart.
- **No TLS** : all traffic is HTTP.

**Do NOT use the playground in production**. For production, see :
- `docs/RELEASE_NOTES_1.0.md` — GA release notes
- `docs/EFE_HELM_CHART_EXTENSION_ARCHITECTURE.md` — Helm chart deployment
- `docs/HANDOFF_OPS.md` — operations team handoff

---

## 📄 License

Apache-2.0 — see [LICENSE](../LICENSE) at repo root.

---

## 🐛 Issues

File issues at : https://github.com/efe-project/efe/issues
