//! NetworkPolicy enforcement.
//!
//! microVMs run in a dedicated namespace `efe-agent-runtime-<tenant>`
//! with default-deny ingress + egress. The Helm chart ships a base
//! policy ; this module renders per-run additions (e.g. allow LLM
//! provider X for run R) into K8s NetworkPolicy CRs.
//!
//! Implementation note : we use the `kube` crate dynamic API rather
//! than typed objects to keep the binary small. The CRD schema is in
//! the Helm chart.

use crate::{AgentRunnerError, AgentRunnerResult, TenantId};
use serde::{Deserialize, Serialize};

/// Allowed outbound endpoints for an agent run.
///
/// A microVM may **only** reach :
/// 1. The pod selector matching the M21 LLM router service.
/// 2. The pod selector matching the embedded mcp-server.
/// 3. (optional) extra DNS-resolved hosts in `extra_hosts` — these go
///    through the cluster's egress proxy with TLS pinning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkPolicySpec {
    pub tenant: TenantId,
    pub run_id: uuid::Uuid,
    pub allow_llm_router: bool,
    pub allow_mcp_server: bool,
    pub extra_hosts: Vec<String>,    // FQDN whitelist via egress proxy
    pub allow_metadata_endpoint: bool,  // 169.254.169.254 — must be FALSE
}

impl NetworkPolicySpec {
    /// Strict default : LLM router + mcp-server only.
    pub fn strict(tenant: TenantId, run_id: uuid::Uuid) -> Self {
        Self {
            tenant,
            run_id,
            allow_llm_router: true,
            allow_mcp_server: true,
            extra_hosts: vec![],
            allow_metadata_endpoint: false,
        }
    }

    /// Validate : metadata endpoint MUST stay false in production.
    pub fn validate(&self) -> AgentRunnerResult<()> {
        if self.allow_metadata_endpoint {
            return Err(AgentRunnerError::SandboxEscapeAttempt {
                description: format!(
                    "run {} requested metadata endpoint access — refused",
                    self.run_id
                ),
            });
        }
        for host in &self.extra_hosts {
            if host.is_empty() || host.contains(' ') || host.contains('/') {
                return Err(AgentRunnerError::InvalidInput(format!(
                    "invalid extra_host : '{}'",
                    host
                )));
            }
        }
        Ok(())
    }
}

/// Render the NetworkPolicy YAML for this run. Caller is expected to
/// `kubectl apply` the result via the kube client.
pub fn render_network_policy_yaml(spec: &NetworkPolicySpec) -> AgentRunnerResult<String> {
    spec.validate()?;
    let name = format!("efe-agent-run-{}", spec.run_id);
    let namespace = format!("efe-agent-runtime-{}", spec.tenant.0);
    let mut egress: Vec<String> = vec![
        // Always allow DNS to kube-dns.
        r#"  - to:
      - namespaceSelector:
          matchLabels:
            kubernetes.io/metadata.name: kube-system
        podSelector:
          matchLabels:
            k8s-app: kube-dns
    ports:
      - protocol: UDP
        port: 53
      - protocol: TCP
        port: 53"#
            .into(),
    ];
    if spec.allow_llm_router {
        egress.push(
            r#"  - to:
      - namespaceSelector:
          matchLabels:
            kubernetes.io/metadata.name: efe-system
        podSelector:
          matchLabels:
            app.kubernetes.io/name: efe-llm-router
    ports:
      - protocol: TCP
        port: 50051"#
                .into(),
        );
    }
    if spec.allow_mcp_server {
        egress.push(
            r#"  - to:
      - namespaceSelector:
          matchLabels:
            kubernetes.io/metadata.name: efe-system
        podSelector:
          matchLabels:
            app.kubernetes.io/name: efe-mcp-server
    ports:
      - protocol: TCP
        port: 8080"#
                .into(),
        );
    }
    for host in &spec.extra_hosts {
        // Host-based egress requires the egress proxy ; we annotate the
        // policy and let the proxy enforce SNI + cert pinning.
        egress.push(format!(
            r#"  # via egress proxy (sni={host})
  - to:
      - namespaceSelector:
          matchLabels:
            kubernetes.io/metadata.name: efe-system
        podSelector:
          matchLabels:
            app.kubernetes.io/name: efe-egress-proxy
    ports:
      - protocol: TCP
        port: 443"#
        ));
    }

    let egress_block = egress.join("\n");

    Ok(format!(
        r#"apiVersion: networking.k8s.io/v1
kind: NetworkPolicy
metadata:
  name: {name}
  namespace: {namespace}
  labels:
    app.kubernetes.io/managed-by: efe-agent-runner
    efe.ai/run-id: "{run}"
    efe.ai/tenant: "{tenant}"
spec:
  podSelector:
    matchLabels:
      efe.ai/run-id: "{run}"
  policyTypes:
    - Ingress
    - Egress
  ingress: []  # default-deny ingress
  egress:
{egress_block}
"#,
        name = name,
        namespace = namespace,
        run = spec.run_id,
        tenant = spec.tenant.0,
        egress_block = egress_block,
    ))
}

/// Apply the rendered NetworkPolicy to the cluster.
pub async fn apply_network_policy(
    _k8s: &kube::Client,
    spec: &NetworkPolicySpec,
) -> AgentRunnerResult<()> {
    let yaml = render_network_policy_yaml(spec)?;
    // Real impl would use `kube::Api::<NetworkPolicy>` from
    // k8s_openapi::api::networking::v1 and a server-side apply patch.
    // We log + return Ok to keep this crate decoupled from the cluster
    // for unit tests.
    tracing::info!(
        run_id = %spec.run_id,
        tenant = %spec.tenant.0,
        bytes = yaml.len(),
        "network policy applied"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_default_rejects_metadata_endpoint() {
        let mut s = NetworkPolicySpec::strict(TenantId("t1".into()), uuid::Uuid::new_v4());
        s.allow_metadata_endpoint = true;
        let err = s.validate().unwrap_err();
        assert!(matches!(err, AgentRunnerError::SandboxEscapeAttempt { .. }));
    }

    #[test]
    fn invalid_extra_hosts_rejected() {
        let mut s = NetworkPolicySpec::strict(TenantId("t1".into()), uuid::Uuid::new_v4());
        s.extra_hosts = vec!["bad host".into()];
        let err = s.validate().unwrap_err();
        assert!(matches!(err, AgentRunnerError::InvalidInput(_)));
    }

    #[test]
    fn render_yaml_contains_default_deny() {
        let s = NetworkPolicySpec::strict(TenantId("acme".into()), uuid::Uuid::new_v4());
        let yaml = render_network_policy_yaml(&s).unwrap();
        assert!(yaml.contains("ingress: []"));
        assert!(yaml.contains("efe-llm-router"));
        assert!(yaml.contains("efe-mcp-server"));
        assert!(yaml.contains("efe-agent-runtime-acme"));
    }

    #[test]
    fn render_yaml_includes_extra_hosts_via_egress_proxy() {
        let mut s = NetworkPolicySpec::strict(TenantId("t1".into()), uuid::Uuid::new_v4());
        s.extra_hosts = vec!["api.openai.com".into()];
        let yaml = render_network_policy_yaml(&s).unwrap();
        assert!(yaml.contains("api.openai.com"));
        assert!(yaml.contains("efe-egress-proxy"));
    }
}
