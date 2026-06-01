// SPDX-License-Identifier: UNLICENSED
//! **Sprint 14 — N22** : endpoint handlers — § 9.

pub mod admin;
pub mod agentic;
/// **Sprint 19 — N27** : `POST /v1/agentic/call-stream` SSE handler.
pub mod agentic_stream;
pub mod gold;
/// **Sprint 18 — N26 + G13** : `GET /v1/llm-provenance` operator
/// handler.
pub mod llm_provenance;
/// **Sprint 15 — N23 + G13** : `GET /v1/nano-catalogue` handler.
pub mod nano_catalogue;
pub mod openapi;
pub mod publishers;
pub mod swarms;

pub use admin::{
    circuit_breakers_handler, compliance_status_handler, drift_sweep_trigger_handler,
};
pub use agentic::agentic_call_handler;
pub use agentic_stream::agentic_call_stream_handler;
pub use gold::{
    approve_gold_set_handler, lookup_gold_run_handler, register_gold_set_handler,
    run_gold_gate_handler,
};
pub use llm_provenance::llm_provenance_handler;
pub use nano_catalogue::nano_catalogue_handler;
pub use openapi::{openapi_json_handler, openapi_spec};
pub use publishers::{
    lookup_publisher_handler, promote_publisher_handler, register_publisher_handler,
    revoke_publisher_handler, rotate_key_handler,
};
pub use swarms::swarms_run_handler;
