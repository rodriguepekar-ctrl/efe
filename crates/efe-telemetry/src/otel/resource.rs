//! Sprint **D13-bis** — OpenTelemetry Resource builder.
//!
//! Cf. `docs/bench/d13bis_otel_sdk_real_architecture.md` § 4.
//!
//! The OTel Resource carries a set of key-value attributes that
//! identify the *source* of the spans. Tempo / Jaeger / Honeycomb
//! use these for filtering ("show me only spans from the prod
//! cluster, version 0.1.17"). Without a resource, dashboards
//! group everything under "<unknown>".
//!
//! We follow the OpenTelemetry **semantic conventions** v1.27 :
//!
//!   * `service.name` — required ; default `efe-server`.
//!   * `service.version` — pinned to `CARGO_PKG_VERSION` at build.
//!   * `service.namespace` — constant `"efe"`.
//!   * `service.instance.id` — K8s pod name (env `HOSTNAME`).
//!   * `deployment.environment.name` — operator-set env
//!     (`EFE_DEPLOY_ENV`).
//!   * `host.name` / `host.arch` / `os.type` — host shape.
//!   * `process.pid` — process identity.
//!   * `efe.git_sha` — compile-time `GIT_SHA` env (custom).
//!   * `efe.deploy_tier` — operator-set `EFE_DEPLOY_TIER` (custom).
//!
//! # Cardinality impact (Tempo backend)
//!
//! Resource attrs sit on the *resource* (one per pod), not on
//! every span — they don't multiply Tempo cardinality the way
//! span attributes do. ~1 k pods × ~9 attrs ≈ 9 k attribute-value
//! cells, trivial for any production Tempo deployment.

use opentelemetry::KeyValue;
use opentelemetry_sdk::Resource;

use super::OtelConfig;

// Semantic-convention attribute keys (string literals — robust
// across `opentelemetry-semantic-conventions` versions ; v0.27
// changed several constant names. Cf. arch § 4 for the canonical
// list.). Lower-case dotted names match the OTel spec.
const SERVICE_NAME: &str = "service.name";
const SERVICE_VERSION: &str = "service.version";
const SERVICE_NAMESPACE: &str = "service.namespace";
const SERVICE_INSTANCE_ID: &str = "service.instance.id";
const HOST_NAME: &str = "host.name";
const HOST_ARCH: &str = "host.arch";
const OS_TYPE: &str = "os.type";
const PROCESS_PID: &str = "process.pid";
const DEPLOYMENT_ENVIRONMENT_NAME: &str = "deployment.environment.name";

/// Build the OTel `Resource` for an `efe-server` process.
///
/// The function is **infallible** — every attribute either has a
/// concrete value or is omitted. Operator misconfiguration produces
/// a resource with fewer attrs but never a fatal error.
#[must_use]
pub fn build_resource(cfg: &OtelConfig) -> Resource {
    let mut attrs: Vec<KeyValue> = Vec::with_capacity(12);

    // --- Service identity (REQUIRED by OTel spec) ---
    attrs.push(KeyValue::new(SERVICE_NAME, cfg.service_name.clone()));
    attrs.push(KeyValue::new(SERVICE_VERSION, cfg.service_version.clone()));
    attrs.push(KeyValue::new(SERVICE_NAMESPACE, "efe"));

    // --- Service instance ---
    if let Ok(hostname) = std::env::var("HOSTNAME") {
        if !hostname.is_empty() {
            attrs.push(KeyValue::new(SERVICE_INSTANCE_ID, hostname.clone()));
            attrs.push(KeyValue::new(HOST_NAME, hostname));
        }
    }

    // --- Host / OS shape (statically resolved at build) ---
    attrs.push(KeyValue::new(HOST_ARCH, std::env::consts::ARCH));
    attrs.push(KeyValue::new(OS_TYPE, std::env::consts::OS));
    attrs.push(KeyValue::new(PROCESS_PID, std::process::id() as i64));

    // --- Deployment context (operator-set) ---
    let deploy_env = std::env::var("EFE_DEPLOY_ENV")
        .unwrap_or_else(|_| "unknown".to_string());
    attrs.push(KeyValue::new(DEPLOYMENT_ENVIRONMENT_NAME, deploy_env));

    // --- EFE-specific attributes (no semantic-convention key,
    // namespaced under `efe.*`) ---
    if let Some(git) = option_env!("GIT_SHA") {
        if !git.is_empty() {
            attrs.push(KeyValue::new("efe.git_sha", git));
        }
    }
    if let Ok(tier) = std::env::var("EFE_DEPLOY_TIER") {
        if !tier.is_empty() {
            attrs.push(KeyValue::new("efe.deploy_tier", tier));
        }
    }

    Resource::new(attrs)
}

// =====================================================================
// Tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Tests that mutate process env vars (HOSTNAME / EFE_DEPLOY_ENV
    /// / EFE_DEPLOY_TIER) must run serially, otherwise a parallel
    /// thread can race between our `set_var` and `assert`. The
    /// Rust test runner does not natively support `#[serial]` ; we
    /// gate via a process-wide Mutex.
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    fn cfg(name: &str, version: &str) -> OtelConfig {
        OtelConfig {
            enabled: true,
            endpoint: "http://localhost:4317".into(),
            sample_rate: 0.05,
            force_sample_on_error: true,
            service_name: name.into(),
            service_version: version.into(),
        }
    }

    /// Helper : extract the value of `key` from a Resource as a
    /// String, if present and stringable.
    fn get_attr(r: &Resource, key: &str) -> Option<String> {
        r.iter()
            .find(|(k, _)| k.as_str() == key)
            .map(|(_, v)| v.to_string())
    }

    #[test]
    fn includes_service_name_and_version() {
        let r = build_resource(&cfg("test-svc", "1.2.3"));
        assert_eq!(get_attr(&r, "service.name").as_deref(), Some("test-svc"));
        assert_eq!(get_attr(&r, "service.version").as_deref(), Some("1.2.3"));
    }

    #[test]
    fn always_includes_service_namespace() {
        let r = build_resource(&cfg("a", "b"));
        assert_eq!(get_attr(&r, "service.namespace").as_deref(), Some("efe"));
    }

    #[test]
    fn includes_hostname_when_env_set() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("HOSTNAME", "test-pod-xyz");
        let r = build_resource(&cfg("a", "b"));
        assert_eq!(
            get_attr(&r, "service.instance.id").as_deref(),
            Some("test-pod-xyz")
        );
        assert_eq!(get_attr(&r, "host.name").as_deref(), Some("test-pod-xyz"));
        std::env::remove_var("HOSTNAME");
    }

    #[test]
    fn includes_host_arch_and_os_type() {
        let r = build_resource(&cfg("a", "b"));
        assert!(get_attr(&r, "host.arch").is_some());
        assert!(get_attr(&r, "os.type").is_some());
    }

    #[test]
    fn includes_process_pid() {
        let r = build_resource(&cfg("a", "b"));
        let pid = get_attr(&r, "process.pid").expect("process.pid present");
        assert!(pid.parse::<u32>().is_ok(), "process.pid should be numeric");
    }

    #[test]
    fn deploy_env_defaults_to_unknown() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        std::env::remove_var("EFE_DEPLOY_ENV");
        let r = build_resource(&cfg("a", "b"));
        assert_eq!(
            get_attr(&r, "deployment.environment.name").as_deref(),
            Some("unknown")
        );
    }

    #[test]
    fn deploy_env_respects_env_var() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("EFE_DEPLOY_ENV", "production");
        let r = build_resource(&cfg("a", "b"));
        assert_eq!(
            get_attr(&r, "deployment.environment.name").as_deref(),
            Some("production")
        );
        std::env::remove_var("EFE_DEPLOY_ENV");
    }

    #[test]
    fn efe_deploy_tier_attached_when_env_set() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("EFE_DEPLOY_TIER", "tier2");
        let r = build_resource(&cfg("a", "b"));
        assert_eq!(get_attr(&r, "efe.deploy_tier").as_deref(), Some("tier2"));
        std::env::remove_var("EFE_DEPLOY_TIER");
    }

    #[test]
    fn empty_hostname_is_skipped() {
        let _g = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        std::env::set_var("HOSTNAME", "");
        let r = build_resource(&cfg("a", "b"));
        assert!(
            get_attr(&r, "service.instance.id").is_none(),
            "empty HOSTNAME should not produce service.instance.id"
        );
        std::env::remove_var("HOSTNAME");
    }
}
