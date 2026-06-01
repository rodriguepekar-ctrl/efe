//! **Sprint 8 — N11 days 14-15** : guest-side vsock listener.
//!
//! Listens on `AF_VSOCK` port 5000 (configurable).  For each accepted
//! connection :
//! 1. Parse the Firecracker "CONNECT <port>" handshake.
//! 2. Reply "OK <port>".
//! 3. Read one length-prefixed frame.
//! 4. Dispatch via [`Handler`].
//! 5. Write the response frame.
//! 6. Close.
//!
//! # Sprint 8 ship vs. real vsock
//!
//! Sprint 8 days 14-15 ship a **Unix-socket fallback** so the daemon
//! can be tested locally (no KVM required).  The transport listens on
//! a Unix-domain socket at `/var/run/efe-inference-daemon.sock` when
//! `EFE_TRANSPORT=unix` is set, and on `AF_VSOCK` otherwise.
//!
//! The actual `AF_VSOCK` listener requires `tokio_vsock` ; Sprint 8
//! day 15 adds it as a Linux-only optional dep.  This file ships the
//! Unix-socket path which is **the path the host integration tests
//! use** because Firecracker's vsock-over-UDS proxy means the host
//! always sees a Unix socket anyway (see `host vsock_client.rs`
//! comment).

use anyhow::{anyhow, Context, Result};
use std::path::PathBuf;
use std::sync::Arc;

use efe_nano::runtime::pool::firecracker::rpc::{
    decode_frame, encode_frame, parse_frame_header, VsockRpcRequest, VsockRpcResponse,
    FRAME_HEADER_LEN,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};

use crate::handler::Handler;

// ─────────────────────────────────────────────────────────────────────
//  Public entry point
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N11** : run the daemon's accept loop.
///
/// The `port` arg is the vsock port number, baked into the handshake
/// (`CONNECT <port>\nOK <port>\n`).  For Unix-socket fallback the port
/// is purely cosmetic — both ends just agree to use the same number.
pub async fn serve(handler: Arc<Handler>, port: u32) -> Result<()> {
    let socket_path = std::env::var("EFE_GUEST_SOCKET")
        .unwrap_or_else(|_| "/var/run/efe-inference-daemon.sock".to_string());
    serve_at(handler, port, PathBuf::from(socket_path)).await
}

/// **Sprint 8 — N11** : test-friendly entry point.  Bypasses
/// environment variables by taking the socket path directly.  Tests
/// call this with unique tempfile paths so they don't race on shared
/// env state.
pub async fn serve_at(
    handler: Arc<Handler>,
    port: u32,
    socket_path: PathBuf,
) -> Result<()> {
    // Best-effort cleanup of stale socket.
    let _ = std::fs::remove_file(&socket_path);
    if let Some(parent) = socket_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("bind unix socket {}", socket_path.display()))?;
    tracing::info!(
        socket = %socket_path.display(),
        port,
        "daemon listening (Unix-socket fallback)"
    );

    loop {
        let (stream, _addr) = match listener.accept().await {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = ?e, "accept failed ; continuing");
                continue;
            }
        };
        let handler = Arc::clone(&handler);
        tokio::spawn(async move {
            if let Err(e) = serve_one(stream, handler, port).await {
                tracing::warn!(error = ?e, "connection failed");
            }
        });
    }
}

/// Serve one connection : handshake + read frame + dispatch + write frame.
async fn serve_one(mut stream: UnixStream, handler: Arc<Handler>, port: u32) -> Result<()> {
    // 1. Read "CONNECT <port>\n".
    let mut handshake = Vec::with_capacity(32);
    let mut byte = [0u8; 1];
    loop {
        stream
            .read_exact(&mut byte)
            .await
            .context("read handshake byte")?;
        handshake.push(byte[0]);
        if byte[0] == b'\n' || handshake.len() > 32 {
            break;
        }
    }
    let line = std::str::from_utf8(&handshake).context("non-utf8 handshake")?;
    let expected_prefix = format!("CONNECT {port}");
    if !line.starts_with(&expected_prefix) {
        return Err(anyhow!(
            "bad handshake : expected '{expected_prefix}', got {line:?}"
        ));
    }

    // 2. Reply "OK <port>\n".
    stream
        .write_all(format!("OK {port}\n").as_bytes())
        .await
        .context("write handshake reply")?;

    // 3. Read one frame.
    let mut header = [0u8; FRAME_HEADER_LEN];
    stream.read_exact(&mut header).await.context("read header")?;
    let body_len = parse_frame_header(&header).context("parse header")?;
    let mut full = Vec::with_capacity(FRAME_HEADER_LEN + body_len);
    full.extend_from_slice(&header);
    full.resize(FRAME_HEADER_LEN + body_len, 0);
    stream
        .read_exact(&mut full[FRAME_HEADER_LEN..])
        .await
        .context("read body")?;
    let (req, _consumed): (VsockRpcRequest, usize) = decode_frame(&full).context("decode frame")?;

    // 4. Dispatch.
    let resp: VsockRpcResponse = handler.dispatch(req).await;

    // 5. Encode + write response.
    let out = encode_frame(&resp).context("encode response")?;
    stream.write_all(&out).await.context("write response")?;
    stream.flush().await.context("flush response")?;

    // 6. Connection is closed implicitly when `stream` drops.
    Ok(())
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ModelLoader;
    use efe_nano::runtime::pool::firecracker::rpc::{
        RpcPayload, RpcResult, SCHEMA_VERSION,
    };
    use std::time::{Duration, Instant};
    use uuid::Uuid;

    fn make_handler() -> Arc<Handler> {
        let bundle = ModelLoader::load("test_model", "1.0.0").unwrap();
        Arc::new(Handler::new(
            "test_model".into(),
            "1.0.0".into(),
            bundle,
            Instant::now(),
        ))
    }

    fn tmpsock(name: &str) -> PathBuf {
        let pid = std::process::id();
        std::env::temp_dir().join(format!("efe-test-daemon-{pid}-{name}.sock"))
    }

    #[tokio::test]
    async fn round_trip_health_call() {
        let socket = tmpsock("rt_health");
        let handler = make_handler();
        // Spawn the server in the background.
        let server = tokio::spawn(serve_at(Arc::clone(&handler), 5000, socket.clone()));

        // Wait for socket to appear.
        for _ in 0..50 {
            if socket.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(socket.exists(), "daemon should have bound the socket");

        // Manually dial as a client.
        let mut stream = UnixStream::connect(&socket).await.unwrap();
        stream
            .write_all(b"CONNECT 5000\n")
            .await
            .unwrap();
        let mut reply = Vec::with_capacity(16);
        let mut byte = [0u8; 1];
        loop {
            stream.read_exact(&mut byte).await.unwrap();
            reply.push(byte[0]);
            if byte[0] == b'\n' {
                break;
            }
        }
        assert!(std::str::from_utf8(&reply).unwrap().starts_with("OK 5000"));

        // Send Health RPC.
        let req = VsockRpcRequest::health(Uuid::from_u128(0xfeed));
        let frame = encode_frame(&req).unwrap();
        stream.write_all(&frame).await.unwrap();
        stream.flush().await.unwrap();

        // Read response.
        let mut header = [0u8; FRAME_HEADER_LEN];
        stream.read_exact(&mut header).await.unwrap();
        let body_len = parse_frame_header(&header).unwrap();
        let mut full = Vec::with_capacity(FRAME_HEADER_LEN + body_len);
        full.extend_from_slice(&header);
        full.resize(FRAME_HEADER_LEN + body_len, 0);
        stream
            .read_exact(&mut full[FRAME_HEADER_LEN..])
            .await
            .unwrap();
        let (resp, _): (VsockRpcResponse, usize) = decode_frame(&full).unwrap();
        assert_eq!(resp.schema_version, SCHEMA_VERSION);
        assert_eq!(resp.request_id, Uuid::from_u128(0xfeed));
        match resp.payload {
            RpcPayload::Ok {
                result: RpcResult::Health(h),
            } => {
                assert_eq!(h.status, "ready");
            }
            other => panic!("expected Health Ok, got {other:?}"),
        }
        drop(stream);
        server.abort();
        let _ = std::fs::remove_file(&socket);
    }

    #[tokio::test]
    async fn handshake_with_wrong_port_rejected() {
        let socket = tmpsock("bad_port");
        let handler = make_handler();
        let server = tokio::spawn(serve_at(Arc::clone(&handler), 5000, socket.clone()));
        for _ in 0..50 {
            if socket.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        // Connect, send WRONG port.
        let mut stream = UnixStream::connect(&socket).await.unwrap();
        stream.write_all(b"CONNECT 9999\n").await.unwrap();
        let mut buf = [0u8; 16];
        let n = stream.read(&mut buf).await.unwrap_or(0);
        assert_eq!(n, 0, "server should close on bad handshake");
        server.abort();
        let _ = std::fs::remove_file(&socket);
    }
}
