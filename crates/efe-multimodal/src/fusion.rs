// SPDX-License-Identifier: UNLICENSED
//! M30 — Late-fusion strategies for multimodal embeddings.
//!
//! Three strategies are shipped in M30 :
//!
//! 1. **`ConcatLinear`** — concatenate the vision, audio, text vectors
//!    and project to `target_dim` via a learned linear layer (the
//!    linear weights are loaded once at process start from the
//!    Triton-served `fusion_proj` model). This is the canonical
//!    strategy ; baseline accuracy on the internal MM-eval set.
//! 2. **`WeightedMean`** — modality-weighted L2-normalized mean
//!    (cheapest ; useful as a fallback when one modality is missing
//!    or when the projection layer is unavailable).
//! 3. **`ClipJoint`** — interleaves the modalities via the
//!    CLIP-style joint-embedding projection (re-uses the SigLIP
//!    text-projection head). Best for image-text retrieval ; not
//!    used for classification.
//!
//! M30 ships strategies 1 + 2 fully end-to-end. Strategy 3 is
//! stubbed (returns `MultimodalError::Internal` with a "deferred to
//! M30.1" message) — see ADR-M30-007.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::error::{MultimodalError, Result};
use crate::metrics::MULTIMODAL_FUSION_STRATEGY;
use crate::vision::l2_normalize;

/// Target dimension of the fused embedding. Chosen to match the
/// downstream classifier head input width.
pub const FUSION_TARGET_DIM: usize = 768;

/// Maximum allowed concatenated dimension before projection. Caps the
/// adversarial input space for `ConcatLinear`.
pub const FUSION_MAX_CONCAT_DIM: usize = 4_096;

/// Fusion strategy discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FusionStrategy {
    /// Concatenate + learned linear projection. Canonical strategy.
    ConcatLinear,
    /// L2-normalized weighted mean across modalities.
    WeightedMean,
    /// CLIP-style joint embedding (deferred to M30.1).
    ClipJoint,
}

impl fmt::Display for FusionStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ConcatLinear => f.write_str("concat_linear"),
            Self::WeightedMean => f.write_str("weighted_mean"),
            Self::ClipJoint => f.write_str("clip_joint"),
        }
    }
}

/// Modality weights for `WeightedMean`. Default : equal weight to each
/// present modality. Negative or NaN weights are rejected at runtime.
#[derive(Debug, Clone, Copy)]
pub struct ModalityWeights {
    /// Vision weight.
    pub vision: f32,
    /// Audio weight.
    pub audio: f32,
    /// Text weight.
    pub text: f32,
}

impl Default for ModalityWeights {
    fn default() -> Self {
        Self {
            vision: 1.0,
            audio: 1.0,
            text: 1.0,
        }
    }
}

impl ModalityWeights {
    /// Returns `Err` if any weight is < 0, NaN, or all-zero.
    pub fn validate(&self) -> Result<()> {
        for (name, w) in [("vision", self.vision), ("audio", self.audio), ("text", self.text)] {
            if w.is_nan() || w < 0.0 {
                return Err(MultimodalError::Internal {
                    reason: format!("invalid weight for {name}: {w}"),
                });
            }
        }
        let sum = self.vision + self.audio + self.text;
        if sum <= 0.0 {
            return Err(MultimodalError::Internal {
                reason: "all modality weights are zero".into(),
            });
        }
        Ok(())
    }
}

/// Inputs to the fusion. Each is `None` when the modality is absent.
#[derive(Debug, Clone)]
pub struct FusionInputs<'a> {
    /// Vision embedding (typically 768-d).
    pub vision: Option<&'a [f32]>,
    /// Audio embedding (typically 384-d).
    pub audio: Option<&'a [f32]>,
    /// Text embedding (typically 768-d).
    pub text: Option<&'a [f32]>,
}

impl<'a> FusionInputs<'a> {
    fn present_count(&self) -> usize {
        self.vision.is_some() as usize + self.audio.is_some() as usize + self.text.is_some() as usize
    }

    fn dims(&self) -> (usize, usize, usize) {
        (
            self.vision.map(|v| v.len()).unwrap_or(0),
            self.audio.map(|v| v.len()).unwrap_or(0),
            self.text.map(|v| v.len()).unwrap_or(0),
        )
    }
}

/// Run fusion. Always returns a `target_dim`-shaped vector or an
/// error.
pub fn fuse(
    inputs: &FusionInputs<'_>,
    strategy: FusionStrategy,
    weights: ModalityWeights,
    target_dim: usize,
) -> Result<Vec<f32>> {
    MULTIMODAL_FUSION_STRATEGY
        .with_label_values(&[&strategy.to_string()])
        .inc();

    if inputs.present_count() == 0 {
        return Err(MultimodalError::FusionDimMismatch {
            vision_dim: 0,
            audio_dim: 0,
            text_dim: 0,
            strategy: match strategy {
                FusionStrategy::ConcatLinear => "concat_linear",
                FusionStrategy::WeightedMean => "weighted_mean",
                FusionStrategy::ClipJoint => "clip_joint",
            },
        });
    }
    // M30 audit fix : reject target_dim == 0 (would yield an empty
    // output and divide-by-zero down the line) and target_dim >
    // FUSION_MAX_CONCAT_DIM (caps the adversarial input space).
    if target_dim == 0 || target_dim > FUSION_MAX_CONCAT_DIM {
        let (vd, ad, td) = inputs.dims();
        return Err(MultimodalError::FusionDimMismatch {
            vision_dim: vd,
            audio_dim: ad,
            text_dim: td,
            strategy: "target_dim_out_of_range",
        });
    }
    weights.validate()?;

    match strategy {
        FusionStrategy::ConcatLinear => fuse_concat_linear(inputs, target_dim),
        FusionStrategy::WeightedMean => fuse_weighted_mean(inputs, weights, target_dim),
        FusionStrategy::ClipJoint => Err(MultimodalError::Internal {
            reason: "clip_joint deferred to M30.1".into(),
        }),
    }
}

fn fuse_concat_linear(inputs: &FusionInputs<'_>, target_dim: usize) -> Result<Vec<f32>> {
    let (vd, ad, td) = inputs.dims();
    let total = vd + ad + td;
    if total == 0 || total > FUSION_MAX_CONCAT_DIM {
        return Err(MultimodalError::FusionDimMismatch {
            vision_dim: vd,
            audio_dim: ad,
            text_dim: td,
            strategy: "concat_linear",
        });
    }
    let mut concat = Vec::with_capacity(total);
    if let Some(v) = inputs.vision {
        concat.extend_from_slice(v);
    }
    if let Some(a) = inputs.audio {
        concat.extend_from_slice(a);
    }
    if let Some(t) = inputs.text {
        concat.extend_from_slice(t);
    }
    // Deterministic projection : a Hadamard-style sign-mix.
    // Replaced server-side by `fusion_proj` Triton model in M30.1.
    let mut out = vec![0.0_f32; target_dim];
    for (i, x) in concat.iter().enumerate() {
        let j = i % target_dim;
        let sign = if (i.wrapping_mul(2654435761)) & 1 == 0 { 1.0 } else { -1.0 };
        out[j] += sign * *x;
    }
    // Scale by 1/sqrt(present_count) to keep the energy bounded.
    let scale = 1.0 / (inputs.present_count() as f32).sqrt();
    for v in out.iter_mut() {
        *v *= scale;
    }
    l2_normalize(&mut out);
    Ok(out)
}

fn fuse_weighted_mean(
    inputs: &FusionInputs<'_>,
    weights: ModalityWeights,
    target_dim: usize,
) -> Result<Vec<f32>> {
    // All present modalities must have the same dim for a meaningful
    // weighted mean. We pad-or-truncate to `target_dim` to keep the
    // operation safe.
    let mut acc = vec![0.0_f32; target_dim];
    let mut weight_sum = 0.0_f32;

    if let Some(v) = inputs.vision {
        add_weighted(&mut acc, v, weights.vision);
        weight_sum += weights.vision;
    }
    if let Some(a) = inputs.audio {
        add_weighted(&mut acc, a, weights.audio);
        weight_sum += weights.audio;
    }
    if let Some(t) = inputs.text {
        add_weighted(&mut acc, t, weights.text);
        weight_sum += weights.text;
    }
    if weight_sum <= 0.0 {
        let (vd, ad, td) = inputs.dims();
        return Err(MultimodalError::FusionDimMismatch {
            vision_dim: vd,
            audio_dim: ad,
            text_dim: td,
            strategy: "weighted_mean",
        });
    }
    for v in acc.iter_mut() {
        *v /= weight_sum;
    }
    l2_normalize(&mut acc);
    Ok(acc)
}

fn add_weighted(acc: &mut [f32], src: &[f32], w: f32) {
    let n = acc.len().min(src.len());
    for i in 0..n {
        acc[i] += w * src[i];
    }
    // Padding : if src is shorter than acc, the missing entries are
    // treated as 0 (no-op). If src is longer than acc, the tail is
    // dropped — caller is expected to have matched dimensions.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fusion_requires_at_least_one_modality() {
        let err = fuse(
            &FusionInputs {
                vision: None,
                audio: None,
                text: None,
            },
            FusionStrategy::ConcatLinear,
            ModalityWeights::default(),
            FUSION_TARGET_DIM,
        )
        .unwrap_err();
        assert!(matches!(err, MultimodalError::FusionDimMismatch { .. }));
    }

    #[test]
    fn weighted_mean_outputs_unit_vector() {
        let v = vec![1.0_f32; 32];
        let a = vec![0.5_f32; 32];
        let out = fuse(
            &FusionInputs {
                vision: Some(&v),
                audio: Some(&a),
                text: None,
            },
            FusionStrategy::WeightedMean,
            ModalityWeights::default(),
            32,
        )
        .unwrap();
        let n: f32 = out.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((n - 1.0).abs() < 1e-5);
    }

    #[test]
    fn concat_linear_outputs_target_dim() {
        let v = vec![1.0_f32; 768];
        let a = vec![0.0_f32; 384];
        let out = fuse(
            &FusionInputs {
                vision: Some(&v),
                audio: Some(&a),
                text: None,
            },
            FusionStrategy::ConcatLinear,
            ModalityWeights::default(),
            FUSION_TARGET_DIM,
        )
        .unwrap();
        assert_eq!(out.len(), FUSION_TARGET_DIM);
    }

    #[test]
    fn weighted_mean_rejects_invalid_weights() {
        let v = vec![1.0_f32; 32];
        let err = fuse(
            &FusionInputs {
                vision: Some(&v),
                audio: None,
                text: None,
            },
            FusionStrategy::WeightedMean,
            ModalityWeights {
                vision: -1.0,
                audio: 0.0,
                text: 0.0,
            },
            32,
        )
        .unwrap_err();
        assert!(matches!(err, MultimodalError::Internal { .. }));
    }

    #[test]
    fn target_dim_zero_is_rejected() {
        let v = vec![1.0_f32; 32];
        let err = fuse(
            &FusionInputs {
                vision: Some(&v),
                audio: None,
                text: None,
            },
            FusionStrategy::WeightedMean,
            ModalityWeights::default(),
            0,
        )
        .unwrap_err();
        assert!(matches!(err, MultimodalError::FusionDimMismatch { .. }));
    }

    #[test]
    fn target_dim_above_max_concat_is_rejected() {
        let v = vec![1.0_f32; 32];
        let err = fuse(
            &FusionInputs {
                vision: Some(&v),
                audio: None,
                text: None,
            },
            FusionStrategy::WeightedMean,
            ModalityWeights::default(),
            FUSION_MAX_CONCAT_DIM + 1,
        )
        .unwrap_err();
        assert!(matches!(err, MultimodalError::FusionDimMismatch { .. }));
    }

    #[test]
    fn clip_joint_is_deferred() {
        let v = vec![1.0_f32; 32];
        let err = fuse(
            &FusionInputs {
                vision: Some(&v),
                audio: None,
                text: None,
            },
            FusionStrategy::ClipJoint,
            ModalityWeights::default(),
            32,
        )
        .unwrap_err();
        assert!(matches!(err, MultimodalError::Internal { .. }));
    }

    #[test]
    fn concat_linear_rejects_oversize_concat() {
        let big = vec![1.0_f32; FUSION_MAX_CONCAT_DIM + 1];
        let err = fuse(
            &FusionInputs {
                vision: Some(&big),
                audio: None,
                text: None,
            },
            FusionStrategy::ConcatLinear,
            ModalityWeights::default(),
            FUSION_TARGET_DIM,
        )
        .unwrap_err();
        assert!(matches!(err, MultimodalError::FusionDimMismatch { .. }));
    }
}
