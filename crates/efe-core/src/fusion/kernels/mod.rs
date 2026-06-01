//! Pure math kernels for swarm attention.
//!
//! Each sub-module implements one step of the fusion pipeline, operating
//! on borrowed views from M4 (`LatentView`, `MaskView`, `AnchorsView`,
//! `EmbeddingView`). These kernels are:
//!
//! - **Pure** — no I/O, no allocation in hot loops beyond the initial
//!   scratch (buffers are passed in).
//! - **Deterministic** — same bytes in ⇒ same bytes out on a given
//!   CPU target.
//! - **Panic-free on valid inputs** — preconditions asserted as
//!   `debug_assert!` only; higher layers (orchestration) validate.
//! - **bf16 ↔ f32 robust** — accumulations in f32, storage in bf16.
//!
//! The kernels are `pub(crate)` — their contract is internal. The
//! public API for fusion is `FusionStrategy::run_round`.

pub(crate) mod aggregate;
pub(crate) mod attention;
pub(crate) mod clip;
pub(crate) mod entropy;
pub(crate) mod mix;
pub(crate) mod project;
pub(crate) mod softmax;

/// Immutable bag of parameters passed down to the kernels.
///
/// `Copy` by design — no allocation, no ownership.
#[derive(Copy, Clone, Debug)]
pub(crate) struct KernelParams {
    pub temperature: f32,
    pub mask_self_attention: bool,
    pub beta: f32,
    pub alpha: f32,
    pub clip_max: f32,
}
