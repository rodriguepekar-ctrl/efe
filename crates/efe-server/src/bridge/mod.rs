//! Bridge between the `efe.v1` proto types and the `efe_core` domain
//! types.
//!
//! All conversions are **infallible** when the domain type permits, and
//! return `Result<T, efe_core::EfeError>` otherwise. The gRPC service
//! layer is responsible for mapping `EfeError` → `tonic::Status` through
//! [`error::to_status`].

pub mod budget;
pub mod cone;
pub mod decode;
pub mod error;
pub mod request;
pub mod response;
pub mod spec;
pub mod verdict;

pub use error::to_status;
