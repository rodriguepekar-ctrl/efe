//! PR #20 § 6.2 / § 12 — gRPC client pool + mTLS bootstrap.
//!
//! Wraps tonic-generated stubs for `MetaSwarmService` (controller) and
//! `MetaParticipantService`.  Maintains N persistent HTTP/2 connections
//! per target (AD22) ; weighted-random target selection per request.

pub mod pool;
pub mod tls;

pub use pool::{
    ClientError, ClientPool, IterateResponseSummary, ProtoIterateInput, ProtoStepInput,
    StateResponseSummary, StepResponseSummary, SimpleResponseSummary,
};
