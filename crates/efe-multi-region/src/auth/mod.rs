// SPDX-License-Identifier: UNLICENSED
//! **Sprint 9 — N16** : Operator JWT authentication for replays.
//!
//! Architecture : `docs/architecture/EFE_M50_N16_MULTI_REGION_REPLAY_ARCHITECTURE.md`
//! § Annex A2.
//!
//! # Token format
//!
//! Bearer JWT with HMAC-SHA256 signature.  Claims :
//! ```jsonc
//! {
//!   "iss": "efe-operator-idp",
//!   "sub": "alice@efe.com",
//!   "operator_id": "uuid",
//!   "tenant_id": "uuid",         // tenant the operator is acting on behalf of
//!   "scope": ["replay:execute", "replay:read"],
//!   "ticket_id": "JIRA-12345",
//!   "iat": 1716200000,
//!   "exp": 1716203600,           // 1h TTL by spec
//!   "jti": "unique-jwt-id"
//! }
//! ```
//!
//! # Why HS256 instead of RS256
//!
//! Sprint 9 ships HS256 with a shared signing secret — operationally
//! simpler for the initial multi-region deployment.  Sprint 10 will
//! migrate to RS256 backed by Vault Transit (architecture ADR-10).

pub mod claims;
pub mod validator;

pub use claims::{OperatorClaims, ReplayScope};
pub use validator::{Hs256JwtValidator, JwtError, JwtValidator};
