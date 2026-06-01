//! CUDA stream pool.
//!
//! A [`StreamPool`] hands out [`StreamHandle`]s that represent a
//! non-blocking CUDA stream (simulated in MVP by an integer id).
//! Handles are RAII: the underlying stream is returned to the pool
//! on drop.
//!
//! Real CUDA streams arrive in Sprint 2 behind the same public API;
//! this module's MVP implementation uses integer "pseudo-streams" so
//! the pool logic and concurrency contract can be exercised without a
//! GPU.

pub mod config;
pub mod handle;
pub mod metrics;
pub mod pool;

pub use config::{StreamFlags, StreamPoolConfig};
pub use handle::StreamHandle;
pub use metrics::{StreamMetrics, StreamMetricsSnapshot};
pub use pool::StreamPool;
