//! [`Device`] — which compute unit owns a tensor's data.

use std::fmt;

/// A compute unit that can own tensor memory.
///
/// Currently supports CPU and (behind the `cuda` feature) one or more
/// NVIDIA GPUs identified by their zero-based device ordinal.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum Device {
    /// Host CPU memory.
    #[default]
    Cpu,
    /// NVIDIA GPU at the given ordinal.
    #[cfg(feature = "cuda")]
    Cuda(u32),
}

impl Device {
    /// Returns `true` if the device is the CPU.
    #[inline]
    pub fn is_cpu(self) -> bool {
        matches!(self, Device::Cpu)
    }

    /// Returns `true` if the device is a CUDA GPU.
    #[cfg(feature = "cuda")]
    #[inline]
    pub fn is_cuda(self) -> bool {
        matches!(self, Device::Cuda(_))
    }

    /// The CUDA device ordinal, if any.
    #[cfg(feature = "cuda")]
    #[inline]
    pub fn cuda_ordinal(self) -> Option<u32> {
        match self {
            Device::Cuda(n) => Some(n),
            Device::Cpu => None,
        }
    }
}

impl fmt::Display for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Device::Cpu => write!(f, "cpu"),
            #[cfg(feature = "cuda")]
            Device::Cuda(n) => write!(f, "cuda:{n}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_default_is_cpu() {
        assert_eq!(Device::default(), Device::Cpu);
        assert!(Device::default().is_cpu());
    }

    #[test]
    fn device_display_cpu() {
        assert_eq!(format!("{}", Device::Cpu), "cpu");
    }

    #[test]
    fn device_is_hashable_and_eq() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(Device::Cpu);
        set.insert(Device::Cpu);
        assert_eq!(set.len(), 1);
    }
}
