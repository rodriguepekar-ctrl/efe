//! PR #20 § 18 — workload profile YAML loader (schema `version: 1`).
//!
//! STABILITY_CONTRACT v1.2 § 9.10.4 commits the schema for ≥ 6 chart-
//! minor versions.  Future schema bumps : `version: 2` + migration
//! tooling in the `compare` subcommand.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cli::RunMode;
use crate::config::{PatternSpec, TlsConfig, WorkloadMix};
use crate::error::{LoadgenError, LoadgenResult};

/// Top-level profile structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    /// Schema version.  Currently only `1` is accepted.
    pub version: u32,

    /// Human-readable identification.
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,

    /// Target endpoints + connection settings.
    pub target: ProfileTarget,

    /// Traffic pattern.
    #[serde(default)]
    pub pattern: Option<PatternSpec>,

    /// Workload shape.
    #[serde(default)]
    pub workload: Option<ProfileWorkload>,

    /// Rate control.
    #[serde(default)]
    pub rate: Option<ProfileRate>,

    /// Termination policies.
    #[serde(default)]
    pub termination: Option<ProfileTermination>,

    /// Output config.
    #[serde(default)]
    pub output: Option<ProfileOutput>,

    /// Observability.
    #[serde(default)]
    pub observability: Option<ProfileObservability>,

    /// Run-level metadata.
    #[serde(default)]
    pub run: Option<ProfileRun>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileTarget {
    pub endpoints: Vec<ProfileEndpoint>,
    #[serde(default = "default_target_role")]
    pub role: String,
    #[serde(default)]
    pub conns_per_target: Option<u32>,
    #[serde(default, with = "humantime_serde_opt")]
    pub per_request_deadline: Option<std::time::Duration>,
    #[serde(default)]
    pub mtls: Option<ProfileMtls>,
}

fn default_target_role() -> String {
    "controller".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileEndpoint {
    pub url: String,
    #[serde(default = "default_weight")]
    pub weight: f64,
}

fn default_weight() -> f64 {
    1.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileMtls {
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
    pub ca_path: PathBuf,
    #[serde(default)]
    pub expected_san: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileWorkload {
    /// `iterate`, `step`, `reset`, `state`, or `mixed`.
    #[serde(rename = "type")]
    pub type_: String,
    /// Mixed weights — only meaningful when `type_ = "mixed"`.
    #[serde(default)]
    pub mix: Option<ProfileWorkloadMix>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileWorkloadMix {
    #[serde(default)]
    pub iterate: f64,
    #[serde(default)]
    pub step: f64,
    #[serde(default)]
    pub reset: f64,
    #[serde(default)]
    pub state: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileRate {
    pub mode: String,
    #[serde(default)]
    pub max_concurrency: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileTermination {
    #[serde(default, with = "humantime_serde_opt")]
    pub duration: Option<std::time::Duration>,
    #[serde(default)]
    pub requests: Option<u64>,
    #[serde(default)]
    pub until_slo: Option<std::collections::HashMap<String, f64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileOutput {
    pub format: String,
    #[serde(default)]
    pub file: Option<PathBuf>,
    #[serde(default)]
    pub record: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileObservability {
    #[serde(default)]
    pub prometheus: Option<ProfileProm>,
    #[serde(default)]
    pub otel: Option<ProfileOtel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileProm {
    pub listen: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileOtel {
    pub endpoint: String,
    #[serde(default = "default_sampler_arg")]
    pub sampler_arg: f64,
    #[serde(default = "default_service_name")]
    pub service_name: String,
}

fn default_sampler_arg() -> f64 {
    1.0
}

fn default_service_name() -> String {
    "efe-meta-swarm-loadgen".into()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileRun {
    #[serde(default)]
    pub seed: Option<u64>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub allow_mutation: bool,
    #[serde(default)]
    pub chaos_marker: Option<String>,
}

// ────────────────────────────────────────────────────────────────
//                             Loader
// ────────────────────────────────────────────────────────────────

impl Profile {
    /// Load + validate a profile YAML from disk.
    pub fn load(path: &Path) -> LoadgenResult<Self> {
        let raw = fs::read_to_string(path).map_err(|source| LoadgenError::Output {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_str(&raw, path)
    }

    /// Parse from string content.
    pub fn from_str(raw: &str, source_hint: &Path) -> LoadgenResult<Self> {
        let p: Self = serde_yaml::from_str(raw).map_err(|source| LoadgenError::ProfileLoad {
            path: source_hint.to_path_buf(),
            source,
        })?;
        if p.version != crate::PROFILE_SCHEMA_VERSION {
            return Err(LoadgenError::ProfileVersion {
                path: source_hint.to_path_buf(),
                expected: crate::PROFILE_SCHEMA_VERSION,
                actual: p.version,
            });
        }
        if p.target.endpoints.is_empty() {
            return Err(LoadgenError::InvalidConfig(format!(
                "profile {source} has no target endpoints",
                source = source_hint.display()
            )));
        }
        Ok(p)
    }

    /// Convert the profile's workload + mix into a [`WorkloadMix`].
    /// Returns `None` when the profile doesn't carry workload info ;
    /// the engine then falls back to CLI defaults.
    #[must_use]
    pub fn workload_mix(&self) -> Option<WorkloadMix> {
        let w = self.workload.as_ref()?;
        Some(match w.type_.as_str() {
            "mixed" => {
                let m = w.mix.clone().unwrap_or_default();
                WorkloadMix {
                    iterate: m.iterate,
                    step: m.step,
                    reset: m.reset,
                    state: m.state,
                }
            }
            "iterate" => WorkloadMix { iterate: 1.0, ..Default::default() },
            "step" => WorkloadMix { step: 1.0, ..Default::default() },
            "reset" => WorkloadMix { reset: 1.0, ..Default::default() },
            "state" => WorkloadMix { state: 1.0, ..Default::default() },
            _ => WorkloadMix { iterate: 1.0, ..Default::default() },
        })
    }

    /// Resolve the profile's TLS config (if any).
    #[must_use]
    pub fn tls(&self) -> Option<TlsConfig> {
        self.target.mtls.as_ref().map(|m| TlsConfig {
            cert_path: m.cert_path.clone(),
            key_path: m.key_path.clone(),
            ca_path: m.ca_path.clone(),
            expected_san: m.expected_san.clone(),
            insecure: false,
        })
    }

    /// Run mode from the profile (defaults to dev).
    #[must_use]
    pub fn run_mode(&self) -> RunMode {
        self.run
            .as_ref()
            .and_then(|r| r.mode.as_deref())
            .map(|m| match m {
                "production" => RunMode::Production,
                "staging" => RunMode::Staging,
                _ => RunMode::Dev,
            })
            .unwrap_or(RunMode::Dev)
    }
}

// ────────────────────────────────────────────────────────────────
//                  humantime serde adapter for Option
// ────────────────────────────────────────────────────────────────

mod humantime_serde_opt {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(d: &Option<Duration>, s: S) -> Result<S::Ok, S::Error> {
        match d {
            Some(dur) => s.serialize_str(&humantime::format_duration(*dur).to_string()),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
        let raw: Option<String> = Option::deserialize(d)?;
        match raw {
            None => Ok(None),
            Some(s) => humantime::parse_duration(&s)
                .map(Some)
                .map_err(serde::de::Error::custom),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_PROFILE: &str = r#"
version: 1
name: "test-profile"
description: "loadgen test profile"
target:
  endpoints:
    - url: "grpc://localhost:50051"
      weight: 1.0
  role: controller
  conns_per_target: 4
  per_request_deadline: 3s
workload:
  type: mixed
  mix:
    iterate: 0.95
    state: 0.05
rate:
  mode: hybrid
  max_concurrency: 100
termination:
  duration: 5m
output:
  format: ndjson
run:
  seed: 42
  mode: staging
"#;

    #[test]
    fn parse_valid_profile() {
        let p = Profile::from_str(VALID_PROFILE, std::path::Path::new("inline")).unwrap();
        assert_eq!(p.version, 1);
        assert_eq!(p.name, "test-profile");
        assert_eq!(p.target.endpoints.len(), 1);
    }

    #[test]
    fn reject_wrong_version() {
        let p = r#"version: 2
name: bad
target: {endpoints: [{url: "grpc://x:50051"}]}"#;
        let err = Profile::from_str(p, std::path::Path::new("inline")).unwrap_err();
        assert!(matches!(err, LoadgenError::ProfileVersion { .. }));
    }

    #[test]
    fn reject_empty_endpoints() {
        let p = r#"version: 1
name: bad
target: {endpoints: []}"#;
        let err = Profile::from_str(p, std::path::Path::new("inline")).unwrap_err();
        assert!(matches!(err, LoadgenError::InvalidConfig(_)));
    }

    #[test]
    fn workload_mix_resolves_mixed() {
        let p = Profile::from_str(VALID_PROFILE, std::path::Path::new("inline")).unwrap();
        let mix = p.workload_mix().unwrap();
        assert!((mix.iterate - 0.95).abs() < 1e-9);
        assert!((mix.state - 0.05).abs() < 1e-9);
    }

    #[test]
    fn run_mode_staging_detected() {
        let p = Profile::from_str(VALID_PROFILE, std::path::Path::new("inline")).unwrap();
        assert_eq!(p.run_mode(), RunMode::Staging);
    }

    #[test]
    fn missing_run_section_defaults_to_dev() {
        let p = r#"version: 1
name: x
target: {endpoints: [{url: "grpc://x:50051"}]}"#;
        let pr = Profile::from_str(p, std::path::Path::new("inline")).unwrap();
        assert_eq!(pr.run_mode(), RunMode::Dev);
    }
}
