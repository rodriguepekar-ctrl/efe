//! M33 — REST gateway for the KG plane under `/v1/kg/*`.

pub mod errors;
pub mod handlers;
pub mod state;

pub use errors::KgApiError;
pub use handlers::router;
pub use state::KgApiState;
