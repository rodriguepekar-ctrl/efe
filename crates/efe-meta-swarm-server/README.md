# `efe-meta-swarm-server`

Tonic-based gRPC servers for the EFE meta-swarm coordinator.

This crate ships **two binaries** under one Cargo crate :

- `controller`  — implements `MetaSwarmService`
- `participant` — implements `MetaParticipantService`

PR #6 (Sprint A1 Phase 2) lands the **scaffold** only.  Real business
logic wires in subsequent PRs (#8, #9, #11, #12, #16, #17, #18).

## Quick start

### Local — 3 participants + 1 controller

In four terminals :

```bash
# Participant 1 (supplier-scoring)
EFE_META_SWARM_PARTICIPANT_ID=supplier-scoring \
EFE_META_SWARM_PARTICIPANT_ADDR=0.0.0.0:50091 \
EFE_META_SWARM_PARTICIPANT_METRICS=0.0.0.0:9092 \
  cargo run -p efe-meta-swarm-server --bin participant

# Participant 2 (contract-risk)
EFE_META_SWARM_PARTICIPANT_ID=contract-risk \
EFE_META_SWARM_PARTICIPANT_ADDR=0.0.0.0:50092 \
EFE_META_SWARM_PARTICIPANT_METRICS=0.0.0.0:9093 \
  cargo run -p efe-meta-swarm-server --bin participant

# Participant 3 (spend-categorization)
EFE_META_SWARM_PARTICIPANT_ID=spend-categorization \
EFE_META_SWARM_PARTICIPANT_ADDR=0.0.0.0:50093 \
EFE_META_SWARM_PARTICIPANT_METRICS=0.0.0.0:9094 \
  cargo run -p efe-meta-swarm-server --bin participant

# Controller
EFE_META_SWARM_CONTROLLER_ADDR=0.0.0.0:50080 \
EFE_META_SWARM_CONTROLLER_METRICS=0.0.0.0:9091 \
EFE_META_SWARM_PARTICIPANTS=supplier-scoring=localhost:50091,contract-risk=localhost:50092,spend-categorization=localhost:50093 \
  cargo run -p efe-meta-swarm-server --bin controller
```

In a fifth terminal (using `grpcurl`) :

```bash
grpcurl -plaintext localhost:50080 efe.v1.meta_swarm.MetaSwarmService/Health
# {"status":"STATUS_SERVING","inflight":0,"reload_gen":0}

grpcurl -plaintext localhost:50080 efe.v1.meta_swarm.MetaSwarmService/Info
# {"buildSha":"abcdef12","protoVersion":"v1","leaderPodName":"hostname",...}

grpcurl -plaintext -d '{}' localhost:50080 efe.v1.meta_swarm.MetaSwarmService/Iterate
# ERROR: Unimplemented: MetaSwarmService::Iterate — wired in PR #8
```

The Prometheus endpoint :

```bash
curl http://localhost:9091/metrics
# (empty body — PR #17 will register the 14 metrics)
```

## Environment variables

### Controller

| Var | Default | Required? |
|---|---|:---:|
| `EFE_META_SWARM_CONTROLLER_ADDR` | `0.0.0.0:50080` | no |
| `EFE_META_SWARM_CONTROLLER_METRICS` | `0.0.0.0:9091` | no |
| `EFE_META_SWARM_DISCOVERY` | `static-localhost` | no |
| `EFE_META_SWARM_PARTICIPANTS` | (empty) | no |
| `EFE_META_SWARM_MANIFEST` | `/etc/efe-meta-swarm/manifest.yaml` | no |
| `EFE_META_SWARM_MAX_INFLIGHT` | `10` | no |
| `EFE_META_SWARM_DEFAULT_MAX_WALL_MS` | `15000` | no |
| `EFE_META_SWARM_WORKER_THREADS` | `2` | no |
| `EFE_META_SWARM_RELOAD_DRAIN_TIMEOUT_S` | `30` | no |
| `EFE_META_SWARM_REFLECTION` | `false` | no |
| `EFE_META_SWARM_TLS_REQUIRED` | `false` | no |
| `EFE_META_SWARM_TLS_CERT` | — | yes if `tls_required=true` |
| `EFE_META_SWARM_TLS_KEY` | — | yes if `tls_required=true` |
| `EFE_META_SWARM_TLS_CA` | — | no |
| `EFE_META_SWARM_LOG` | `info` | no |

### Participant

| Var | Default | Required? |
|---|---|:---:|
| `EFE_META_SWARM_PARTICIPANT_ID` | — | **YES** |
| `EFE_META_SWARM_PARTICIPANT_ADDR` | `0.0.0.0:50090` | no |
| `EFE_META_SWARM_PARTICIPANT_METRICS` | `0.0.0.0:9092` | no |
| `EFE_META_SWARM_WORKER_THREADS` | `1` | no |
| TLS vars | — | same as controller |

## Cargo features

| Feature | Default | What it does |
|---|:---:|---|
| `reflection` | off | Plumbing for `tonic-reflection`.  PR #6 only ; PR #18 ships the dep + endpoint. |

## Building locally

```bash
cargo build -p efe-meta-swarm-server --release
ls target/release/{controller,participant}
```

## Tests

```bash
cargo test -p efe-meta-swarm-server               # unit tests
cargo test -p efe-meta-swarm-server --test integration_health_smoke
```

The integration test spawns the `controller` binary on a random port
and asserts `Health` returns `STATUS_SERVING`.  Run with
`EFE_META_SWARM_LOG=info` for verbose output.

## Cross-references

- Phase 2 architecture : [`docs/EFE_A1_PHASE2_ARCHITECTURE.md`](../../docs/EFE_A1_PHASE2_ARCHITECTURE.md)
- Phase 2 roadmap : [`docs/EFE_A1_PHASE2_ROADMAP.md`](../../docs/EFE_A1_PHASE2_ROADMAP.md)
- PR #6 detailed architecture : [`docs/EFE_A1_PHASE2_PR06_ARCHITECTURE.md`](../../docs/EFE_A1_PHASE2_PR06_ARCHITECTURE.md)
- Proto crate : [`crates/efe-meta-swarm-proto/README.md`](../efe-meta-swarm-proto/README.md)
