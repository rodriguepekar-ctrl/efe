# PR #15 — Rego rule : runtime pods reject ServiceAccount-token mount.
# Cf. docs/EFE_A1_PHASE2_PR15_ARCHITECTURE.md § 26.6.

package efe.sa_token

import future.keywords.in
import future.keywords.if
import future.keywords.contains

runtime_kinds := {"Deployment", "StatefulSet", "DaemonSet", "Pod"}

# Fire when a runtime pod doesn't explicitly disable SA-token mount.
deny contains msg if {
  input.kind in runtime_kinds
  not exempt(input.metadata.name)
  not pod_spec_of(input).automountServiceAccountToken == false
  msg := sprintf(
    "[sa_token] %s/%s does NOT set `automountServiceAccountToken: false` (PR #13 hardening).",
    [input.kind, input.metadata.name],
  )
}

# Exempt pods : driven by `data/runtime_pods.json` patterns.
exempt(name) if {
  patterns := object.get(data.runtime_pods, "hook_patterns", [])
  some pattern in patterns
  regex.match(pattern, name)
}

exempt(name) if {
  names := object.get(data.runtime_pods, "exempt_names", [])
  some exact in names
  name == exact
}

# Helpers.
pod_spec_of(resource) := resource.spec if resource.kind == "Pod"
pod_spec_of(resource) := resource.spec.template.spec if {
  resource.kind in {"Deployment", "StatefulSet", "DaemonSet"}
}
