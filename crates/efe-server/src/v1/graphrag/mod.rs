//! M33 — REST gateway for the GraphRAG plane under `/v1/graphrag/*`.

pub mod errors;
pub mod handlers;
pub mod state;

pub use errors::GraphRagApiError;
pub use handlers::router;
pub use state::GraphRagApiState;
