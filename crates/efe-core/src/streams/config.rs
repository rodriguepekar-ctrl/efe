//! Stream pool configuration.

use crate::errors::{EfeError, Result};

/// Flags controlling the CUDA stream creation mode.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum StreamFlags {
    /// `cudaStreamDefault` — implicit synchronization with the null stream.
    Default,
    /// `cudaStreamNonBlocking` — independent of the null stream. Preferred.
    NonBlocking,
}

impl Default for StreamFlags {
    fn default() -> Self {
        Self::NonBlocking
    }
}

/// Configuration of the [`crate::streams::StreamPool`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct StreamPoolConfig {
    /// Number of streams in the pool.
    pub size: usize,
    /// Flags applied on creation.
    pub flags: StreamFlags,
    /// GPU ordinal (ignored by the MVP backend).
    pub device_id: u32,
}

impl Default for StreamPoolConfig {
    fn default() -> Self {
        Self {
            size: 4,
            flags: StreamFlags::NonBlocking,
            device_id: 0,
        }
    }
}

impl StreamPoolConfig {
    /// Validate.
    ///
    /// # Errors
    ///
    /// - [`EfeError::InvalidConfig`] if `size == 0`.
    pub fn validate(&self) -> Result<()> {
        if self.size == 0 {
            return Err(EfeError::InvalidConfig(
                "StreamPool size must be >= 1".into(),
            ));
        }
        Ok(())
    }

    /// Builder: set size.
    #[must_use]
    pub fn with_size(mut self, size: usize) -> Self {
        self.size = size;
        self
    }

    /// Builder: set flags.
    #[must_use]
    pub fn with_flags(mut self, flags: StreamFlags) -> Self {
        self.flags = flags;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_validate() {
        StreamPoolConfig::default().validate().expect("ok");
    }

    #[test]
    fn rejects_zero_size() {
        assert!(StreamPoolConfig::default().with_size(0).validate().is_err());
    }

    #[test]
    fn flags_default_non_blocking() {
        assert_eq!(StreamFlags::default(), StreamFlags::NonBlocking);
    }
}
