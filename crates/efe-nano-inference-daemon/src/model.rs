//! **Sprint 8 — N11 days 13-14** : guest-side model loader.
//!
//! # Sprint 8 scope
//!
//! Sprint 8 ships the **loader skeleton** + integrity-verification +
//! a stub inference adaptor.  Sprint 8 day 15 / Sprint 9 wires the
//! real ONNX runtime (via the `ort` crate or `tract`).
//!
//! The skeleton is functional : it locates the model file, verifies
//! its SHA-256 against the expected hash baked into the rootfs at
//! build time, and exposes an [`InferenceEngine`] trait so the daemon's
//! handler doesn't depend on a specific ML backend.

use anyhow::{anyhow, Context, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

// ─────────────────────────────────────────────────────────────────────
//  Inference contract
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N11** : per-VM inference engine.
///
/// One impl per model family.  Sprint 8 ships [`StubEngine`] (returns
/// echo-style responses, useful for the integration tests that exercise
/// the boot → vsock → infer round-trip without a real model).  Sprint 8
/// day 15 wires [`OrtEngine`] for ONNX models.
pub trait InferenceEngine: Send + Sync {
    /// Invoke the model on `input`, returning a JSON output.
    fn infer(&self, input: &serde_json::Value) -> Result<InferOutput>;

    /// Estimated steady-state memory footprint of the loaded model.
    fn memory_mb(&self) -> u32;

    /// Stable identifier of the engine implementation.
    fn engine_name(&self) -> &'static str;
}

/// Output of one inference call.
#[derive(Debug, Clone)]
pub struct InferOutput {
    pub value: serde_json::Value,
    pub model_invocation_micros: u64,
    pub model_memory_high_water_mb: u32,
}

// ─────────────────────────────────────────────────────────────────────
//  Bundle : model file + schemas + integrity check
// ─────────────────────────────────────────────────────────────────────

/// One on-disk model bundle (model.onnx + schemas + sha256 file).
///
/// Sprint 8 day 13 : `model_path` / `input_schema` / `output_schema` are
/// recorded but not yet consumed (the host validates input/output
/// schemas).  Sprint 9 wires guest-side schema enforcement for defense
/// in depth.
pub struct ModelBundle {
    #[allow(dead_code)]
    pub model_path: PathBuf,
    #[allow(dead_code)]
    pub input_schema: serde_json::Value,
    #[allow(dead_code)]
    pub output_schema: serde_json::Value,
    pub engine: Box<dyn InferenceEngine>,
}

impl ModelBundle {
    pub fn estimated_memory_mb(&self) -> u32 {
        self.engine.memory_mb()
    }

    pub fn engine_name(&self) -> &'static str {
        self.engine.engine_name()
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Loader
// ─────────────────────────────────────────────────────────────────────

pub struct ModelLoader;

impl ModelLoader {
    /// Load the model bundle from `/opt/models/{id}/{version}/`.
    ///
    /// Verifies SHA-256 against `/etc/efe/model.sha256` if present (
    /// rootfs-level integrity check).  Falls back to "trust on first
    /// load" if the hash file is absent — useful for unit tests.
    pub fn load(model_id: &str, model_version: &str) -> Result<ModelBundle> {
        let model_dir = super::model_dir(model_id, model_version);
        let model_path = model_dir.join("model.onnx");

        // Try to load input/output schemas (optional).
        let input_schema = Self::load_json_if_present(model_dir.join("input_schema.json"))
            .unwrap_or_else(|| serde_json::json!({}));
        let output_schema = Self::load_json_if_present(model_dir.join("output_schema.json"))
            .unwrap_or_else(|| serde_json::json!({}));

        // Integrity check (skipped if the model file doesn't exist —
        // Sprint 8 stub deployment).
        if model_path.exists() {
            Self::verify_integrity(&model_path).context("model integrity check failed")?;
        } else {
            tracing::warn!(
                model_id, model_version,
                "model file not found at {} — running in STUB mode (Sprint 8 dev)",
                model_path.display()
            );
        }

        // Choose engine.  Sprint 8 : always StubEngine.
        let engine: Box<dyn InferenceEngine> =
            Box::new(StubEngine::new(model_id, model_version));

        Ok(ModelBundle {
            model_path,
            input_schema,
            output_schema,
            engine,
        })
    }

    fn load_json_if_present(path: PathBuf) -> Option<serde_json::Value> {
        let bytes = std::fs::read(path).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn verify_integrity(model_path: &Path) -> Result<()> {
        let expected_path = Path::new("/etc/efe/model.sha256");
        if !expected_path.exists() {
            tracing::warn!(
                "{} absent — skipping integrity check (Sprint 8 dev)",
                expected_path.display()
            );
            return Ok(());
        }
        let expected_hex = std::fs::read_to_string(expected_path)
            .context("read expected hash")?
            .trim()
            .to_string();
        if expected_hex.len() != 64 {
            return Err(anyhow!(
                "{} should be a 64-char SHA-256 hex string, got {} chars",
                expected_path.display(),
                expected_hex.len()
            ));
        }
        let actual = Self::sha256_of(model_path)?;
        let actual_hex = hex_encode(&actual);
        if actual_hex.to_lowercase() != expected_hex.to_lowercase() {
            return Err(anyhow!(
                "model SHA-256 mismatch : expected {expected_hex}, got {actual_hex}"
            ));
        }
        Ok(())
    }

    fn sha256_of(path: &Path) -> Result<[u8; 32]> {
        use std::io::Read;
        let mut f = std::fs::File::open(path)
            .with_context(|| format!("open {} for hashing", path.display()))?;
        let mut hasher = Sha256::new();
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let d = hasher.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&d);
        Ok(out)
    }
}

fn hex_encode(bytes: &[u8; 32]) -> String {
    let mut s = String::with_capacity(64);
    for b in bytes {
        use std::fmt::Write;
        write!(s, "{b:02x}").ok();
    }
    s
}

// ─────────────────────────────────────────────────────────────────────
//  StubEngine — Sprint 8 default
// ─────────────────────────────────────────────────────────────────────

/// **Sprint 8 — N11** : stub inference engine.  Returns a deterministic
/// echo-style output.  Replaced by `OrtEngine` in Sprint 8 day 15.
pub struct StubEngine {
    model_id: String,
    model_version: String,
}

impl StubEngine {
    pub fn new(model_id: &str, model_version: &str) -> Self {
        Self {
            model_id: model_id.to_string(),
            model_version: model_version.to_string(),
        }
    }
}

impl InferenceEngine for StubEngine {
    fn infer(&self, input: &serde_json::Value) -> Result<InferOutput> {
        let started = std::time::Instant::now();
        // Echo-style with metadata so the integration test can verify
        // the guest actually ran.
        let value = serde_json::json!({
            "stub": true,
            "model_id": self.model_id,
            "model_version": self.model_version,
            "echoed_input": input,
        });
        Ok(InferOutput {
            value,
            model_invocation_micros: started.elapsed().as_micros().min(u64::MAX as u128) as u64,
            model_memory_high_water_mb: 32,
        })
    }

    fn memory_mb(&self) -> u32 {
        32
    }

    fn engine_name(&self) -> &'static str {
        "stub"
    }
}

// ─────────────────────────────────────────────────────────────────────
//  Tests
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_engine_returns_echo() {
        let engine = StubEngine::new("foo", "1.0.0");
        let out = engine
            .infer(&serde_json::json!({"text": "hello"}))
            .unwrap();
        assert_eq!(out.value["stub"], true);
        assert_eq!(out.value["echoed_input"]["text"], "hello");
        assert_eq!(out.value["model_id"], "foo");
    }

    #[test]
    fn stub_engine_memory_is_reasonable() {
        let engine = StubEngine::new("foo", "1.0.0");
        assert!(engine.memory_mb() < 256);
    }

    #[test]
    fn stub_engine_name() {
        let engine = StubEngine::new("foo", "1.0.0");
        assert_eq!(engine.engine_name(), "stub");
    }

    #[test]
    fn sha256_of_known_input_matches_expected() {
        let tmp = std::env::temp_dir().join(format!(
            "efe-test-model-{}.bin",
            std::process::id()
        ));
        std::fs::write(&tmp, b"hello").unwrap();
        let h = ModelLoader::sha256_of(&tmp).unwrap();
        // SHA-256("hello") = 2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824
        assert_eq!(
            hex_encode(&h),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn hex_encode_correctness() {
        let mut bytes = [0u8; 32];
        bytes[0] = 0xab;
        bytes[1] = 0xcd;
        bytes[31] = 0xef;
        let s = hex_encode(&bytes);
        assert_eq!(s.len(), 64);
        assert!(s.starts_with("abcd"));
        assert!(s.ends_with("ef"));
    }

    #[test]
    fn load_returns_bundle_when_model_dir_missing_on_dev() {
        // On the dev machine /opt/models/foo/0.0.1 doesn't exist —
        // the loader warns + falls back to StubEngine.
        let bundle = ModelLoader::load("nonexistent_model", "0.0.1").unwrap();
        assert_eq!(bundle.engine_name(), "stub");
    }

    #[test]
    fn bundle_estimated_memory_via_engine() {
        let bundle = ModelLoader::load("nonexistent_model", "0.0.1").unwrap();
        assert_eq!(bundle.estimated_memory_mb(), 32);
    }
}
