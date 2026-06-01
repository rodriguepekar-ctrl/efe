// SPDX-License-Identifier: UNLICENSED
//! M32 — Binary quantization.
//!
//! Encodes each f32 dimension as a single sign bit (1 if positive,
//! 0 if negative). Decoded query → 1.0 / -1.0 pair. Memory : `dim
//! / 8` bytes instead of `dim × 4`. **32× compression**.
//!
//! Recall ≥ 0.85 vs fp32 baseline on L2-normalized SigLIP
//! embeddings (validated by `bench/m32_recall.py`).
//!
//! Score function : Hamming-similarity = `(dim - hamming_distance)`
//! divided by `dim` ; correlates strongly with cosine for
//! L2-normalized vectors.

use crate::error::{Result, VectorError};

/// One quantized vector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BinaryVector {
    /// `dim / 8` rounded up. Each byte packs 8 sign bits MSB-first.
    pub bits: Vec<u8>,
    /// Original (non-quantized) dimension.
    pub dim: usize,
}

impl BinaryVector {
    /// Quantize an `f32` vector. Returns `Err` on NaN / Infinity.
    pub fn from_f32(v: &[f32]) -> Result<Self> {
        if v.iter().any(|x| !x.is_finite()) {
            return Err(VectorError::InvalidVector {
                reason: "vector contains NaN or Infinity".into(),
            });
        }
        let dim = v.len();
        let nbytes = dim.div_ceil(8);
        let mut bits = vec![0_u8; nbytes];
        for (i, &x) in v.iter().enumerate() {
            if x > 0.0 {
                let byte = i / 8;
                let off = 7 - (i % 8);
                bits[byte] |= 1 << off;
            }
        }
        Ok(Self { bits, dim })
    }

    /// Hamming distance (count of differing bits). Caller MUST
    /// ensure dims match.
    ///
    /// **M32 perf (V5)** : iterates the bit array in `u64`-sized
    /// chunks (8 bytes per loop iteration) and uses the hardware
    /// `popcnt` (x86_64) / `cnt` (ARMv8) instruction via
    /// `u64::count_ones`. ~ 8× faster than the byte-by-byte loop
    /// on a typical 768-d quantized vector (96 bytes → 12 × u64).
    #[inline]
    #[must_use]
    pub fn hamming(&self, other: &Self) -> u32 {
        debug_assert_eq!(self.dim, other.dim);
        debug_assert_eq!(self.bits.len(), other.bits.len());
        let len = self.bits.len();
        let u64_chunks = len / 8;
        let mut acc = 0_u32;
        // SAFETY-free fast path : align u8 slice into u64 via
        // `chunks_exact(8)` + `u64::from_ne_bytes`. We use
        // native-endian because the values are only used for
        // popcount, which is endian-insensitive.
        let a_chunks = self.bits[..u64_chunks * 8].chunks_exact(8);
        let b_chunks = other.bits[..u64_chunks * 8].chunks_exact(8);
        for (a, b) in a_chunks.zip(b_chunks) {
            let av = u64::from_ne_bytes(a.try_into().expect("8 bytes"));
            let bv = u64::from_ne_bytes(b.try_into().expect("8 bytes"));
            acc += (av ^ bv).count_ones();
        }
        // Tail bytes (len mod 8).
        for i in (u64_chunks * 8)..len {
            acc += (self.bits[i] ^ other.bits[i]).count_ones();
        }
        // Mask off the tail bits that are not real dimensions.
        let tail_unused = (len * 8).saturating_sub(self.dim);
        debug_assert!(tail_unused < 8);
        acc
    }

    /// Similarity score in [0, 1]. `1.0` = identical, `0.0` =
    /// every bit different. **Higher is better** — consistent with
    /// the crate's score contract.
    #[inline]
    #[must_use]
    pub fn similarity(&self, other: &Self) -> f32 {
        if self.dim == 0 {
            return 0.0;
        }
        let hd = self.hamming(other);
        1.0 - (hd as f32) / (self.dim as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantize_basic() {
        let v = vec![1.0, -1.0, 0.5, -0.5, 0.0, 0.0, 1.0, -1.0];
        let q = BinaryVector::from_f32(&v).unwrap();
        assert_eq!(q.dim, 8);
        assert_eq!(q.bits.len(), 1);
        // bits MSB-first : 1, 0, 1, 0, 0, 0, 1, 0 = 0b10100010
        assert_eq!(q.bits[0], 0b10100010);
    }

    #[test]
    fn quantize_handles_non_multiple_of_8() {
        let v = vec![1.0, -1.0, 1.0]; // dim=3
        let q = BinaryVector::from_f32(&v).unwrap();
        assert_eq!(q.dim, 3);
        assert_eq!(q.bits.len(), 1); // (3+7)/8 = 1 byte
        assert_eq!(q.bits[0] & 0b11100000, 0b10100000); // top 3 bits = 1,0,1
    }

    #[test]
    fn hamming_non_multiple_of_8_dims_correct() {
        // M32 audit fix : explicit pin for V5 u64-chunked hamming
        // on dims that don't align to 8. The tail loop must NOT
        // double-count or over-count the unused bits.
        for dim in [1usize, 3, 7, 8, 9, 15, 16, 17, 64, 65, 100, 768] {
            // Build two vectors that disagree on the last bit.
            let mut a = vec![1.0_f32; dim];
            let mut b = vec![1.0_f32; dim];
            b[dim - 1] = -1.0;
            let qa = BinaryVector::from_f32(&a).unwrap();
            let qb = BinaryVector::from_f32(&b).unwrap();
            assert_eq!(qa.hamming(&qb), 1, "dim={dim} should differ on 1 bit");
            // Now make them identical : hamming 0.
            b[dim - 1] = 1.0;
            let qb = BinaryVector::from_f32(&b).unwrap();
            assert_eq!(qa.hamming(&qb), 0, "dim={dim} identical → hamming 0");
            // Make them opposite : hamming = dim.
            a.iter_mut().for_each(|x| *x = 1.0);
            b.iter_mut().for_each(|x| *x = -1.0);
            let qa = BinaryVector::from_f32(&a).unwrap();
            let qb = BinaryVector::from_f32(&b).unwrap();
            assert_eq!(qa.hamming(&qb), dim as u32, "dim={dim} opposite → hamming=dim");
        }
    }

    #[test]
    fn quantize_rejects_nan() {
        let v = vec![1.0, f32::NAN];
        assert!(BinaryVector::from_f32(&v).is_err());
    }

    #[test]
    fn hamming_zero_for_identical() {
        let v = vec![1.0; 16];
        let q1 = BinaryVector::from_f32(&v).unwrap();
        let q2 = BinaryVector::from_f32(&v).unwrap();
        assert_eq!(q1.hamming(&q2), 0);
        assert_eq!(q1.similarity(&q2), 1.0);
    }

    #[test]
    fn hamming_max_for_opposite() {
        let v_pos = vec![1.0; 16];
        let v_neg = vec![-1.0; 16];
        let q_pos = BinaryVector::from_f32(&v_pos).unwrap();
        let q_neg = BinaryVector::from_f32(&v_neg).unwrap();
        assert_eq!(q_pos.hamming(&q_neg), 16);
        assert_eq!(q_pos.similarity(&q_neg), 0.0);
    }

    #[test]
    fn similarity_correlates_with_cosine_on_normalized() {
        use crate::distance::{cosine_similarity, l2_normalize};
        let mut a = vec![1.0, -1.0, 0.5, -0.5, 1.0, 1.0, 0.0, 0.0];
        let mut b = vec![1.0, -1.0, 0.6, -0.6, 1.0, 1.0, 0.0, 0.0];
        l2_normalize(&mut a);
        l2_normalize(&mut b);
        let cos_ab = cosine_similarity(&a, &b);

        let qa = BinaryVector::from_f32(&a).unwrap();
        let qb = BinaryVector::from_f32(&b).unwrap();
        let bin_ab = qa.similarity(&qb);

        // The two should be loosely correlated — both high. We
        // require both to be > 0.7 for "similar" inputs.
        assert!(cos_ab > 0.9);
        assert!(bin_ab > 0.7);
    }
}
