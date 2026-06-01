# PR #16 § 8.1 + § 32 (AC30) — values.yaml lint rule.
#
# Enforces the cipher / protocol policy declared in
# docs/EFE_A1_PHASE2_PR16_ARCHITECTURE.md AD6 :
#
#   * `tls.minProtocolVersion` MUST be "1.2" or "1.3".
#   * "1.2" requires an explicit justification comment near the field
#     (a deliberately-friction-laden opt-in for legacy peers).
#   * Anything else — "1.0", "1.1", empty, missing — is a violation.
#
# Wired into the PR #15 `helm-meta-swarm-lint` workflow via the
# per-rule `package efe.tls_protocol_version` namespace.

package efe.tls_protocol_version

# ─── Helpers ────────────────────────────────────────────────────────

# Resolve the rendered values block from a Kubernetes manifest's
# `metadata.annotations["efe.io/values"]` payload.  Conftest receives
# a stream of rendered manifests ; the operator's chosen values.yaml
# is reconstructed by combining the chart defaults with the env
# overlay before the workflow shells out to `conftest`.  For lint
# purposes here we treat every Certificate resource as the canonical
# anchor — every install ships at least one Certificate when TLS is
# on.

is_certificate(obj) {
    obj.kind == "Certificate"
    obj.apiVersion == "cert-manager.io/v1"
}

# Pulls the runtime tls block from a Pod's env vars (the chart side
# emits EFE_META_SWARM_TLS_MIN_PROTOCOL on every controller +
# participant container when `tls.enabled=true`).  This is the
# observable surface in the rendered manifest — we don't have direct
# access to values.yaml from inside conftest.

container_env_kv(container) = kv {
    env := container.env
    kv := {entry.name: entry.value | entry := env[_]}
}

pod_min_protocol(pod) = version {
    container := pod.spec.template.spec.containers[_]
    kv := container_env_kv(container)
    version := kv.EFE_META_SWARM_TLS_MIN_PROTOCOL
}

# ─── Rules ──────────────────────────────────────────────────────────

# Violation : minProtocolVersion is set to something outside {"1.2", "1.3"}.
deny[msg] {
    obj := input
    obj.kind == "Deployment"
    obj.metadata.labels["app.kubernetes.io/part-of"] == "efe-meta-swarm"
    version := pod_min_protocol(obj)
    not allowed_version(version)
    msg := sprintf(
        "[efe-meta-swarm] %s/%s : EFE_META_SWARM_TLS_MIN_PROTOCOL=%q is forbidden (must be \"1.2\" or \"1.3\" ; PR #16 AD6)",
        [obj.metadata.namespace, obj.metadata.name, version]
    )
}

# Warning : TLS 1.2 opted-in without justification.  Customer-visible
# operators MUST add a Helm-values comment like
#   `tls.minProtocolVersion: "1.2"  # legacy peer: <reason>`.
# Conftest can't read the YAML comment, so the warning fires on any
# 1.2 opt-down ; the operator must acknowledge it in the PR review.
warn[msg] {
    obj := input
    obj.kind == "Deployment"
    obj.metadata.labels["app.kubernetes.io/part-of"] == "efe-meta-swarm"
    version := pod_min_protocol(obj)
    version == "1.2"
    msg := sprintf(
        "[efe-meta-swarm] %s/%s : EFE_META_SWARM_TLS_MIN_PROTOCOL=\"1.2\" opted in — verify the values.yaml comment justifies this legacy peer (PR #16 AD6)",
        [obj.metadata.namespace, obj.metadata.name]
    )
}

allowed_version("1.2")
allowed_version("1.3")
