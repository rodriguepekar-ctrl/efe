//! Optional gRPC client (efe-server direct calls).
//!
//! Sprint A scope : minimal — used only for the smoke-test command
//! which performs N classify calls directly via gRPC. All other
//! commands route through the mcp-server `m23_*` tools.
//!
//! For the v1.0.0 ship we use a thin REST gateway endpoint exposed
//! by efe-server (`/v1/classify`) instead of a full tonic client to
//! keep the binary footprint small. Future versions can swap in a
//! tonic-based path.

mod client;
pub mod active_learning_client;
pub mod agent_client;
pub mod curation_client;

pub use client::{ClassifyRequest, ClassifyResponse, GrpcClient};
