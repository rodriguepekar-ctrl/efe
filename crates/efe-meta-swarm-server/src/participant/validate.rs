//! Request validators for the participant's gRPC handlers.
//!
//! These checks are **defence in depth** — Phase-1's `MockParticipant`
//! would catch most shape errors via `DimensionMismatch`, but :
//!
//!   * Phase-1 maps `DimensionMismatch` through `meta_swarm_error_to_status`
//!     to `FAILED_PRECONDITION` ; we want a clearer diagnostic than the
//!     post-Phase-1 path can offer.
//!   * `Tensor::from_slice` happily accepts NaN / ±∞ ; the participant
//!     boundary is the right place to reject them (threat **T11** —
//!     silent NaN propagation poisons every future Step).
//!   * The wire-level invariant `pulled.dim == pulled.data.len()` is
//!     not Phase-1's concern.

use efe_meta_swarm_proto::meta_swarm::{ResetParticipantRequest, StepRequest, TensorF32};

use crate::error::ServerError;
use crate::participant::local::LocalParticipant;

/// Validate a `StepRequest` against the local participant's expected
/// dimensions.  Returns `(pulled, step_index)` ready for
/// [`LocalParticipant::step`].
///
/// # Errors
///
/// Returns [`ServerError::Config`] (→ `FAILED_PRECONDITION`) for :
///   * missing `pulled_from_meta`,
///   * `pulled.dim ≠ local.dim()`,
///   * envelope corruption (`pulled.dim ≠ pulled.data.len()`),
///   * any non-finite element in `pulled.data`.
pub fn validate_step<'a>(
    req: &'a StepRequest,
    local: &LocalParticipant,
) -> Result<(&'a TensorF32, u32), ServerError> {
    let pulled = req
        .pulled_from_meta
        .as_ref()
        .ok_or_else(|| ServerError::Config("StepRequest.pulled_from_meta is required".into()))?;
    validate_tensor_shape(pulled, local.dim())?;
    validate_tensor_finite(pulled)?;
    Ok((pulled, req.step_index))
}

/// Validate a `ResetParticipantRequest`.  Returns `Some(tensor)` if a
/// custom initial state was provided (validated for shape + finite),
/// or `None` for the default seeded reset.
///
/// # Errors
///
/// Same surface as [`validate_step`] (when an `initial` is present).
pub fn validate_reset<'a>(
    req: &'a ResetParticipantRequest,
    local: &LocalParticipant,
) -> Result<Option<&'a TensorF32>, ServerError> {
    let Some(initial) = req.initial.as_ref() else {
        return Ok(None);
    };
    validate_tensor_shape(initial, local.dim())?;
    validate_tensor_finite(initial)?;
    Ok(Some(initial))
}

fn validate_tensor_shape(t: &TensorF32, expected_dim: usize) -> Result<(), ServerError> {
    if t.dim as usize != expected_dim {
        return Err(ServerError::Config(format!(
            "TensorF32.dim = {} but participant expects {expected_dim}",
            t.dim,
        )));
    }
    if t.dim as usize != t.data.len() {
        return Err(ServerError::Config(format!(
            "TensorF32 envelope corrupt : dim = {} but data.len() = {}",
            t.dim,
            t.data.len(),
        )));
    }
    Ok(())
}

fn validate_tensor_finite(t: &TensorF32) -> Result<(), ServerError> {
    if let Some((i, v)) = t.data.iter().enumerate().find(|(_, v)| !v.is_finite()) {
        return Err(ServerError::Config(format!(
            "TensorF32.data[{i}] = {v} is not finite (NaN or ±∞)",
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::ParticipantManifest;

    fn local() -> LocalParticipant {
        LocalParticipant::from_manifest_row(&ParticipantManifest {
            id: "p-1".into(),
            dim: 8,
            kappa: Some(0.6),
            seed: Some(11),
            blas_backend: "none".into(),
            controller: None,
        })
        .expect("local")
    }

    fn good_step_req() -> StepRequest {
        StepRequest {
            pulled_from_meta: Some(TensorF32 {
                dim: 8,
                data: vec![0.1; 8],
            }),
            step_index: 42,
        }
    }

    #[test]
    fn validate_step_happy_path() {
        let req = good_step_req();
        let (pulled, idx) = validate_step(&req, &local()).expect("ok");
        assert_eq!(pulled.dim, 8);
        assert_eq!(idx, 42);
    }

    #[test]
    fn validate_step_rejects_missing_pulled() {
        let mut r = good_step_req();
        r.pulled_from_meta = None;
        let err = validate_step(&r, &local()).unwrap_err();
        assert!(matches!(err, ServerError::Config(ref m) if m.contains("required")));
    }

    #[test]
    fn validate_step_rejects_dim_mismatch() {
        let mut r = good_step_req();
        r.pulled_from_meta = Some(TensorF32 {
            dim: 7,
            data: vec![0.0; 7],
        });
        let err = validate_step(&r, &local()).unwrap_err();
        assert!(matches!(err, ServerError::Config(ref m) if m.contains("expects 8")));
    }

    #[test]
    fn validate_step_rejects_envelope_corruption() {
        let mut r = good_step_req();
        r.pulled_from_meta = Some(TensorF32 {
            dim: 8,
            data: vec![0.0; 4], // dim says 8 but only 4 elements
        });
        let err = validate_step(&r, &local()).unwrap_err();
        assert!(matches!(err, ServerError::Config(ref m) if m.contains("envelope corrupt")));
    }

    #[test]
    fn validate_step_rejects_nan() {
        let mut r = good_step_req();
        r.pulled_from_meta.as_mut().unwrap().data[3] = f32::NAN;
        let err = validate_step(&r, &local()).unwrap_err();
        assert!(matches!(err, ServerError::Config(ref m) if m.contains("not finite")));
        assert!(matches!(err, ServerError::Config(ref m) if m.contains("[3]")));
    }

    #[test]
    fn validate_step_rejects_positive_infinity() {
        let mut r = good_step_req();
        r.pulled_from_meta.as_mut().unwrap().data[0] = f32::INFINITY;
        let err = validate_step(&r, &local()).unwrap_err();
        assert!(matches!(err, ServerError::Config(ref m) if m.contains("not finite")));
    }

    #[test]
    fn validate_step_rejects_negative_infinity() {
        let mut r = good_step_req();
        r.pulled_from_meta.as_mut().unwrap().data[5] = f32::NEG_INFINITY;
        let err = validate_step(&r, &local()).unwrap_err();
        assert!(matches!(err, ServerError::Config(_)));
    }

    #[test]
    fn validate_reset_no_initial_returns_none() {
        let r = ResetParticipantRequest { initial: None };
        let opt = validate_reset(&r, &local()).expect("ok");
        assert!(opt.is_none());
    }

    #[test]
    fn validate_reset_with_initial_happy_path() {
        let r = ResetParticipantRequest {
            initial: Some(TensorF32 {
                dim: 8,
                data: vec![0.25; 8],
            }),
        };
        let opt = validate_reset(&r, &local()).expect("ok");
        assert!(opt.is_some());
        assert_eq!(opt.unwrap().dim, 8);
    }

    #[test]
    fn validate_reset_with_initial_rejects_dim_mismatch() {
        let r = ResetParticipantRequest {
            initial: Some(TensorF32 {
                dim: 4,
                data: vec![0.0; 4],
            }),
        };
        let err = validate_reset(&r, &local()).unwrap_err();
        assert!(matches!(err, ServerError::Config(_)));
    }

    #[test]
    fn validate_reset_with_initial_rejects_nan() {
        let mut data = vec![0.0_f32; 8];
        data[2] = f32::NAN;
        let r = ResetParticipantRequest {
            initial: Some(TensorF32 { dim: 8, data }),
        };
        let err = validate_reset(&r, &local()).unwrap_err();
        assert!(matches!(err, ServerError::Config(ref m) if m.contains("not finite")));
    }
}
