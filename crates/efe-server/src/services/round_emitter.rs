//! Sprint **Φ-D-STREAM Completion (D80)** — RoundEmitter trait + concrete impls.
//!
//! Cf. `docs/EFE_STREAMING_RPC_COMPLETION_ARCHITECTURE.md` § 4.
//!
//! ## Mission
//!
//! Provide a callback abstraction that lets a streaming backend (Mode-1
//! `SwarmController`, Mode-2 anchor-routed swarm, Mode-3 sub-swarm) emit
//! per-fusion-round telemetry into a tonic `mpsc::Sender<Result<C, Status>>`.
//!
//! The trait is intentionally **generic over the proto chunk type** `C`
//! so the same emitter implementation serves all 4 streaming RPCs :
//!
//! | RPC                                  | C = chunk type            |
//! |--------------------------------------|---------------------------|
//! | `SwarmService.ExecuteStreaming`      | `StreamChunk`             |
//! | `Mode2Service.ExecuteStreaming`      | `Mode2StreamChunk`        |
//! | `Mode3Service.SubscribeVerdict`      | `Mode3StreamChunk` (NullEmitter for sub-swarms) |
//! | `AdminService.ListAuditEvents`       | n/a (no rounds)           |
//!
//! ## Doctrines implemented
//!
//! * **D-STREAM-3** — `is_cancelled()` probe avant chaque emit ;
//!   `tx.is_closed()` détecté → `RoundEmitError::Cancelled`.
//! * **D-STREAM-5** — caller doit utiliser `mpsc::channel(64)` ; ce trait
//!   ne crée pas le channel.
//! * **D-STREAM-9** — chaque emit incrémente `efe_streaming_chunks_total`
//!   via la callback `chunks_emitted` interne.
//! * **D-STREAM-10** — la durée totale + nombre de chunks émis sont
//!   exposés via `total_emitted()` à la fermeture du stream.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use tokio::sync::mpsc::Sender;
use tonic::Status;

// L2 audit fix Φ-D-STREAM.L2-3 : `efe_core::Result<T>` is a single-arg
// type alias for `std::result::Result<T, EfeError>`. Using `Result<(),
// RoundEmitError>` from that import would fail to compile (= arity mismatch).
// Solution : alias the foreign Result and use std's Result explicitly in
// emitter signatures.
use efe_core::Result as EfeResult;

/// Domain-level RoundUpdate, distinct from any proto type. The bridge
/// layer maps this to the per-RPC proto chunk variant via the `map_fn`
/// supplied to `TonicRoundEmitter::new`.
#[derive(Debug, Clone)]
pub struct RoundUpdate {
    /// 1-based round counter.
    pub round_index: u32,
    /// Number of agents active at the end of this round.
    pub active_agents: u32,
    /// Variance of agent positions (proxy for convergence dispersion).
    pub variance: f32,
    /// Confidence so-far for the swarm (≥ 0, ≤ 1).
    pub confidence_so_far: f32,
    /// Total elapsed milliseconds since the streaming handshake.
    pub elapsed_ms: f32,
}

/// Error returned by a `RoundEmitter::emit` call.
#[derive(Debug, thiserror::Error)]
pub enum RoundEmitError {
    /// The client dropped the receiver (= mpsc channel closed). Caller
    /// SHOULD short-circuit remaining rounds (D-STREAM-3).
    #[error("client cancelled (mpsc channel closed)")]
    Cancelled,
    /// Server is shutting down ; pending streams must drain via the
    /// shared `CancellationToken`.
    #[error("server shutting down")]
    ServerShutdown,
}

/// Round emission callback used by streaming backends.
///
/// Implementations are :
/// * `TonicRoundEmitter<C, M>` — proto-chunk emitter for production.
/// * `NullEmitter` — discards rounds (Mode-3 sub-swarm pattern).
/// * `BufferEmitter` — in-memory capture for unit tests.
#[async_trait]
pub trait RoundEmitter: Send + Sync {
    /// Emit a single round update.
    async fn emit(&self, update: RoundUpdate) -> std::result::Result<(), RoundEmitError>;

    /// Probe whether the client is still listening. Backends should call
    /// this between rounds (D-STREAM-3) to bail out early.
    fn is_cancelled(&self) -> bool;

    /// Number of chunks successfully emitted so far. Used by the
    /// `streaming_total_duration_seconds` histogram at close.
    fn chunks_emitted(&self) -> u64;
}

// ─────────────────────────────────────────────────────────────────────
// TonicRoundEmitter — production impl.
// ─────────────────────────────────────────────────────────────────────

/// Tonic-backed emitter that maps `RoundUpdate` → proto chunk variant.
///
/// Generic over :
/// * `C` — proto chunk type (e.g. `StreamChunk`, `Mode2StreamChunk`).
/// * `M` — `Fn(RoundUpdate) -> C` mapping function. Provided by the
///   service module so that each RPC stays decoupled from the others.
pub struct TonicRoundEmitter<C, M> {
    tx: Sender<Result<C, Status>>,
    map_fn: Arc<M>,
    cancelled: AtomicBool,
    started: Instant,
    chunks_emitted: AtomicU64,
}

impl<C, M> TonicRoundEmitter<C, M>
where
    C: Send + 'static,
    M: Fn(RoundUpdate) -> C + Send + Sync + 'static,
{
    /// Build an emitter from a tonic Sender + a chunk mapping function.
    pub fn new(tx: Sender<Result<C, Status>>, map_fn: M) -> Self {
        Self {
            tx,
            map_fn: Arc::new(map_fn),
            cancelled: AtomicBool::new(false),
            started: Instant::now(),
            chunks_emitted: AtomicU64::new(0),
        }
    }

    /// Time elapsed since the emitter was constructed.
    pub fn started(&self) -> Instant {
        self.started
    }
}

#[async_trait]
impl<C, M> RoundEmitter for TonicRoundEmitter<C, M>
where
    C: Send + 'static,
    M: Fn(RoundUpdate) -> C + Send + Sync + 'static,
{
    async fn emit(&self, mut update: RoundUpdate) -> std::result::Result<(), RoundEmitError> {
        // D-STREAM-3 : probe BEFORE attempting send.
        if self.tx.is_closed() {
            self.cancelled.store(true, Ordering::Relaxed);
            return Err(RoundEmitError::Cancelled);
        }
        // Server-side elapsed_ms is the source of truth (= ignores any
        // jitter the backend might inject).
        update.elapsed_ms = self.started.elapsed().as_secs_f32() * 1000.0;
        let chunk = (self.map_fn)(update);
        match self.tx.send(Ok(chunk)).await {
            Ok(()) => {
                self.chunks_emitted.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(_) => {
                self.cancelled.store(true, Ordering::Relaxed);
                Err(RoundEmitError::Cancelled)
            }
        }
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed) || self.tx.is_closed()
    }

    fn chunks_emitted(&self) -> u64 {
        self.chunks_emitted.load(Ordering::Relaxed)
    }
}

// ─────────────────────────────────────────────────────────────────────
// NullEmitter — Mode-3 sub-swarm fallback.
// ─────────────────────────────────────────────────────────────────────

/// Discards every emitted round.
///
/// Used by `Mode3Service.SubscribeVerdict` for the sub-swarm execution
/// path : the Mode-3 proto's `oneof Mode3StreamChunk` does NOT carry a
/// `RoundUpdate` variant, so wiring a real `TonicRoundEmitter` would
/// have nowhere to forward the chunk. Observability for intra-sub-swarm
/// rounds passes via OTel spans + Prometheus instruments instead (cf.
/// architecture § 7.3).
#[derive(Debug, Default)]
pub struct NullEmitter {
    chunks_emitted: AtomicU64,
}

impl NullEmitter {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl RoundEmitter for NullEmitter {
    async fn emit(&self, _update: RoundUpdate) -> std::result::Result<(), RoundEmitError> {
        self.chunks_emitted.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn is_cancelled(&self) -> bool {
        false
    }

    fn chunks_emitted(&self) -> u64 {
        self.chunks_emitted.load(Ordering::Relaxed)
    }
}

// ─────────────────────────────────────────────────────────────────────
// BufferEmitter — in-memory capture for unit tests.
// ─────────────────────────────────────────────────────────────────────

/// In-memory ring buffer of emitted rounds.
///
/// Useful in unit tests of streaming backends — capture every emission,
/// then assert sequence, monotonicity, content.
///
/// Audit fix Φ-D-STREAM.L1-1 : `std::sync::Mutex` instead of
/// `parking_lot::Mutex` to avoid pulling a new crate dependency into
/// `efe-server`. The lock is short-lived (= single push or clone) so
/// the perf delta vs parking_lot is negligible.
pub struct BufferEmitter {
    buffer: std::sync::Mutex<Vec<RoundUpdate>>,
    chunks_emitted: AtomicU64,
}

impl BufferEmitter {
    pub fn new() -> Self {
        Self {
            buffer: std::sync::Mutex::new(Vec::new()),
            chunks_emitted: AtomicU64::new(0),
        }
    }

    /// Snapshot of all emitted rounds.
    pub fn snapshot(&self) -> Vec<RoundUpdate> {
        self.buffer
            .lock()
            .expect("BufferEmitter mutex poisoned")
            .clone()
    }
}

impl Default for BufferEmitter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl RoundEmitter for BufferEmitter {
    async fn emit(&self, update: RoundUpdate) -> std::result::Result<(), RoundEmitError> {
        self.buffer
            .lock()
            .expect("BufferEmitter mutex poisoned")
            .push(update);
        self.chunks_emitted.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn is_cancelled(&self) -> bool {
        false
    }

    fn chunks_emitted(&self) -> u64 {
        self.chunks_emitted.load(Ordering::Relaxed)
    }
}

// ─────────────────────────────────────────────────────────────────────
// Streaming-aware backend trait (D-STREAM § 4.2).
// ─────────────────────────────────────────────────────────────────────

use crate::services::ExecuteBackend;
use efe_core::{Request, Response};

/// Extension of `ExecuteBackend` that supports per-round emission.
///
/// **Default implementation** : delegates to `execute()` and emits one
/// synthetic round AFTER completion (= D32 behavior). Production
/// backends override with a real per-round emission via
/// `SwarmController::execute_streaming`.
///
/// This default impl preserves wire compat with D32 ; the doctrine
/// **D-STREAM-1** is enforced by the *production* override, not the
/// default.
#[async_trait]
pub trait StreamingExecuteBackend: ExecuteBackend {
    async fn execute_streaming(
        &self,
        req: Request,
        emitter: &dyn RoundEmitter,
    ) -> EfeResult<Response> {
        let resp = self.execute(req).await?;
        // Default emission : one synthetic round = final state.
        let _ = emitter
            .emit(RoundUpdate {
                round_index: resp.rounds_executed,
                active_agents: 0,
                variance: 0.0,
                confidence_so_far: resp.confidence,
                elapsed_ms: 0.0,
            })
            .await;
        Ok(resp)
    }
}

impl<T: ExecuteBackend + ?Sized> StreamingExecuteBackend for T {}

// ─────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[tokio::test]
    async fn buffer_emitter_captures_all_rounds() {
        let e = BufferEmitter::new();
        for i in 1..=5 {
            e.emit(RoundUpdate {
                round_index: i,
                active_agents: i + 10,
                variance: 0.5,
                confidence_so_far: 0.8,
                elapsed_ms: i as f32 * 10.0,
            })
            .await
            .unwrap();
        }
        let snap = e.snapshot();
        assert_eq!(snap.len(), 5);
        for (i, r) in snap.iter().enumerate() {
            assert_eq!(r.round_index, (i + 1) as u32);
        }
        assert_eq!(e.chunks_emitted(), 5);
    }

    #[tokio::test]
    async fn null_emitter_increments_counter_but_discards() {
        let e = NullEmitter::new();
        for i in 1..=3 {
            e.emit(RoundUpdate {
                round_index: i,
                active_agents: 0,
                variance: 0.0,
                confidence_so_far: 0.0,
                elapsed_ms: 0.0,
            })
            .await
            .unwrap();
        }
        assert_eq!(e.chunks_emitted(), 3);
        assert!(!e.is_cancelled());
    }

    #[tokio::test]
    async fn tonic_emitter_emits_then_cancels_on_drop() {
        #[derive(Debug, PartialEq)]
        enum TestChunk {
            Round(u32),
        }
        let (tx, mut rx) = mpsc::channel::<Result<TestChunk, tonic::Status>>(64);
        let emitter = TonicRoundEmitter::new(tx.clone(), |ru: RoundUpdate| {
            TestChunk::Round(ru.round_index)
        });
        // Emit 3 rounds.
        for i in 1..=3 {
            emitter
                .emit(RoundUpdate {
                    round_index: i,
                    active_agents: 0,
                    variance: 0.0,
                    confidence_so_far: 0.0,
                    elapsed_ms: 0.0,
                })
                .await
                .unwrap();
        }
        // Drain receiver.
        let mut chunks = Vec::new();
        for _ in 0..3 {
            chunks.push(rx.recv().await.unwrap().unwrap());
        }
        assert_eq!(
            chunks,
            vec![
                TestChunk::Round(1),
                TestChunk::Round(2),
                TestChunk::Round(3),
            ]
        );
        // Drop receiver → next emit returns Cancelled.
        drop(rx);
        let result = emitter
            .emit(RoundUpdate {
                round_index: 4,
                active_agents: 0,
                variance: 0.0,
                confidence_so_far: 0.0,
                elapsed_ms: 0.0,
            })
            .await;
        assert!(matches!(result, Err(RoundEmitError::Cancelled)));
        assert!(emitter.is_cancelled());
    }
}
