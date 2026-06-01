//! Catalog implementations.
//!
//! Re-exports the [`efe_nano_sdk::Catalog`] trait + provides
//! [`in_memory::InMemoryCatalog`] (Sprint 1) and (Sprint 2)
//! `postgres::PostgresCatalog`.

pub mod in_memory;

pub use efe_nano_sdk::traits::catalog::{Catalog, CatalogResult, ListFilter};
pub use in_memory::InMemoryCatalog;
