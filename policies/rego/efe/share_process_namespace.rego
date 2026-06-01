# PR #15 — Rego rule : shareProcessNamespace gated on bundle-loader sidecar.
# Cf. docs/EFE_A1_PHASE2_PR15_ARCHITECTURE.md § 26.7.

package efe.share_process_namespace

import future.keywords.in
import future.keywords.if
import future.keywords.contains

pod_kinds := {"Deployment", "StatefulSet", "DaemonSet", "Job", "ReplicaSet", "Pod"}

# Reject shareProcessNamespace WITHOUT a `bundle-loader` container.
deny contains msg if {
  input.kind in pod_kinds
  pod_spec_of(input).shareProcessNamespace == true
  not has_bundle_loader(input)
  msg := sprintf(
    "[share_process_namespace] %s/%s sets shareProcessNamespace=true but has NO `bundle-loader` container (T26).",
    [input.kind, input.metadata.name],
  )
}

# Inverse : has bundle-loader BUT shareProcessNamespace is not true.
deny contains msg if {
  input.kind in pod_kinds
  has_bundle_loader(input)
  not pod_spec_of(input).shareProcessNamespace == true
  msg := sprintf(
    "[share_process_namespace] %s/%s has `bundle-loader` sidecar but does NOT set shareProcessNamespace=true (sidecar will not see PID 1).",
    [input.kind, input.metadata.name],
  )
}

has_bundle_loader(resource) if {
  some container in containers_of(resource)
  container.name == "bundle-loader"
}

# Helpers.
pod_spec_of(resource) := resource.spec if resource.kind == "Pod"
pod_spec_of(resource) := resource.spec.template.spec if {
  resource.kind in {"Deployment", "StatefulSet", "DaemonSet", "ReplicaSet", "Job"}
}

containers_of(resource) := object.get(pod_spec_of(resource), "containers", [])
