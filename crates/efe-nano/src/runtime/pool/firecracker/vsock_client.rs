//! **Sprint 8 — N11 days 12-13** : vsock RPC client.
//!
//! Two backends behind one trait :
//! - [`UnixSocketBackend`] : connects to the Firecracker-managed Unix
//!   domain socket that the guest's vsock CID is mapped onto.  This is
//!   the **production path** for Sprint 8 N11 : Firecracker translates
//!   `AF_VSOCK` ↔ `AF_UNIX` for us, so the host code uses plain Unix
//!   sockets and the guest still sees a vsock.
//! - [`MockVsockBackend`] : in-process channel for tests.  Used by
//!   `MockLifecycle::rpc` so tests don't need any socket I/O.
//!
//! # Why Firecracker's vsock-over-Unix indirection ?
//!
//! Firecracker exposes vsock by binding a host-side **Unix domain
//! socket** that proxies to the guest's vsock CID.  This means the
//! host doesn't need the `AF_VSOCK` Linux address family directly —
//! we can talk to the guest via standard `UnixStream`.  See
//! [Firecracker docs](https://github.com/firecracker-microvm/firecracker/blob/main/docs/vsock.md).
//!
//! Protocol-on-the-wire is the standard
//! [`super::rpc::encode_frame`] / [`super::rpc::decode_frame`] format :
//! magic byte `0xEF` + length-prefix + JSON body.
//!
//! # Connection model
//!
//! Sprint 8 ships **one connection per RPC** : connect, send, receive,
//! close.  Phase 4 may switch to a keep-alive pool if vsock setup
//! cost becomes measurable (today ~ 200 µs).

#![cfg(target_os = "linux")]

use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::time::Duration;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use super::rpc::{
    decode_frame, encode_frame, parse_frame_header, FRAME_HEADER_LEN, VsockRpcRequest,
    VsockRpcResponse,
};

#[derive(Debug, Error)]
pub enum VsockError {
    #[error("connect to {path} failed : {reason}")]
    Connect { path: PathBuf, reason: String },

    #[error("I/O error : {0}")]
    Io(String),

    #[error("frame error : {0}")]
    Frame(#[from] super::rpc::FrameError),

    #[error("timeout after {0:?}")]
    Timeout(Duration),
}

// ─────────────────────────────────────────────────────────────────────
//  VsockBackend trait
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N11** : transport-agnostic vsock client.
///
/// One impl per environment ; the production path is
/// [`UnixSocketBackend`], the test path is `MockVsockBackend` (in
/// `lifecycle.rs`).
#[async_trait]
pub trait VsockBackend: Send + Sync {
    /// Send one request, receive one response, close.
    async fn call(
        &self,
        request: VsockRpcRequest,
        timeout: Duration,
    ) -> Result<VsockRpcResponse, VsockError>;
}

// ─────────────────────────────────────────────────────────────────────
//  UnixSocketBackend
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N11** : connect to the Firecracker-managed UDS that
/// proxies into the guest's vsock CID.
#[derive(Debug, Clone)]
pub struct UnixSocketBackend {
    socket_path: PathBuf,
    /// Firecracker vsock-over-UDS protocol requires the **first line**
    /// of the connection to be `CONNECT <port>\n` followed by the
    /// guest's expected response `OK <port>\n`.  This handshake routes
    /// the byte stream to the right guest vsock port.
    guest_port: u32,
}

impl UnixSocketBackend {
    pub fn new(socket_path: impl Into<PathBuf>, guest_port: u32) -> Self {
        Self {
            socket_path: socket_path.into(),
            guest_port,
        }
    }
}

#[async_trait]
impl VsockBackend for UnixSocketBackend {
    async fn call(
        &self,
        request: VsockRpcRequest,
        timeout: Duration,
    ) -> Result<VsockRpcResponse, VsockError> {
        tokio::time::timeout(timeout, self.call_inner(request))
            .await
            .map_err(|_| VsockError::Timeout(timeout))?
    }
}

impl UnixSocketBackend {
    async fn call_inner(
        &self,
        request: VsockRpcRequest,
    ) -> Result<VsockRpcResponse, VsockError> {
        let mut stream = UnixStream::connect(&self.socket_path).await.map_err(|e| {
            VsockError::Connect {
                path: self.socket_path.clone(),
                reason: e.to_string(),
            }
        })?;

        // 1. Firecracker vsock-UDS handshake : "CONNECT <port>\n"
        let handshake = format!("CONNECT {}\n", self.guest_port);
        stream
            .write_all(handshake.as_bytes())
            .await
            .map_err(|e| VsockError::Io(format!("write handshake : {e}")))?;

        // Read the "OK <port>\n" response.  Firecracker sends back the
        // port number it bound on the host side.
        let mut handshake_resp = Vec::with_capacity(32);
        let mut byte = [0u8; 1];
        loop {
            stream
                .read_exact(&mut byte)
                .await
                .map_err(|e| VsockError::Io(format!("read handshake : {e}")))?;
            handshake_resp.push(byte[0]);
            if byte[0] == b'\n' || handshake_resp.len() > 32 {
                break;
            }
        }
        let resp_line = std::str::from_utf8(&handshake_resp)
            .map_err(|_| VsockError::Io("non-utf8 handshake reply".into()))?;
        if !resp_line.starts_with("OK ") {
            return Err(VsockError::Io(format!(
                "vsock handshake failed : got '{resp_line}', expected 'OK <port>'"
            )));
        }

        // 2. Frame + send request.
        let frame = encode_frame(&request)?;
        stream
            .write_all(&frame)
            .await
            .map_err(|e| VsockError::Io(format!("write frame : {e}")))?;
        stream
            .flush()
            .await
            .map_err(|e| VsockError::Io(format!("flush : {e}")))?;

        // 3. Read response header.
        let mut header = [0u8; FRAME_HEADER_LEN];
        stream
            .read_exact(&mut header)
            .await
            .map_err(|e| VsockError::Io(format!("read header : {e}")))?;
        let body_len = parse_frame_header(&header)?;

        // 4. Read response body.
        let mut body = vec![0u8; FRAME_HEADER_LEN + body_len];
        body[..FRAME_HEADER_LEN].copy_from_slice(&header);
        stream
            .read_exact(&mut body[FRAME_HEADER_LEN..])
            .await
            .map_err(|e| VsockError::Io(format!("read body : {e}")))?;

        // 5. Decode.
        let (resp, _consumed): (VsockRpcResponse, usize) = decode_frame(&body)?;
        Ok(resp)
    }
}

/// Convenience : `tokio::net::UnixStream::connect` with a Sprint-8
/// retry loop : Firecracker may not have bound the UDS yet immediately
/// after `InstanceStart` (kernel boot + init scripts can take 300+ ms).
///
/// Polls every 20 ms up to `total_timeout`.
pub async fn wait_for_uds(
    path: &Path,
    total_timeout: Duration,
) -> Result<(), VsockError> {
    let start = std::time::Instant::now();
    while start.elapsed() < total_timeout {
        match UnixStream::connect(path).await {
            Ok(_) => return Ok(()),
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    }
    Err(VsockError::Timeout(total_timeout))
}

// ─────────────────────────────────────────────────────────────────────
//  Tests — exercised against tokio::net loopback UnixListener
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::rpc::{HealthResult, RpcPayload, RpcResult, SCHEMA_VERSION};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixListener;
    use uuid::Uuid;

    /// Run a fake guest server : accept one connection, do handshake,
    /// echo back a Health response.  Returns when the connection
    /// closes.
    async fn fake_guest_server(listener: UnixListener, port: u32) {
        let (mut sock, _) = listener.accept().await.unwrap();

        // 1. Handshake : expect "CONNECT <port>\n".
        let mut buf = Vec::with_capacity(32);
        let mut byte = [0u8; 1];
        loop {
            sock.read_exact(&mut byte).await.unwrap();
            buf.push(byte[0]);
            if byte[0] == b'\n' {
                break;
            }
        }
        let line = std::str::from_utf8(&buf).unwrap();
        assert!(line.starts_with(&format!("CONNECT {port}")));
        sock.write_all(format!("OK {port}\n").as_bytes()).await.unwrap();

        // 2. Read frame.
        let mut header = [0u8; FRAME_HEADER_LEN];
        sock.read_exact(&mut header).await.unwrap();
        let body_len = parse_frame_header(&header).unwrap();
        let mut body = vec![0u8; body_len];
        sock.read_exact(&mut body).await.unwrap();
        let mut full = header.to_vec();
        full.extend_from_slice(&body);
        let (req, _): (VsockRpcRequest, usize) = decode_frame(&full).unwrap();

        // 3. Write fake response.
        let resp = VsockRpcResponse {
            schema_version: SCHEMA_VERSION,
            request_id: req.request_id,
            payload: RpcPayload::Ok {
                result: RpcResult::Health(HealthResult {
                    status: "ready".into(),
                    uptime_secs: 42,
                    model_id: "fake".into(),
                    model_version: "0.0.0".into(),
                }),
            },
        };
        let frame = encode_frame(&resp).unwrap();
        sock.write_all(&frame).await.unwrap();
        sock.flush().await.unwrap();
        drop(sock);
    }

    fn tmpsock(name: &str) -> PathBuf {
        let pid = std::process::id();
        std::env::temp_dir().join(format!("efe-nano-test-{pid}-{name}.sock"))
    }

    #[tokio::test]
    async fn round_trip_handshake_and_health_rpc() {
        let path = tmpsock("vsock_rt");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        let server = tokio::spawn(fake_guest_server(listener, 5000));

        let backend = UnixSocketBackend::new(&path, 5000);
        let req = VsockRpcRequest::health(Uuid::from_u128(0xabcd));
        let resp = backend.call(req, Duration::from_secs(2)).await.unwrap();
        match resp.payload {
            RpcPayload::Ok {
                result: RpcResult::Health(h),
            } => {
                assert_eq!(h.status, "ready");
                assert_eq!(h.uptime_secs, 42);
            }
            other => panic!("expected Health Ok, got {other:?}"),
        }
        let _ = std::fs::remove_file(&path);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn call_to_nonexistent_socket_errors_connect() {
        let path = tmpsock("nonexistent");
        let _ = std::fs::remove_file(&path);
        let backend = UnixSocketBackend::new(&path, 5000);
        let req = VsockRpcRequest::health(Uuid::new_v4());
        let err = backend
            .call(req, Duration::from_millis(100))
            .await
            .unwrap_err();
        assert!(matches!(err, VsockError::Connect { .. }));
    }

    #[tokio::test]
    async fn call_times_out_when_server_doesnt_respond() {
        let path = tmpsock("hang");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).unwrap();
        // Spawn a server that accepts but never speaks.
        let _server = tokio::spawn(async move {
            let (_sock, _) = listener.accept().await.unwrap();
            tokio::time::sleep(Duration::from_secs(5)).await;
        });

        let backend = UnixSocketBackend::new(&path, 5000);
        let req = VsockRpcRequest::health(Uuid::new_v4());
        let err = backend
            .call(req, Duration::from_millis(100))
            .await
            .unwrap_err();
        assert!(matches!(err, VsockError::Timeout(_)));
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn wait_for_uds_returns_ok_when_socket_appears() {
        let path = tmpsock("wait_appear");
        let _ = std::fs::remove_file(&path);
        // Create the socket after a delay.
        let path_clone = path.clone();
        let h = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            let _listener = UnixListener::bind(&path_clone).unwrap();
            tokio::time::sleep(Duration::from_millis(200)).await;
        });
        wait_for_uds(&path, Duration::from_millis(500))
            .await
            .unwrap();
        h.await.unwrap();
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn wait_for_uds_times_out_when_socket_never_appears() {
        let path = tmpsock("never");
        let _ = std::fs::remove_file(&path);
        let err = wait_for_uds(&path, Duration::from_millis(60))
            .await
            .unwrap_err();
        assert!(matches!(err, VsockError::Timeout(_)));
    }
}
