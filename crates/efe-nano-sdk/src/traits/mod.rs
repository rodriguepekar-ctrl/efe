//! Traits defining the public API surface of the M50 platform.
//!
//! - [`catalog::Catalog`] : the registry of nano agents.
//! - [`runtime::Runtime`] : the executor.

pub mod catalog;
pub mod runtime;

pub use catalog::{Catalog, CatalogResult, ListFilter, ResolvedAgent};
pub use runtime::{NanoAgentResponse, PoolHealth, Runtime, RuntimeHealth, RuntimeResult, TraceId};
