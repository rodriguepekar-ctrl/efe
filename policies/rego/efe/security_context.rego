# PR #15 — Rego rule : every container MUST declare securityContext.
# Cf. docs/EFE_A1_PHASE2_PR15_ARCHITECTURE.md § 26.3.
#
# Audit-pass fix : use object.get() everywhere instead of bare field
# access.  Bare access on undefined fields evaluates to undefined,
# which makes the rule body silently fail (no deny fires) — a false
# negative.  object.get(..., default) provides explicit fallback so
# missing sub-fields trigger the deny correctly.

package efe.security_context

import future.keywords.in
import future.keywords.if
import future.keywords.contains

pod_kinds := {"Deployment", "StatefulSet", "DaemonSet", "Job", "ReplicaSet", "Pod", "CronJob"}

# 1) Container-level securityContext entirely missing.
deny contains msg if {
  input.kind in pod_kinds
  some container in containers_of(input)
  not container.securityContext
  msg := sprintf(
    "[security_context] container %s/%s has no securityContext.",
    [input.metadata.name, container.name],
  )
}

# 2) capabilities.drop must include "ALL".
deny contains msg if {
  input.kind in pod_kinds
  some container in containers_of(input)
  container.securityContext
  sc := container.securityContext
  caps := object.get(sc, "capabilities", {})
  drop := object.get(caps, "drop", [])
  not "ALL" in drop
  msg := sprintf(
    "[security_context] container %s/%s does NOT drop ALL capabilities.",
    [input.metadata.name, container.name],
  )
}

# 3) runAsNonRoot must be true.
deny contains msg if {
  input.kind in pod_kinds
  some container in containers_of(input)
  container.securityContext
  not object.get(container.securityContext, "runAsNonRoot", false) == true
  msg := sprintf(
    "[security_context] container %s/%s : securityContext.runAsNonRoot is not `true`.",
    [input.metadata.name, container.name],
  )
}

# 4) allowPrivilegeEscalation must be false.
deny contains msg if {
  input.kind in pod_kinds
  some container in containers_of(input)
  container.securityContext
  not object.get(container.securityContext, "allowPrivilegeEscalation", true) == false
  msg := sprintf(
    "[security_context] container %s/%s : allowPrivilegeEscalation is not `false`.",
    [input.metadata.name, container.name],
  )
}

# 5) readOnlyRootFilesystem must be true.
deny contains msg if {
  input.kind in pod_kinds
  some container in containers_of(input)
  container.securityContext
  not object.get(container.securityContext, "readOnlyRootFilesystem", false) == true
  msg := sprintf(
    "[security_context] container %s/%s : readOnlyRootFilesystem is not `true`.",
    [input.metadata.name, container.name],
  )
}

# 6) Pod-level seccompProfile must be RuntimeDefault or Localhost.
# This is opt-in : we only fire if a seccompProfile is SET but with
# a bad type.  Missing seccompProfile is fine (cluster default).
deny contains msg if {
  input.kind in pod_kinds
  spec := pod_spec_of(input)
  spec.securityContext
  sp := object.get(spec.securityContext, "seccompProfile", null)
  sp != null
  not sp.type in {"RuntimeDefault", "Localhost"}
  msg := sprintf(
    "[security_context] %s/%s : seccompProfile.type must be RuntimeDefault or Localhost (got %v).",
    [input.kind, input.metadata.name, sp.type],
  )
}

# Helpers.
containers_of(resource) := object.get(resource.spec, "containers", []) if {
  resource.kind == "Pod"
}
containers_of(resource) := object.get(resource.spec.template.spec, "containers", []) if {
  resource.kind in {"Deployment", "StatefulSet", "DaemonSet", "ReplicaSet", "Job"}
}
containers_of(resource) := object.get(resource.spec.jobTemplate.spec.template.spec, "containers", []) if {
  resource.kind == "CronJob"
}

pod_spec_of(resource) := resource.spec if resource.kind == "Pod"
pod_spec_of(resource) := resource.spec.template.spec if {
  resource.kind in {"Deployment", "StatefulSet", "DaemonSet", "ReplicaSet", "Job"}
}
pod_spec_of(resource) := resource.spec.jobTemplate.spec.template.spec if {
  resource.kind == "CronJob"
}
