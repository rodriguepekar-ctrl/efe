//! Memory pool configuration.
//!
//! All sizes are in **bytes** unless the field name carries an explicit
//! unit (e.g. `_gb`, `_mb`). The defaults target an H100 80 GiB SKU:
//! 8 arenas × 1 GiB = 8 GiB reserved, 4 streams, device 0.

use serde::{Deserialize, Serialize};

use crate::errors::{EfeError, Result};

/// Minimum arena size accepted by the pool. Anything smaller makes no
/// sense because the alignment overhead dominates.
pub const MIN_ARENA_BYTES: u64 = 4 * 1024;

/// Alignment enforced on every allocation inside an arena. Matches the
/// coalesced-load requirement on NVIDIA H100+.
pub const TENSOR_ALIGNMENT: usize = 256;

/// Configuration of the [`crate::memory::MemoryPool`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct PoolConfig {
    /// Default arena size when allocating fresh (bytes).
    pub default_arena_size: u64,
    /// Hard cap on simultaneously-allocated arenas.
    pub max_arenas: usize,
    /// How many arenas to create eagerly at `new()`.
    pub prealloc_arenas: usize,
    /// Target device ordinal (ignored by the host backend).
    pub device_id: u32,
    /// If true, panic in debug builds on arena leak at pool shutdown.
    pub debug_leak_check: bool,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            default_arena_size: 1 << 30, // 1 GiB
            max_arenas: 8,
            prealloc_arenas: 4,
            device_id: 0,
            debug_leak_check: cfg!(debug_assertions),
        }
    }
}

impl PoolConfig {
    /// Validate the configuration.
    ///
    /// # Errors
    ///
    /// Returns [`EfeError::InvalidConfig`] on:
    /// - `max_arenas < 1`
    /// - `prealloc_arenas > max_arenas`
    /// - `default_arena_size < MIN_ARENA_BYTES`
    /// - `default_arena_size` not aligned to [`TENSOR_ALIGNMENT`]
    pub fn validate(&self) -> Result<()> {
        if self.max_arenas == 0 {
            return Err(EfeError::InvalidConfig("max_arenas must be >= 1".into()));
        }
        if self.prealloc_arenas > self.max_arenas {
            return Err(EfeError::InvalidConfig(format!(
                "prealloc_arenas ({}) must not exceed max_arenas ({})",
                self.prealloc_arenas, self.max_arenas
            )));
        }
        if self.default_arena_size < MIN_ARENA_BYTES {
            return Err(EfeError::InvalidConfig(format!(
                "default_arena_size {} < MIN_ARENA_BYTES {}",
                self.default_arena_size, MIN_ARENA_BYTES
            )));
        }
        if self.default_arena_size % TENSOR_ALIGNMENT as u64 != 0 {
            return Err(EfeError::InvalidConfig(format!(
                "default_arena_size {} not aligned to {}",
                self.default_arena_size, TENSOR_ALIGNMENT
            )));
        }
        Ok(())
    }

    /// Builder-style setter for `max_arenas`.
    #[must_use]
    pub fn with_max_arenas(mut self, n: usize) -> Self {
        self.max_arenas = n;
        self
    }

    /// Builder-style setter for `default_arena_size`.
    #[must_use]
    pub fn with_arena_size(mut self, bytes: u64) -> Self {
        self.default_arena_size = bytes;
        self
    }

    /// Builder-style setter for `prealloc_arenas`.
    #[must_use]
    pub fn with_prealloc(mut self, n: usize) -> Self {
        self.prealloc_arenas = n;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_validates() {
        PoolConfig::default().validate().expect("defaults OK");
    }

    #[test]
    fn rejects_zero_max_arenas() {
        assert!(PoolConfig::default().with_max_arenas(0).validate().is_err());
    }

    #[test]
    fn rejects_prealloc_above_max() {
        let c = PoolConfig::default().with_max_arenas(2).with_prealloc(4);
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_tiny_arena() {
        let c = PoolConfig::default().with_arena_size(64);
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_unaligned_arena() {
        let c = PoolConfig::default().with_arena_size(8193);
        assert!(c.validate().is_err());
    }

    #[test]
    fn accepts_aligned_arena() {
        let c = PoolConfig::default().with_arena_size(4096);
        assert!(c.validate().is_ok());
    }
}
