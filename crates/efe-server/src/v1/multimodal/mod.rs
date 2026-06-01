//! M30 — REST gateway for the multimodal inference plane under
//! `/v1/multimodal/*`.
//!
//! Routes :
//!
//! - `POST /v1/multimodal/vision/encode`     — SigLIP image embedding
//! - `POST /v1/multimodal/audio/encode`      — Whisper audio
//!   transcript + embedding
//! - `POST /v1/multimodal/text/chunk`        — Long-context chunker
//! - `POST /v1/multimodal/fuse`              — Late-fusion of N
//!   per-modality embeddings into a single 768-d vector
//! - `GET  /v1/multimodal/policy`            — Read tenant modality
//!   policy
//! - `PUT  /v1/multimodal/policy`            — Update tenant modality
//!   policy (admin scope required)
//!
//! Every handler authenticates the caller via the standard EFE
//! bearer-token middleware, then loads the per-tenant
//! `TenantModalityPolicy` from PG and enforces it (DPIA-gated, MIME
//! allow-list, size + duration caps).
//!
//! Raw image / audio bytes NEVER leave this module. Only the
//! SHA-256 first-16-hex prefix is logged. Cf. ADR-M30-002.

pub mod errors;
pub mod handlers;
pub mod state;

pub use errors::MultimodalApiError;
pub use handlers::router;
pub use state::MultimodalApiState;
