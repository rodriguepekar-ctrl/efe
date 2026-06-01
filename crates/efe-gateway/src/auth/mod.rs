// SPDX-License-Identifier: UNLICENSED
//! **Sprint 14 — N22** : JWT bearer authentication — § 7.4 + ADR-2
//! + G5.
//!
//! HS256 only in Sprint 14 (RS256+JWKS deferred Sprint 15).

pub mod claims;
pub mod jwt;
pub mod layer;

pub use claims::{GatewayClaims, OperatorScope};
pub use jwt::{Hs256JwtValidator, JwtError, JwtValidator};
pub use layer::AuthLayer;

pub use crate::config::AuthMode as AuthConfig;
