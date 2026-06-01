# PR #15 — Rego rule : every NetworkPolicy declares spec.policyTypes.
# Cf. docs/EFE_A1_PHASE2_PR15_ARCHITECTURE.md § 26.10.

package efe.networkpolicy_types

import future.keywords.in
import future.keywords.if
import future.keywords.contains

deny contains msg if {
  input.kind == "NetworkPolicy"
  not input.spec.policyTypes
  msg := sprintf(
    "[networkpolicy_types] NetworkPolicy `%s` has no `spec.policyTypes` (default depends on ingress/egress blocks — be explicit).",
    [input.metadata.name],
  )
}

deny contains msg if {
  input.kind == "NetworkPolicy"
  count(input.spec.policyTypes) == 0
  msg := sprintf(
    "[networkpolicy_types] NetworkPolicy `%s` has empty `policyTypes` (allow-list of `Ingress`, `Egress`).",
    [input.metadata.name],
  )
}

valid_policy_types := {"Ingress", "Egress"}

deny contains msg if {
  input.kind == "NetworkPolicy"
  some pt in input.spec.policyTypes
  not pt in valid_policy_types
  msg := sprintf(
    "[networkpolicy_types] NetworkPolicy `%s` has invalid policyTypes value %v (must be Ingress or Egress).",
    [input.metadata.name, pt],
  )
}

# Egress rules present but `Egress` not in policyTypes → silently ignored.
deny contains msg if {
  input.kind == "NetworkPolicy"
  count(object.get(input.spec, "egress", [])) > 0
  not "Egress" in object.get(input.spec, "policyTypes", [])
  msg := sprintf(
    "[networkpolicy_types] NetworkPolicy `%s` has egress rules but `policyTypes` does not include Egress (rules silently ignored).",
    [input.metadata.name],
  )
}

# Symmetric : ingress rules without `Ingress` in policyTypes.
deny contains msg if {
  input.kind == "NetworkPolicy"
  count(object.get(input.spec, "ingress", [])) > 0
  not "Ingress" in object.get(input.spec, "policyTypes", [])
  msg := sprintf(
    "[networkpolicy_types] NetworkPolicy `%s` has ingress rules but `policyTypes` does not include Ingress (rules silently ignored).",
    [input.metadata.name],
  )
}
