//! Command dispatch — one module per top-level subcommand.
//!
//! Each module exposes a `dispatch` (or `run`) function that receives
//! the parsed args + the `AdminContext` and returns `Result<(), AdminError>`.

pub mod agent;
pub mod audit;
pub mod catalog;
pub mod compliance;
pub mod curation;
pub mod health;
pub mod keys;
pub mod learning;
pub mod login;
pub mod ops;
pub mod quotas;
pub mod smoke;
pub mod tenant;
pub mod version;
