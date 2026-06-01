# PR #15 — Rego rule : every namespaced resource MUST carry the
# `efe.io/tenant` label (AC28 from PR #14, locked here as policy).
#
# Cf. docs/EFE_A1_PHASE2_PR15_ARCHITECTURE.md § 26.1.
#
# Scope : the rendered K8s manifests (output of `helm template`).
# Cluster-scoped kinds (PriorityClass, ClusterRole, ClusterRoleBinding)
# are exempt — the label is namespaced-resource semantics.
#
# PR #15 audit pass : per-rule package so `conftest verify` isolates
# this rule's tests from neighbours' denies.  `conftest test` aggregates
# via `--all-namespaces`.

package efe.tenant_label

import future.keywords.in
import future.keywords.if
import future.keywords.contains

# Cluster-scoped kinds that are exempt from the tenant label rule.
cluster_scoped_kinds := {
  "PriorityClass",
  "ClusterRole",
  "ClusterRoleBinding",
  "Namespace",
  "Node",
  "StorageClass",
  "PersistentVolume",
  "MutatingWebhookConfiguration",
  "ValidatingWebhookConfiguration",
  "CustomResourceDefinition",
}

# Fire when a namespaced resource is missing efe.io/tenant.
deny contains msg if {
  input.kind != ""
  not input.kind in cluster_scoped_kinds
  labels := object.get(input.metadata, "labels", {})
  not labels["efe.io/tenant"]
  msg := sprintf(
    "[tenant_label] %s/%s is missing the `efe.io/tenant` label (PR #14 AC28).",
    [input.kind, input.metadata.name],
  )
}

# Fire when the tenant label is set but is an empty string.
deny contains msg if {
  input.kind != ""
  not input.kind in cluster_scoped_kinds
  labels := object.get(input.metadata, "labels", {})
  labels["efe.io/tenant"] == ""
  msg := sprintf(
    "[tenant_label] %s/%s has empty `efe.io/tenant` label.",
    [input.kind, input.metadata.name],
  )
}
