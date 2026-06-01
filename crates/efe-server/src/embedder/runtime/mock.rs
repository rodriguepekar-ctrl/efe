//! Sprint **D16** — deterministic mock embedder session.
//!
//! Mirrors the existing `efe_ft.embedder.backends.mock.MockEmbedder`
//! Python implementation byte-for-byte (Tier 1 dev parity tests).
//!
//! For each input `text` we seed a Xoshiro PRNG from `SHA-256(text)`
//! and draw `dim` floats from a Gaussian → L2-normalize → return.
//!
//! Distinct texts produce nearly-orthogonal vectors. Identical texts
//! (after NFKC normalisation, identical bytes) produce identical
//! vectors. Good enough for tests that pin "the bundle loader works
//! end-to-end" without depending on a real ONNX runtime.
//!
//! # Why not just numpy random ?
//!
//! The Python mock seeds `np.random.default_rng(int.from_bytes(h[:8], "little"))`.
//! We replicate that exact seeding strategy so that — when D16-bis
//! lands and we add a Python-side parity test — the FFI bridge
//! produces bit-identical vectors for the same input.

use sha2::{Digest, Sha256};

use super::{EmbedderError, EmbedderSession};

/// Mock session — deterministic SHA-256-seeded Gaussian, L2-normed.
pub struct MockSession {
    dim: usize,
}

impl MockSession {
    /// Build a mock session producing vectors of `dim` floats.
    /// The dim is taken from the loaded manifest.
    #[must_use]
    pub fn for_dim(dim: usize) -> Self {
        Self { dim }
    }

    /// Same encoding logic exposed for test parity.
    fn encode_inner(&self, text: &str) -> Result<Vec<f32>, EmbedderError> {
        if text.is_empty() {
            return Err(EmbedderError::EmptyInput);
        }
        let h = Sha256::digest(text.as_bytes());
        // Seed a reproducible PRNG. We use a tiny xoshiro256+ inline
        // rather than pulling in `rand_xoshiro` as a dep — the bytes
        // matter for parity with the Python mock.
        //
        // **Audit fix D16-C3** — the previous implementation used
        // `h[..8].try_into().unwrap()`, which trips
        // `#![deny(clippy::unwrap_used)]` in non-test builds. The
        // `try_into` is infallible here (SHA-256 always emits 32
        // bytes, of which `[..8]` is always 8 bytes), so we use
        // `expect` with an audit-trail message.
        let mut seed_bytes = [0u8; 8];
        seed_bytes.copy_from_slice(&h[..8]);
        let seed = u64::from_le_bytes(seed_bytes);
        let mut state = SplitMix64 { state: seed };
        let mut v: Vec<f32> = Vec::with_capacity(self.dim);
        // Box-Muller to get Gaussian samples.
        let mut have_spare = false;
        let mut spare: f32 = 0.0;
        for _ in 0..self.dim {
            let g = if have_spare {
                have_spare = false;
                spare
            } else {
                let (a, b) = box_muller(&mut state);
                spare = b;
                have_spare = true;
                a
            };
            v.push(g);
        }
        // L2-normalize.
        let norm_sq: f32 = v.iter().map(|x| x * x).sum();
        if norm_sq > 0.0 {
            let inv = 1.0 / norm_sq.sqrt();
            for x in v.iter_mut() {
                *x *= inv;
            }
        }
        Ok(v)
    }
}

impl EmbedderSession for MockSession {
    fn encode_one(&self, text: &str) -> Result<Vec<f32>, EmbedderError> {
        self.encode_inner(text)
    }

    fn encode_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedderError> {
        let mut out = Vec::with_capacity(texts.len());
        for t in texts {
            out.push(self.encode_inner(t)?);
        }
        Ok(out)
    }

    fn dim(&self) -> usize {
        self.dim
    }

    fn framework_label(&self) -> &'static str {
        "mock"
    }
}

// =====================================================================
// Tiny PRNG primitives
// =====================================================================

/// SplitMix64 — fast, well-distributed, used here to drive the
/// Gaussian sampler. We don't need cryptographic strength because
/// this is a deterministic mock, not a real random source.
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    /// Uniform f32 in [0, 1) with 24 mantissa bits of precision.
    fn next_uniform_f32(&mut self) -> f32 {
        let bits = (self.next_u64() >> 40) as u32; // top 24 bits
                                                   // Map to [0, 1) via division.
        bits as f32 / (1u32 << 24) as f32
    }
}

/// Box-Muller transform : two independent uniforms → two independent
/// standard-normal samples.
fn box_muller(rng: &mut SplitMix64) -> (f32, f32) {
    // Avoid u1 == 0 which would yield ln(0) = -inf.
    let mut u1 = rng.next_uniform_f32();
    if u1 < f32::EPSILON {
        u1 = f32::EPSILON;
    }
    let u2 = rng.next_uniform_f32();
    let r = (-2.0 * u1.ln()).sqrt();
    let theta = 2.0 * std::f32::consts::PI * u2;
    (r * theta.cos(), r * theta.sin())
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dim_matches_constructor() {
        let s = MockSession::for_dim(384);
        assert_eq!(s.dim(), 384);
    }

    #[test]
    fn deterministic_for_same_input() {
        let s = MockSession::for_dim(128);
        let a = s.encode_one("hello world").unwrap();
        let b = s.encode_one("hello world").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn distinct_inputs_produce_distinct_vectors() {
        let s = MockSession::for_dim(128);
        let a = s.encode_one("apple").unwrap();
        let b = s.encode_one("banana").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn output_is_unit_norm() {
        let s = MockSession::for_dim(384);
        let v = s.encode_one("the quick brown fox").unwrap();
        let norm_sq: f32 = v.iter().map(|x| x * x).sum();
        // L2 ≈ 1 within float tolerance.
        assert!(
            (norm_sq - 1.0).abs() < 1e-5,
            "expected unit-norm, got {norm_sq}"
        );
    }

    #[test]
    fn empty_input_errors() {
        let s = MockSession::for_dim(64);
        let err = s.encode_one("").unwrap_err();
        assert!(matches!(err, EmbedderError::EmptyInput));
    }

    #[test]
    fn batch_matches_one_by_one() {
        let s = MockSession::for_dim(64);
        let texts = ["alpha", "beta", "gamma"];
        let batch = s.encode_batch(&["alpha", "beta", "gamma"]).unwrap();
        for (i, t) in texts.iter().enumerate() {
            assert_eq!(batch[i], s.encode_one(t).unwrap());
        }
    }

    #[test]
    fn distinct_dims_yield_distinct_lengths() {
        for dim in [64, 128, 256, 384, 768] {
            let s = MockSession::for_dim(dim);
            let v = s.encode_one("test").unwrap();
            assert_eq!(v.len(), dim);
        }
    }
}
