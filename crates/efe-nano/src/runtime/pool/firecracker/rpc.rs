//! **Sprint 8 — N11** : host ↔ guest RPC framing + protocol.
//!
//! See `EFE_M50_N11_FIRECRACKER_POOL_ARCHITECTURE.md` § 6.
//!
//! # Frame format
//!
//! ```text
//! ┌────────┬──────────────────────┬──────────────────────────────────────┐
//! │ 0xEF   │   length : u32 BE    │   JSON body : `length` bytes         │
//! └────────┴──────────────────────┴──────────────────────────────────────┘
//!   magic           4 bytes                 variable
//! ```
//!
//! The magic byte `0xEF` defends against protocol confusion (e.g. an
//! attacker who tries to dial the vsock port with a different wire
//! protocol like HTTP gets rejected on byte 1).
//!
//! # Frame parsing : pure functions, no I/O
//!
//! This module ships **only the frame codec** + protocol message types.
//! Actual vsock I/O lives in `lifecycle.rs::FirecrackerLifecycle` (Linux)
//! / `lifecycle.rs::MockLifecycle` (cross-platform).  Tests below cover
//! every framing edge case without touching the network.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────
//  Framing constants
// ─────────────────────────────────────────────────────────────────────

/// Magic byte for every Sprint-8 vsock frame.  Mnemonic : EFE.
pub const FRAME_MAGIC: u8 = 0xEF;

/// Header size in bytes : magic (1) + length (4 BE).
pub const FRAME_HEADER_LEN: usize = 5;

/// Maximum allowed body size, hard-capped to defeat OOM-by-RPC attacks.
/// 16 MiB is plenty for any realistic JSON payload.
pub const MAX_RPC_BYTES: usize = 16 * 1024 * 1024;

// ─────────────────────────────────────────────────────────────────────
//  Errors
// ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FrameError {
    #[error("invalid magic byte : got 0x{got:02x}, expected 0x{expected:02x}")]
    BadMagic { got: u8, expected: u8 },

    #[error("body too large : {actual} > {max}")]
    TooLarge { actual: usize, max: usize },

    #[error("body too short : got {got}, header claimed {claimed}")]
    Truncated { got: usize, claimed: usize },

    #[error("JSON decode : {0}")]
    Json(String),

    #[error("buffer too small for header (need {FRAME_HEADER_LEN}, got {got})")]
    HeaderUnderflow { got: usize },
}

// ─────────────────────────────────────────────────────────────────────
//  Frame codec (pure functions, no I/O)
// ─────────────────────────────────────────────────────────────────────

/// Encode a value as a complete vsock frame : header + JSON body.
///
/// Returns `FrameError::TooLarge` if the JSON encoding exceeds
/// `MAX_RPC_BYTES`.
pub fn encode_frame<T: Serialize>(value: &T) -> Result<Vec<u8>, FrameError> {
    let body = serde_json::to_vec(value).map_err(|e| FrameError::Json(e.to_string()))?;
    encode_frame_bytes(&body)
}

/// Encode an already-serialized body as a vsock frame.
pub fn encode_frame_bytes(body: &[u8]) -> Result<Vec<u8>, FrameError> {
    if body.len() > MAX_RPC_BYTES {
        return Err(FrameError::TooLarge {
            actual: body.len(),
            max: MAX_RPC_BYTES,
        });
    }
    let mut frame = Vec::with_capacity(FRAME_HEADER_LEN + body.len());
    frame.push(FRAME_MAGIC);
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(body);
    Ok(frame)
}

/// Parse a frame header.  Returns the body length on success.
pub fn parse_frame_header(buf: &[u8]) -> Result<usize, FrameError> {
    if buf.len() < FRAME_HEADER_LEN {
        return Err(FrameError::HeaderUnderflow { got: buf.len() });
    }
    if buf[0] != FRAME_MAGIC {
        return Err(FrameError::BadMagic {
            got: buf[0],
            expected: FRAME_MAGIC,
        });
    }
    let len = u32::from_be_bytes([buf[1], buf[2], buf[3], buf[4]]) as usize;
    if len > MAX_RPC_BYTES {
        return Err(FrameError::TooLarge {
            actual: len,
            max: MAX_RPC_BYTES,
        });
    }
    Ok(len)
}

/// Decode a complete frame (header + body) into a typed value.
/// Returns the deserialized value AND the number of bytes consumed.
pub fn decode_frame<'de, T: Deserialize<'de>>(buf: &'de [u8]) -> Result<(T, usize), FrameError> {
    let body_len = parse_frame_header(buf)?;
    let total = FRAME_HEADER_LEN + body_len;
    if buf.len() < total {
        return Err(FrameError::Truncated {
            got: buf.len(),
            claimed: total,
        });
    }
    let body = &buf[FRAME_HEADER_LEN..total];
    let value = serde_json::from_slice(body).map_err(|e| FrameError::Json(e.to_string()))?;
    Ok((value, total))
}

// ─────────────────────────────────────────────────────────────────────
//  Protocol message types
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N11** : request envelope.  Method-tagged via
/// `#[serde(flatten)]` so the wire form is :
/// `{"schema_version":1,"request_id":"...","method":"infer","params":{...}}`
/// (NOT a nested `method` object).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VsockRpcRequest {
    pub schema_version: u32,
    pub request_id: Uuid,
    #[serde(flatten)]
    pub method: RpcMethod,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "method", rename_all = "snake_case")]
pub enum RpcMethod {
    Infer { params: InferParams },
    Health,
    Shutdown { grace_secs: u32 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InferParams {
    pub input: serde_json::Value,
    pub agent_id: String,
    pub agent_version: String,
    pub trace_id: Uuid,
    /// Absolute deadline in Unix microseconds.  Guest checks `now_us > deadline_micros`
    /// before starting inference and surfaces `deadline_exceeded` if so.
    pub deadline_micros: i64,
}

/// **Sprint 8 — N11** : response envelope.  Either `result` or `error`,
/// never both.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VsockRpcResponse {
    pub schema_version: u32,
    pub request_id: Uuid,
    #[serde(flatten)]
    pub payload: RpcPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum RpcPayload {
    Ok { result: RpcResult },
    Err { error: RpcError },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum RpcResult {
    Infer(InferResult),
    Health(HealthResult),
    Shutdown {
        acknowledged: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InferResult {
    pub output: serde_json::Value,
    pub model_invocation_micros: u64,
    pub model_memory_high_water_mb: u32,
    #[serde(default)]
    pub guest_metadata: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HealthResult {
    pub status: String,
    pub uptime_secs: u64,
    pub model_id: String,
    pub model_version: String,
}

/// **Sprint 8 — N11** : closed taxonomy of guest-side errors.  Host
/// uses the `code` to decide whether to reap the VM or keep it warm.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RpcError {
    pub code: RpcErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RpcErrorCode {
    /// Guest rejected the input against its schema.  VM stays warm.
    InputSchemaInvalid,
    /// Inference exceeded the deadline.  VM stays warm (we treat
    /// per-call timeout as a soft error ; the model itself is fine).
    DeadlineExceeded,
    /// Model OOM'd.  VM is unrecoverable ; reap.
    ModelOom,
    /// Internal daemon bug.  Reap, alarm.
    Internal,
    /// Model not loaded — should never happen post-boot.  Reap, alarm.
    NotLoaded,
}

impl RpcErrorCode {
    /// True iff this error means the VM is unrecoverable and must be reaped.
    pub fn is_fatal_to_vm(self) -> bool {
        matches!(self, Self::ModelOom | Self::Internal | Self::NotLoaded)
    }
}

/// **Sprint 8 — N11** : protocol schema version.  Bump on breaking
/// changes ; host + guest negotiate at boot via the first `Health` RPC.
pub const SCHEMA_VERSION: u32 = 1;

// ─────────────────────────────────────────────────────────────────────
//  Convenience constructors
// ─────────────────────────────────────────────────────────────────────

impl VsockRpcRequest {
    pub fn infer(
        request_id: Uuid,
        input: serde_json::Value,
        agent_id: impl Into<String>,
        agent_version: impl Into<String>,
        trace_id: Uuid,
        deadline_micros: i64,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            request_id,
            method: RpcMethod::Infer {
                params: InferParams {
                    input,
                    agent_id: agent_id.into(),
                    agent_version: agent_version.into(),
                    trace_id,
                    deadline_micros,
                },
            },
        }
    }

    pub fn health(request_id: Uuid) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            request_id,
            method: RpcMethod::Health,
        }
    }

    pub fn shutdown(request_id: Uuid, grace_secs: u32) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            request_id,
            method: RpcMethod::Shutdown { grace_secs },
        }
    }
}

impl VsockRpcResponse {
    pub fn ok(request_id: Uuid, result: RpcResult) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            request_id,
            payload: RpcPayload::Ok { result },
        }
    }

    pub fn err(request_id: Uuid, error: RpcError) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            request_id,
            payload: RpcPayload::Err { error },
        }
    }

    pub fn as_result(&self) -> Result<&RpcResult, &RpcError> {
        match &self.payload {
            RpcPayload::Ok { result } => Ok(result),
            RpcPayload::Err { error } => Err(error),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_infer_request() -> VsockRpcRequest {
        VsockRpcRequest::infer(
            Uuid::from_u128(0x1234),
            serde_json::json!({"text": "hello"}),
            "ner_clinical",
            "1.0.0",
            Uuid::from_u128(0x5678),
            1_700_000_000_000_000,
        )
    }

    fn sample_health_request() -> VsockRpcRequest {
        VsockRpcRequest::health(Uuid::from_u128(0xaaaa))
    }

    fn sample_infer_response() -> VsockRpcResponse {
        VsockRpcResponse::ok(
            Uuid::from_u128(0x1234),
            RpcResult::Infer(InferResult {
                output: serde_json::json!({"entities": []}),
                model_invocation_micros: 18432,
                model_memory_high_water_mb: 187,
                guest_metadata: HashMap::new(),
            }),
        )
    }

    // ─────────────────────────────────────────────────────────────────
    //  Framing — encode + decode round-trip
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn encode_then_decode_round_trip_request() {
        let req = sample_infer_request();
        let frame = encode_frame(&req).unwrap();
        let (got, consumed): (VsockRpcRequest, usize) = decode_frame(&frame).unwrap();
        assert_eq!(got, req);
        assert_eq!(consumed, frame.len());
    }

    #[test]
    fn encode_then_decode_round_trip_response() {
        let resp = sample_infer_response();
        let frame = encode_frame(&resp).unwrap();
        let (got, _): (VsockRpcResponse, usize) = decode_frame(&frame).unwrap();
        match &got.payload {
            RpcPayload::Ok {
                result: RpcResult::Infer(r),
            } => {
                assert_eq!(r.model_invocation_micros, 18432);
            }
            _ => panic!("expected Infer Ok"),
        }
    }

    #[test]
    fn frame_starts_with_magic() {
        let req = sample_health_request();
        let frame = encode_frame(&req).unwrap();
        assert_eq!(frame[0], FRAME_MAGIC);
    }

    #[test]
    fn frame_length_is_big_endian_u32() {
        let req = sample_health_request();
        let frame = encode_frame(&req).unwrap();
        let body_len = frame.len() - FRAME_HEADER_LEN;
        let claimed = u32::from_be_bytes([frame[1], frame[2], frame[3], frame[4]]);
        assert_eq!(claimed as usize, body_len);
    }

    // ─────────────────────────────────────────────────────────────────
    //  Framing — edge cases & error paths
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn parse_header_rejects_bad_magic() {
        let mut frame = vec![0u8; FRAME_HEADER_LEN];
        frame[0] = 0xDE;
        let err = parse_frame_header(&frame).unwrap_err();
        assert_eq!(
            err,
            FrameError::BadMagic {
                got: 0xDE,
                expected: FRAME_MAGIC
            }
        );
    }

    #[test]
    fn parse_header_rejects_short_buffer() {
        let frame = vec![FRAME_MAGIC, 0, 0];
        let err = parse_frame_header(&frame).unwrap_err();
        assert!(matches!(err, FrameError::HeaderUnderflow { got: 3 }));
    }

    #[test]
    fn parse_header_rejects_oversized_body() {
        let mut frame = vec![FRAME_MAGIC];
        frame.extend_from_slice(&u32::MAX.to_be_bytes());
        let err = parse_frame_header(&frame).unwrap_err();
        match err {
            FrameError::TooLarge { actual, max } => {
                assert_eq!(actual, u32::MAX as usize);
                assert_eq!(max, MAX_RPC_BYTES);
            }
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }

    #[test]
    fn decode_rejects_truncated_body() {
        let req = sample_health_request();
        let mut frame = encode_frame(&req).unwrap();
        frame.pop();
        let err: FrameError =
            decode_frame::<VsockRpcRequest>(&frame).unwrap_err();
        match err {
            FrameError::Truncated { got, claimed } => {
                assert!(got < claimed);
            }
            other => panic!("expected Truncated, got {other:?}"),
        }
    }

    #[test]
    fn encode_rejects_oversized_body() {
        let huge = vec![0u8; MAX_RPC_BYTES + 1];
        let err = encode_frame_bytes(&huge).unwrap_err();
        match err {
            FrameError::TooLarge { actual, max } => {
                assert_eq!(actual, MAX_RPC_BYTES + 1);
                assert_eq!(max, MAX_RPC_BYTES);
            }
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }

    #[test]
    fn decode_rejects_invalid_json() {
        let mut frame = Vec::new();
        let body = b"{ this is not json }";
        frame.push(FRAME_MAGIC);
        frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
        frame.extend_from_slice(body);
        let err = decode_frame::<VsockRpcRequest>(&frame).unwrap_err();
        match err {
            FrameError::Json(_) => {}
            other => panic!("expected Json, got {other:?}"),
        }
    }

    #[test]
    fn empty_body_decodes_for_empty_struct() {
        // serde_json::from_slice(b"") fails; we test that this path
        // surfaces as Json error (NOT a magic / size error).
        let mut frame = Vec::new();
        frame.push(FRAME_MAGIC);
        frame.extend_from_slice(&0u32.to_be_bytes());
        let err = decode_frame::<VsockRpcRequest>(&frame).unwrap_err();
        assert!(matches!(err, FrameError::Json(_)));
    }

    #[test]
    fn parse_header_accepts_exact_max_size() {
        let mut frame = vec![FRAME_MAGIC];
        frame.extend_from_slice(&(MAX_RPC_BYTES as u32).to_be_bytes());
        let len = parse_frame_header(&frame).unwrap();
        assert_eq!(len, MAX_RPC_BYTES);
    }

    #[test]
    fn parse_header_rejects_max_size_plus_one() {
        let mut frame = vec![FRAME_MAGIC];
        frame.extend_from_slice(&((MAX_RPC_BYTES + 1) as u32).to_be_bytes());
        let err = parse_frame_header(&frame).unwrap_err();
        match err {
            FrameError::TooLarge { actual, max } => {
                assert_eq!(actual, MAX_RPC_BYTES + 1);
                assert_eq!(max, MAX_RPC_BYTES);
            }
            other => panic!("expected TooLarge, got {other:?}"),
        }
    }

    // ─────────────────────────────────────────────────────────────────
    //  Protocol message serialization
    // ─────────────────────────────────────────────────────────────────

    #[test]
    fn infer_request_serializes_with_method_tag() {
        let req = sample_infer_request();
        let s = serde_json::to_value(&req).unwrap();
        assert_eq!(s["method"], "infer");
        assert_eq!(s["params"]["agent_id"], "ner_clinical");
        assert_eq!(s["schema_version"], 1);
    }

    #[test]
    fn health_request_has_no_params() {
        let req = sample_health_request();
        let s = serde_json::to_value(&req).unwrap();
        assert_eq!(s["method"], "health");
        // method-tagged serde with no fields should not have "params"
        assert!(s.get("params").is_none() || s["params"].is_null() || s["params"] == serde_json::json!({}));
    }

    #[test]
    fn shutdown_request_carries_grace_secs() {
        let req = VsockRpcRequest::shutdown(Uuid::new_v4(), 5);
        let s = serde_json::to_value(&req).unwrap();
        assert_eq!(s["method"], "shutdown");
        assert_eq!(s["grace_secs"], 5);
    }

    #[test]
    fn ok_response_serializes_with_result_field() {
        let resp = sample_infer_response();
        let s = serde_json::to_value(&resp).unwrap();
        assert!(s.get("result").is_some());
        assert!(s.get("error").is_none());
    }

    #[test]
    fn err_response_serializes_with_error_field() {
        let resp = VsockRpcResponse::err(
            Uuid::from_u128(0x1234),
            RpcError {
                code: RpcErrorCode::ModelOom,
                message: "model exhausted memory at 248 MB".into(),
            },
        );
        let s = serde_json::to_value(&resp).unwrap();
        assert_eq!(s["error"]["code"], "model_oom");
        assert!(s.get("result").is_none());
    }

    #[test]
    fn rpc_error_code_is_fatal_to_vm_classification() {
        assert!(!RpcErrorCode::InputSchemaInvalid.is_fatal_to_vm());
        assert!(!RpcErrorCode::DeadlineExceeded.is_fatal_to_vm());
        assert!(RpcErrorCode::ModelOom.is_fatal_to_vm());
        assert!(RpcErrorCode::Internal.is_fatal_to_vm());
        assert!(RpcErrorCode::NotLoaded.is_fatal_to_vm());
    }

    #[test]
    fn response_as_result_extracts_correctly() {
        let ok_resp = sample_infer_response();
        assert!(ok_resp.as_result().is_ok());

        let err_resp = VsockRpcResponse::err(
            Uuid::from_u128(0x1234),
            RpcError {
                code: RpcErrorCode::Internal,
                message: "boom".into(),
            },
        );
        match err_resp.as_result() {
            Err(e) => assert_eq!(e.code, RpcErrorCode::Internal),
            Ok(_) => panic!("expected Err"),
        }
    }

    #[test]
    fn round_trip_with_real_protocol_message() {
        // End-to-end : request → frame → wire → frame → request
        let req = sample_infer_request();
        let frame = encode_frame(&req).unwrap();

        // Simulate a sloppy reader that gets the frame in two chunks.
        let split = FRAME_HEADER_LEN + 5;
        let (first, second) = frame.split_at(split);
        let mut reassembled = Vec::with_capacity(frame.len());
        reassembled.extend_from_slice(first);
        reassembled.extend_from_slice(second);
        let (got, _): (VsockRpcRequest, usize) = decode_frame(&reassembled).unwrap();
        assert_eq!(got, req);
    }

    #[test]
    fn schema_version_constant() {
        assert_eq!(SCHEMA_VERSION, 1);
        let req = sample_health_request();
        assert_eq!(req.schema_version, 1);
    }
}
