//! M32 — REST gateway for the vector search plane under
//! `/v1/vector/*`.
//!
//! Routes :
//!
//! - `GET    /v1/vector/collections`                    — list collections
//! - `POST   /v1/vector/collections`                    — create
//! - `GET    /v1/vector/collections/{name}`             — inspect
//! - `DELETE /v1/vector/collections/{name}`             — drop
//! - `POST   /v1/vector/collections/{name}/documents`   — insert / upsert
//! - `DELETE /v1/vector/collections/{name}/documents/{id}` — delete
//! - `POST   /v1/vector/collections/{name}/search`      — top-k
//! - `POST   /v1/vector/collections/{name}/snapshot`    — force snapshot
//! - `GET    /v1/vector/collections/{name}/status`      — health + size
//!
//! All endpoints require a bearer-authed tenant. Collection name
//! is tenant-scoped (validated to `[a-z0-9_-]{1,32}`).

pub mod errors;
pub mod handlers;
pub mod state;

pub use errors::VectorApiError;
pub use handlers::router;
pub use state::VectorApiState;
