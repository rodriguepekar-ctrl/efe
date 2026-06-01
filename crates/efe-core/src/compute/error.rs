//! [`ComputeError`] — the unified error type for all [`crate::compute`] operations.

use thiserror::Error;

use super::device::Device;

/// Errors returned by [`crate::compute::ComputeBackend`] operations.
///
/// Every variant carries enough diagnostic context to debug without a
/// stack trace. No operation returning `Result<_, ComputeError>` is
/// allowed to panic on well-formed input.
#[derive(Debug, Error, Clone)]
pub enum ComputeError {
    /// Tensor shape did not match the expected shape for the operation.
    #[error("{op}: shape mismatch (expected {expected}, got {actual})")]
    ShapeMismatch {
        /// Operation name, e.g. `"matmul"`.
        op: &'static str,
        /// Human-readable expected shape string.
        expected: String,
        /// Human-readable actual shape string.
        actual: String,
    },

    /// Axis index is out of range for the tensor's rank.
    #[error("invalid axis {axis} for tensor of rank {rank}")]
    InvalidAxis {
        /// The rejected axis (possibly negative).
        axis: i32,
        /// The tensor's rank.
        rank: usize,
    },

    /// Numerical instability detected (NaN, Inf, overflow, …).
    #[error("{op}: numerical instability: {detail}")]
    NumericalInstability {
        /// Operation name.
        op: &'static str,
        /// What went wrong.
        detail: String,
    },

    /// Tensor lives on a different device than the backend.
    #[error("device mismatch: tensor on {tensor_dev}, backend on {backend_dev}")]
    DeviceMismatch {
        /// Device of the tensor argument.
        tensor_dev: Device,
        /// Device of the backend operating on it.
        backend_dev: Device,
    },

    /// Memory allocation failed.
    #[error("allocation failed: {bytes} bytes on {device}")]
    AllocationFailed {
        /// Requested size in bytes.
        bytes: usize,
        /// Target device.
        device: Device,
    },

    /// A GPU kernel launch failed.
    #[error("kernel launch failed: {kernel}: {reason}")]
    KernelLaunchFailed {
        /// Kernel name.
        kernel: &'static str,
        /// Backend-provided failure reason.
        reason: String,
    },

    /// dtype is not supported by the op.
    #[error("dtype {dtype} is not supported by op {op}")]
    DtypeNotSupported {
        /// The offending dtype name.
        dtype: &'static str,
        /// Operation name.
        op: &'static str,
    },

    /// Operation requires a contiguous tensor but the argument is a view.
    #[error("{op}: tensor is not contiguous")]
    NotContiguous {
        /// Operation name.
        op: &'static str,
    },

    /// Attempted to get mutable access to an aliased tensor.
    #[error("tensor is aliased (shared Arc); cannot get mutable slice")]
    Aliased,

    /// Operation is not yet implemented on the current backend.
    #[error("{op}: not implemented on backend {backend}")]
    NotImplemented {
        /// Operation name.
        op: &'static str,
        /// Backend name, e.g. `"cpu"` or `"cuda"`.
        backend: &'static str,
    },

    /// An internal invariant was violated (i.e. a bug).
    #[error("internal invariant violated: {detail}")]
    Internal {
        /// Free-form description of the invariant.
        detail: String,
    },
}

/// Convenience type alias.
pub type Result<T> = std::result::Result<T, ComputeError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_shape_mismatch() {
        let e = ComputeError::ShapeMismatch {
            op: "matmul",
            expected: "[2, 3]".into(),
            actual: "[2, 4]".into(),
        };
        assert!(format!("{e}").contains("matmul"));
        assert!(format!("{e}").contains("[2, 3]"));
    }

    #[test]
    fn display_invalid_axis() {
        let e = ComputeError::InvalidAxis { axis: 5, rank: 2 };
        assert_eq!(format!("{e}"), "invalid axis 5 for tensor of rank 2");
    }

    #[test]
    fn not_implemented_variant() {
        let e = ComputeError::NotImplemented {
            op: "matmul",
            backend: "cpu",
        };
        assert!(format!("{e}").contains("not implemented"));
    }
}
