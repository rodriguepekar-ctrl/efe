//! M26 — REST gateway namespace `v1`. Houses the axum routers exposed
//! at `/v1/...` URLs, distinct from `crate::proto::v1` (which contains
//! tonic-generated gRPC stubs from `protos/efe/v1/`).

pub mod agent;
/// M29 — Audit log export sink management.
pub mod audit_export;
/// M28 — Async batch classify : S3 input → worker → S3 output.
pub mod batch;
/// M31 — Stripe-backed billing self-service.
pub mod billing;
/// M29 — BYOK key rotation API.
pub mod byok;
/// M29 — GDPR Art. 17 erasure + Art. 20 portability.
pub mod data_rights;
/// M28 — Customer-facing event log : paginated read of recent events.
pub mod events;
/// M33 — GraphRAG : multi-hop retrieval fusing M32 + M33.
pub mod graphrag;
/// M33 — Knowledge graph CRUD + extraction + traversal.
pub mod kg;
/// M30 — Multimodal inference (vision + audio + long-text + fusion).
pub mod multimodal;
/// Sprint **D19 (P2-Foundation)** — Self-service activation funnel.
/// Cf. `docs/EFE_P2_SELF_SERVICE_ARCHITECTURE.md`.
pub mod onboarding;
/// M30 — Multi-region health + manual failover + DR drill.
pub mod region;
/// M29 — Sub-processor list + change notifications (GDPR Art. 28).
pub mod subprocessors;
/// M31 — Real-time usage + budget alerts.
pub mod usage;
/// M32 — Vector search collections + hybrid retrieval.
pub mod vector;
/// M28 — Webhook subscription system : per-tenant subscribe to events
/// (`agent.run.*`, `classify.completed`, `drift.detected`, ...) with
/// HMAC-signed delivery, retry, circuit breaker.
pub mod webhooks;
