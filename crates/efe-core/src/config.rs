//! Runtime configuration loader.
//!
//! Configuration is assembled hierarchically:
//!
//! 1. **Defaults** — hard-coded in [`Config::defaults`].
//! 2. **TOML file** — path discovered via `$EFE_CONFIG` → `./efe.toml`
//!    → `/etc/efe/config.toml`. Each present key overrides the default.
//! 3. **Environment variables** — keys prefixed with `EFE__` and using
//!    `__` as the section separator. Example:
//!    `EFE__FUSION__MAX_ROUNDS=10`.
//!
//! The final [`Config`] is validated by [`Config::validate`].

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::errors::{EfeError, Result};
use crate::types::{ConvergenceThresholds, ResourceBudget};

// =====================================================================
// Config sections
// =====================================================================

/// Runtime parameters (worker pool, memory).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct RuntimeConfig {
    /// Number of tokio worker threads.
    pub workers: u32,
    /// HBM pool size in GiB.
    pub hbm_pool_size_gb: u32,
    /// CUDA stream pool size.
    pub stream_pool_size: u32,
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            workers: 4,
            hbm_pool_size_gb: 8,
            stream_pool_size: 4,
        }
    }
}

/// Latent space dimensions.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct LatentConfig {
    /// Default latent dimension.
    pub dim: u32,
    /// Numeric precision (`bf16`, `fp16`, `fp32`).
    pub precision: String,
}

impl Default for LatentConfig {
    fn default() -> Self {
        Self {
            dim: 2048,
            precision: "bf16".into(),
        }
    }
}

/// Fusion parameters.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct FusionConfig {
    /// Strategy name: `dense` or `hierarchical`.
    pub strategy: String,
    /// Max rounds.
    pub max_rounds: u32,
    /// Attention heads.
    pub attention_heads: u32,
    /// LoRA rank for template projections.
    pub lora_rank: u32,
}

impl Default for FusionConfig {
    fn default() -> Self {
        Self {
            strategy: "dense".into(),
            max_rounds: 8,
            attention_heads: 8,
            lora_rank: 16,
        }
    }
}

/// Alignment configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct AlignmentConfig {
    /// Path to the constitution safetensors.
    pub constitution_path: String,
    /// Max projection distance.
    pub max_delta: f32,
    /// Projection strength in `[0, 1]`.
    pub projection_strength: f32,
}

impl Default for AlignmentConfig {
    fn default() -> Self {
        Self {
            constitution_path: "/etc/efe/constitution_mvp_v1.sig.sf".into(),
            max_delta: 0.15,
            projection_strength: 0.05,
        }
    }
}

/// Telemetry configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct TelemetryConfig {
    /// OTLP gRPC endpoint.
    pub otel_endpoint: String,
    /// Prometheus scrape port.
    pub prom_port: u16,
    /// Log level filter.
    pub log_level: String,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        Self {
            otel_endpoint: "http://otel-collector:4317".into(),
            prom_port: 9090,
            log_level: "info,efe=debug".into(),
        }
    }
}

/// gRPC server configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct ServerConfig {
    /// Bind address (`host:port`).
    pub bind: String,
    /// Request timeout in ms.
    pub request_timeout_ms: u32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "0.0.0.0:50051".into(),
            request_timeout_ms: 30_000,
        }
    }
}

// =====================================================================
// Top-level Config
// =====================================================================

/// Top-level configuration loaded at server startup.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct Config {
    /// Runtime parameters.
    pub runtime: RuntimeConfig,
    /// Latent space parameters.
    pub latent: LatentConfig,
    /// Fusion parameters.
    pub fusion: FusionConfig,
    /// Consensus thresholds.
    pub consensus: ConvergenceThresholds,
    /// Alignment configuration.
    pub alignment: AlignmentConfig,
    /// Default resource budget.
    pub budget: ResourceBudget,
    /// Telemetry configuration.
    pub telemetry: TelemetryConfig,
    /// Server configuration.
    pub server: ServerConfig,
}

impl Config {
    /// Built-in defaults.
    #[must_use]
    pub fn defaults() -> Self {
        Self::default()
    }

    /// Load configuration using the standard discovery chain.
    ///
    /// # Errors
    ///
    /// Returns [`EfeError::Io`] or [`EfeError::InvalidConfig`] when the
    /// discovered TOML cannot be read / parsed, or when validation fails.
    pub async fn load() -> Result<Self> {
        let mut cfg = Self::defaults();
        if let Some(path) = Self::find_config_file() {
            tracing::info!(path = %path.display(), "loading config file");
            cfg = cfg.merge_toml_path(&path).await?;
        }
        cfg.apply_env();
        cfg.validate()?;
        Ok(cfg)
    }

    /// Load configuration from a specific TOML path (no env merging).
    ///
    /// # Errors
    ///
    /// Returns [`EfeError::Io`] or [`EfeError::InvalidConfig`].
    pub async fn from_toml_path(path: &Path) -> Result<Self> {
        let cfg = Self::defaults().merge_toml_path(path).await?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// Parse a configuration from a TOML string.
    ///
    /// # Errors
    ///
    /// Returns [`EfeError::InvalidConfig`] on parse failure.
    pub fn from_toml_str(s: &str) -> Result<Self> {
        let cfg: Self = toml::from_str(s)?;
        cfg.validate()?;
        Ok(cfg)
    }

    async fn merge_toml_path(self, path: &Path) -> Result<Self> {
        let contents = tokio::fs::read_to_string(path).await?;
        let parsed: Self = toml::from_str(&contents)?;
        // Full replacement (not additive). serde(default) ensures missing
        // fields take the defaults. This is simpler and predictable.
        let _ = self;
        Ok(parsed)
    }

    /// Apply environment overrides (`EFE__<SECTION>__<FIELD>`).
    ///
    /// Unknown keys are silently ignored to avoid log spam.
    pub fn apply_env(&mut self) {
        for (k, v) in std::env::vars() {
            let Some(rest) = k.strip_prefix("EFE__") else {
                continue;
            };
            let path: Vec<&str> = rest.split("__").collect();
            self.apply_env_override(&path, &v);
        }
    }

    #[allow(clippy::too_many_lines)]
    fn apply_env_override(&mut self, path: &[&str], value: &str) {
        // Minimal whitelist of env-overridable fields. Each entry is
        // strictly typed via parse().
        match path {
            ["RUNTIME", "WORKERS"] => set(&mut self.runtime.workers, value),
            ["RUNTIME", "HBM_POOL_SIZE_GB"] => set(&mut self.runtime.hbm_pool_size_gb, value),
            ["RUNTIME", "STREAM_POOL_SIZE"] => set(&mut self.runtime.stream_pool_size, value),

            ["LATENT", "DIM"] => set(&mut self.latent.dim, value),
            ["LATENT", "PRECISION"] => self.latent.precision = value.to_string(),

            ["FUSION", "MAX_ROUNDS"] => set(&mut self.fusion.max_rounds, value),
            ["FUSION", "ATTENTION_HEADS"] => set(&mut self.fusion.attention_heads, value),
            ["FUSION", "LORA_RANK"] => set(&mut self.fusion.lora_rank, value),
            ["FUSION", "STRATEGY"] => self.fusion.strategy = value.to_string(),

            ["ALIGNMENT", "MAX_DELTA"] => set(&mut self.alignment.max_delta, value),
            ["ALIGNMENT", "PROJECTION_STRENGTH"] => {
                set(&mut self.alignment.projection_strength, value);
            }

            ["BUDGET", "MAX_WALL_CLOCK_MS"] => set(&mut self.budget.max_wall_clock_ms, value),
            ["BUDGET", "MAX_ENERGY_WH"] => set(&mut self.budget.max_energy_wh, value),

            ["TELEMETRY", "OTEL_ENDPOINT"] => {
                self.telemetry.otel_endpoint = value.to_string();
            }
            ["TELEMETRY", "PROM_PORT"] => set(&mut self.telemetry.prom_port, value),
            ["TELEMETRY", "LOG_LEVEL"] => self.telemetry.log_level = value.to_string(),

            ["SERVER", "BIND"] => self.server.bind = value.to_string(),
            ["SERVER", "REQUEST_TIMEOUT_MS"] => {
                set(&mut self.server.request_timeout_ms, value);
            }

            _ => {
                // Silently ignore unknown keys — `EFE__FOO=bar` is not a
                // hard error, to allow safely sharing env files.
            }
        }
    }

    /// Discover the config file path.
    ///
    /// Precedence: `$EFE_CONFIG` > `./efe.toml` > `/etc/efe/config.toml`.
    #[must_use]
    pub fn find_config_file() -> Option<PathBuf> {
        if let Ok(v) = std::env::var("EFE_CONFIG") {
            let p = PathBuf::from(v);
            if p.exists() {
                return Some(p);
            }
        }
        for candidate in ["efe.toml", "/etc/efe/config.toml"] {
            let p = PathBuf::from(candidate);
            if p.exists() {
                return Some(p);
            }
        }
        None
    }

    /// Validate cross-field invariants.
    ///
    /// # Errors
    ///
    /// Returns [`EfeError::InvalidConfig`] on inconsistency.
    pub fn validate(&self) -> Result<()> {
        if self.runtime.workers == 0 {
            return Err(EfeError::InvalidConfig(
                "runtime.workers must be >= 1".into(),
            ));
        }
        if self.fusion.max_rounds == 0 {
            return Err(EfeError::InvalidConfig(
                "fusion.max_rounds must be >= 1".into(),
            ));
        }
        if !(self.alignment.projection_strength >= 0.0 && self.alignment.projection_strength <= 1.0)
        {
            return Err(EfeError::InvalidConfig(
                "alignment.projection_strength must be in [0,1]".into(),
            ));
        }
        if self.alignment.max_delta <= 0.0 || !self.alignment.max_delta.is_finite() {
            return Err(EfeError::InvalidConfig(
                "alignment.max_delta must be positive and finite".into(),
            ));
        }
        if self.budget.max_wall_clock_ms == 0 {
            return Err(EfeError::InvalidConfig(
                "budget.max_wall_clock_ms must be > 0".into(),
            ));
        }
        if !matches!(self.fusion.strategy.as_str(), "dense" | "hierarchical") {
            return Err(EfeError::InvalidConfig(format!(
                "unknown fusion.strategy '{}'",
                self.fusion.strategy
            )));
        }
        if !matches!(self.latent.precision.as_str(), "bf16" | "fp16" | "fp32") {
            return Err(EfeError::InvalidConfig(format!(
                "unknown latent.precision '{}'",
                self.latent.precision
            )));
        }
        Ok(())
    }
}

fn set<T: std::str::FromStr>(target: &mut T, raw: &str) {
    if let Ok(v) = raw.parse::<T>() {
        *target = v;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_validate() {
        Config::defaults()
            .validate()
            .expect("defaults must validate");
    }

    #[test]
    fn rejects_zero_workers() {
        let mut c = Config::defaults();
        c.runtime.workers = 0;
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_zero_max_rounds() {
        let mut c = Config::defaults();
        c.fusion.max_rounds = 0;
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_bad_strategy() {
        let mut c = Config::defaults();
        c.fusion.strategy = "exotic".into();
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_bad_precision() {
        let mut c = Config::defaults();
        c.latent.precision = "int8".into();
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_projection_strength_out_of_range() {
        let mut c = Config::defaults();
        c.alignment.projection_strength = 1.5;
        assert!(c.validate().is_err());
        c.alignment.projection_strength = -0.1;
        assert!(c.validate().is_err());
    }

    #[test]
    fn rejects_nan_max_delta() {
        let mut c = Config::defaults();
        c.alignment.max_delta = f32::NAN;
        assert!(c.validate().is_err());
    }

    #[test]
    fn from_toml_str_parses_partial_config() {
        let s = r"
[fusion]
max_rounds = 12
";
        let c = Config::from_toml_str(s).expect("valid partial");
        assert_eq!(c.fusion.max_rounds, 12);
        // Unset fields fall back to defaults.
        assert_eq!(c.runtime.workers, RuntimeConfig::default().workers);
    }

    #[test]
    fn from_toml_str_rejects_invalid_partial() {
        let s = r"
[fusion]
max_rounds = 0
";
        assert!(Config::from_toml_str(s).is_err());
    }

    #[test]
    fn apply_env_override_updates_value() {
        let mut c = Config::defaults();
        c.apply_env_override(&["FUSION", "MAX_ROUNDS"], "15");
        assert_eq!(c.fusion.max_rounds, 15);
        c.apply_env_override(&["SERVER", "BIND"], "127.0.0.1:7000");
        assert_eq!(c.server.bind, "127.0.0.1:7000");
    }

    #[test]
    fn apply_env_override_ignores_unknown() {
        let mut c = Config::defaults();
        c.apply_env_override(&["UNKNOWN", "KEY"], "value");
        // No panic, no change.
        assert_eq!(c.runtime.workers, RuntimeConfig::default().workers);
    }

    #[tokio::test]
    async fn from_toml_path_works() {
        let tmp = tempfile::NamedTempFile::new().expect("tmpfile");
        std::fs::write(
            tmp.path(),
            r"
[runtime]
workers = 8

[fusion]
max_rounds = 10
",
        )
        .expect("write");
        let c = Config::from_toml_path(tmp.path())
            .await
            .expect("valid TOML");
        assert_eq!(c.runtime.workers, 8);
        assert_eq!(c.fusion.max_rounds, 10);
    }
}
