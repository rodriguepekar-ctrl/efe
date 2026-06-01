//! Auth — bearer token providers.
//!
//! Three modes :
//!   - `None` — for local dev / k3d.
//!   - `Token` — file-backed bearer token (POC default).
//!   - `KubeSa` — Kubernetes service-account token (CI/CD).
//!   - `Sso` — OAuth2 device flow (post-MVP, gated behind `sso` feature).

mod token;

pub use token::TokenProvider;
