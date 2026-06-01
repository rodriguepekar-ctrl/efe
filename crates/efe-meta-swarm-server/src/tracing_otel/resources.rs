//! PR #18 § 15 — OTel resource attribute detection.
//!
//! Builds the [`opentelemetry_sdk::Resource`] attached to every span
//! exported by this process.  Resource attributes are the
//! **immutable identity** of the emitting service ; they're emitted
//! once per export batch and copied into every span on the consuming
//! side.
//!
//! Source order (later overrides earlier) :
//!
//!   1. Hard-coded defaults (`telemetry.sdk.*`).
//!   2. Auto-detected from the host (`host.arch`, `host.os.type`,
//!      `process.pid`).
//!   3. Auto-detected from K8s downward API (`k8s.pod.name`,
//!      `k8s.namespace.name`) via env vars set by the Helm chart.
//!   4. OTel-spec env vars (`OTEL_SERVICE_NAME`,
//!      `OTEL_DEPLOYMENT_ENVIRONMENT`, `OTEL_RESOURCE_ATTRIBUTES`).
//!
//! All attribute keys conform to OpenTelemetry semantic conventions
//! v1.27.

use opentelemetry::KeyValue;
use opentelemetry_sdk::Resource;
use opentelemetry_semantic_conventions::resource as semconv;
use tracing::debug;

use super::OtelInitError;

/// Discriminator passed to [`detect_resource`] so the helper can set
/// `service.name` correctly for the controller vs participant binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The controller binary.
    Controller,
    /// One participant binary instance.
    Participant,
}

impl Role {
    /// `service.name` resource attribute value when `OTEL_SERVICE_NAME`
    /// is unset.
    #[must_use]
    pub const fn default_service_name(self) -> &'static str {
        match self {
            Self::Controller => "efe-meta-swarm-controller",
            Self::Participant => "efe-meta-swarm-participant",
        }
    }

    /// Lowercase tag used for the `efe.meta_swarm.role` custom
    /// resource attribute.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Controller => "controller",
            Self::Participant => "participant",
        }
    }
}

/// Auto-detect the process resource attributes.
///
/// Returns an [`opentelemetry_sdk::Resource`] ready to attach to the
/// `TracerProvider`.  Never fails on auto-detection — missing env vars
/// are silently skipped — but errors when an explicit
/// `OTEL_RESOURCE_ATTRIBUTES` value fails to parse (caller fail-fast).
pub fn detect_resource(role: Role) -> Result<Resource, OtelInitError> {
    let mut kvs: Vec<KeyValue> = Vec::with_capacity(16);

    // ── 1. service.name + service.version + service.instance.id ──
    let service_name = std::env::var(super::ENV_SERVICE_NAME)
        .unwrap_or_else(|_| role.default_service_name().to_string());
    kvs.push(KeyValue::new(semconv::SERVICE_NAME, service_name));
    kvs.push(KeyValue::new(semconv::SERVICE_VERSION, crate::VERSION));

    let instance_id = std::env::var("HOSTNAME").unwrap_or_else(|_| {
        // Fallback : deterministic UUID-like identifier for dev/unit-test.
        // Not a real UUID — just a stable string for the lifetime of the
        // process.  AD22 says "falls back to a random UUID" ; we use a
        // process-pid-based string here to avoid pulling in the `uuid`
        // crate.
        format!("efe-dev-{}", std::process::id())
    });
    kvs.push(KeyValue::new(semconv::SERVICE_INSTANCE_ID, instance_id));

    // ── 2. deployment.environment ──
    if let Ok(env) = std::env::var(super::ENV_DEPLOYMENT_ENVIRONMENT) {
        kvs.push(KeyValue::new(semconv::DEPLOYMENT_ENVIRONMENT_NAME, env));
    }

    // ── 3. host + process auto-detection ──
    kvs.push(KeyValue::new(semconv::HOST_ARCH, host_arch()));
    kvs.push(KeyValue::new(semconv::OS_TYPE, host_os_type()));
    kvs.push(KeyValue::new(
        semconv::PROCESS_PID,
        i64::from(std::process::id().min(i32::MAX as u32)),
    ));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(name) = exe.file_name().and_then(|n| n.to_str()) {
            kvs.push(KeyValue::new(
                semconv::PROCESS_EXECUTABLE_NAME,
                name.to_string(),
            ));
        }
    }

    // ── 4. K8s downward API (when running in K8s) ──
    if let Ok(ns) = std::env::var("EFE_KUBERNETES_NAMESPACE") {
        kvs.push(KeyValue::new(semconv::K8S_NAMESPACE_NAME, ns));
    } else if let Ok(ns) = std::env::var("POD_NAMESPACE") {
        kvs.push(KeyValue::new(semconv::K8S_NAMESPACE_NAME, ns));
    }
    if let Ok(pod) = std::env::var("EFE_KUBERNETES_POD_NAME") {
        kvs.push(KeyValue::new(semconv::K8S_POD_NAME, pod));
    } else if let Ok(pod) = std::env::var("POD_NAME") {
        kvs.push(KeyValue::new(semconv::K8S_POD_NAME, pod));
    }
    if let Ok(deploy) = std::env::var("EFE_KUBERNETES_DEPLOYMENT_NAME") {
        kvs.push(KeyValue::new("k8s.deployment.name", deploy));
    }
    if let Ok(node) = std::env::var("EFE_KUBERNETES_NODE_NAME") {
        kvs.push(KeyValue::new(semconv::K8S_NODE_NAME, node));
    }

    // ── 5. EFE-specific custom attribute (role) ──
    kvs.push(KeyValue::new("efe.meta_swarm.role", role.as_str()));

    // ── 6. OTEL_RESOURCE_ATTRIBUTES (sep can be `,` or `;` per OTel-spec) ──
    if let Ok(raw) = std::env::var(super::ENV_RESOURCE_ATTRIBUTES) {
        for entry in raw.split(|c| c == ',' || c == ';') {
            let entry = entry.trim();
            if entry.is_empty() {
                continue;
            }
            let Some((k, v)) = entry.split_once('=') else {
                return Err(OtelInitError::InvalidResourceAttributes(format!(
                    "malformed entry {entry:?} ; expected `k=v`",
                )));
            };
            let k = k.trim().to_string();
            let v = v.trim().to_string();
            if k.is_empty() {
                return Err(OtelInitError::InvalidResourceAttributes(format!(
                    "empty key in entry {entry:?}",
                )));
            }
            kvs.push(KeyValue::new(k, v));
        }
    }

    debug!(count = kvs.len(), role = ?role, "OTel resource detected");

    Ok(Resource::new(kvs))
}

// ─────────────────────────────────────────────────────────────────────
// Helpers.
// ─────────────────────────────────────────────────────────────────────

const fn host_arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64"
    } else if cfg!(target_arch = "x86_64") {
        "amd64"
    } else if cfg!(target_arch = "x86") {
        "x86"
    } else {
        "unknown"
    }
}

const fn host_os_type() -> &'static str {
    if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "freebsd") {
        "freebsd"
    } else {
        "unknown"
    }
}

// ─────────────────────────────────────────────────────────────────────
// Tests.
// ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Use a process-wide lock so env-var-touching tests don't race.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn clear_env() {
        for v in [
            super::super::ENV_SERVICE_NAME,
            super::super::ENV_DEPLOYMENT_ENVIRONMENT,
            super::super::ENV_RESOURCE_ATTRIBUTES,
            "EFE_KUBERNETES_NAMESPACE",
            "EFE_KUBERNETES_POD_NAME",
            "EFE_KUBERNETES_DEPLOYMENT_NAME",
            "EFE_KUBERNETES_NODE_NAME",
            "POD_NAMESPACE",
            "POD_NAME",
        ] {
            std::env::remove_var(v);
        }
    }

    #[test]
    fn role_default_service_name_is_stable() {
        assert_eq!(
            Role::Controller.default_service_name(),
            "efe-meta-swarm-controller"
        );
        assert_eq!(
            Role::Participant.default_service_name(),
            "efe-meta-swarm-participant"
        );
    }

    #[test]
    fn role_as_str_is_stable() {
        assert_eq!(Role::Controller.as_str(), "controller");
        assert_eq!(Role::Participant.as_str(), "participant");
    }

    #[test]
    fn detect_resource_sets_default_service_name() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear_env();
        let r = detect_resource(Role::Controller).expect("ok");
        let sn = r
            .iter()
            .find(|(k, _)| k.as_str() == semconv::SERVICE_NAME)
            .map(|(_, v)| v.to_string());
        assert_eq!(sn.as_deref(), Some("efe-meta-swarm-controller"));
    }

    #[test]
    fn detect_resource_honours_otel_service_name_env() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear_env();
        std::env::set_var(super::super::ENV_SERVICE_NAME, "custom-name");
        let r = detect_resource(Role::Controller).expect("ok");
        let sn = r
            .iter()
            .find(|(k, _)| k.as_str() == semconv::SERVICE_NAME)
            .map(|(_, v)| v.to_string());
        assert_eq!(sn.as_deref(), Some("custom-name"));
        clear_env();
    }

    #[test]
    fn detect_resource_picks_up_k8s_env() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear_env();
        std::env::set_var("EFE_KUBERNETES_NAMESPACE", "efe");
        std::env::set_var("EFE_KUBERNETES_POD_NAME", "test-pod-abc");
        let r = detect_resource(Role::Participant).expect("ok");
        let ns = r
            .iter()
            .find(|(k, _)| k.as_str() == semconv::K8S_NAMESPACE_NAME)
            .map(|(_, v)| v.to_string());
        assert_eq!(ns.as_deref(), Some("efe"));
        let pod = r
            .iter()
            .find(|(k, _)| k.as_str() == semconv::K8S_POD_NAME)
            .map(|(_, v)| v.to_string());
        assert_eq!(pod.as_deref(), Some("test-pod-abc"));
        clear_env();
    }

    #[test]
    fn detect_resource_emits_efe_role_attribute() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear_env();
        let r = detect_resource(Role::Participant).expect("ok");
        let role = r
            .iter()
            .find(|(k, _)| k.as_str() == "efe.meta_swarm.role")
            .map(|(_, v)| v.to_string());
        assert_eq!(role.as_deref(), Some("participant"));
    }

    #[test]
    fn detect_resource_parses_otel_resource_attributes_env() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear_env();
        std::env::set_var(
            super::super::ENV_RESOURCE_ATTRIBUTES,
            "deployment.tier=canary,my.custom.key=42",
        );
        let r = detect_resource(Role::Controller).expect("ok");
        let tier = r
            .iter()
            .find(|(k, _)| k.as_str() == "deployment.tier")
            .map(|(_, v)| v.to_string());
        assert_eq!(tier.as_deref(), Some("canary"));
        let custom = r
            .iter()
            .find(|(k, _)| k.as_str() == "my.custom.key")
            .map(|(_, v)| v.to_string());
        assert_eq!(custom.as_deref(), Some("42"));
        clear_env();
    }

    #[test]
    fn detect_resource_rejects_malformed_resource_attributes() {
        let _g = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        clear_env();
        std::env::set_var(super::super::ENV_RESOURCE_ATTRIBUTES, "no-equals-sign");
        let err = detect_resource(Role::Controller).unwrap_err();
        match err {
            OtelInitError::InvalidResourceAttributes(msg) => {
                assert!(msg.contains("no-equals-sign"));
            }
            other => panic!("expected InvalidResourceAttributes, got {other:?}"),
        }
        clear_env();
    }

    #[test]
    fn host_arch_returns_known_value() {
        let a = host_arch();
        assert!(["arm64", "amd64", "x86", "unknown"].contains(&a));
    }

    #[test]
    fn host_os_type_returns_known_value() {
        let o = host_os_type();
        assert!(["linux", "darwin", "windows", "freebsd", "unknown"].contains(&o));
    }
}
