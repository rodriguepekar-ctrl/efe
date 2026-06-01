//! M29 — BYOK key rotation API.
//!
//! Customers trigger key rotation via `POST /v1/byok/rotate` ; the
//! worker re-encrypts every encrypted column under the new key. During
//! rotation, BOTH old + new keys can decrypt for a 90-day grace window.

pub mod handlers;
pub mod state;
