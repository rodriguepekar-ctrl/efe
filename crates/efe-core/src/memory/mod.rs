//! HBM arena + memory pool (M2).
//!
//! This module provides the low-level allocation layer of EFE:
//!
//! - [`Arena`]: bump allocator over a single device-memory region.
//! - [`MemoryPool`]: thread-safe pool of pre-allocated arenas.
//! - [`ArenaHandle`]: RAII wrapper that returns an arena to the pool
//!   on drop.
//! - [`DeviceSlice`]: typed borrowed view with lifetime + generation
//!   counter safety.
//! - [`DeviceBackend`]: abstracts host / CUDA memory. MVP ships with
//!   [`HostBackend`]; the CUDA backend arrives in Sprint 2.
//! - [`NanoAgentBatch`]: SoA container tying a swarm to its arena.
//!
//! # Typical usage
//!
//! ```no_run
//! # use std::sync::Arc;
//! # use efe_core::memory::{DeviceBackend, HostBackend, MemoryPool, PoolConfig};
//! # async fn example() -> efe_core::Result<()> {
//! let backend: Arc<dyn DeviceBackend> = Arc::new(HostBackend::new());
//! let pool = MemoryPool::new(PoolConfig::default(), backend)?;
//! let handle = pool.acquire(1 << 20, None).await?;
//! let tensor = handle.arena().alloc_zeroed::<f32>(1024)?;
//! assert_eq!(tensor.len(), 1024);
//! // on drop, handle returns the arena to the pool
//! # Ok(())
//! # }
//! ```

pub mod arena;
pub mod backend;
pub mod batch;
pub mod config;
pub mod device_slice;
pub mod handle;
pub mod metrics;
pub mod pool;

pub use arena::{copy_slice_to_host, Arena, RawAllocation};
pub use backend::{DeviceBackend, HostBackend};
pub use batch::NanoAgentBatch;
pub use config::{PoolConfig, MIN_ARENA_BYTES, TENSOR_ALIGNMENT};
pub use device_slice::{DeviceCopy, DevicePtr, DeviceSlice};
pub use handle::ArenaHandle;
pub use metrics::{PoolMetrics, PoolMetricsSnapshot};
pub use pool::MemoryPool;
