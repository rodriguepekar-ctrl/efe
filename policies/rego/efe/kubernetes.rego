# PR #15 — CIS K8s benchmark v1.8 top-10 aggregator.
# Cf. docs/EFE_A1_PHASE2_PR15_ARCHITECTURE.md § 26.12.

package efe.kubernetes

import future.keywords.in
import future.keywords.if
import future.keywords.contains

# ─── Default ServiceAccount forbidden for runtime pods ──────────────
runtime_kinds := {"Deployment", "StatefulSet", "DaemonSet", "Pod"}

deny contains msg if {
  input.kind in runtime_kinds
  spec := pod_spec_of(input)
  sa := object.get(spec, "serviceAccountName", "default")
  sa == "default"
  msg := sprintf(
    "[kubernetes.cis] %s/%s uses default ServiceAccount (CIS 5.1.5 — assign explicit SA).",
    [input.kind, input.metadata.name],
  )
}

# ─── RoleBinding / ClusterRoleBinding to default SA forbidden ─────
deny contains msg if {
  input.kind in {"RoleBinding", "ClusterRoleBinding"}
  some subject in input.subjects
  subject.kind == "ServiceAccount"
  subject.name == "default"
  msg := sprintf(
    "[kubernetes.cis] %s `%s` binds to default ServiceAccount (CIS 5.1.5).",
    [input.kind, input.metadata.name],
  )
}

# ─── Service.spec.type LoadBalancer requires explicit annotation ──
deny contains msg if {
  input.kind == "Service"
  input.spec.type == "LoadBalancer"
  ann := object.get(input.metadata, "annotations", {})
  not ann["efe.io/lb-acl"]
  msg := sprintf(
    "[kubernetes.cis] Service `%s` is LoadBalancer but has no `efe.io/lb-acl` annotation (operator opt-in required).",
    [input.metadata.name],
  )
}

# ─── HPA bounds sanity ─────────────────────────────────────────────
deny contains msg if {
  input.kind == "HorizontalPodAutoscaler"
  input.spec.minReplicas > input.spec.maxReplicas
  msg := sprintf(
    "[kubernetes.cis] HPA `%s` has minReplicas (%v) > maxReplicas (%v).",
    [input.metadata.name, input.spec.minReplicas, input.spec.maxReplicas],
  )
}

# Phase-2 lock : meta-swarm controller HPA must be min=max=1.
deny contains msg if {
  input.kind == "HorizontalPodAutoscaler"
  regex.match("-efe-meta-swarm-controller$", input.metadata.name)
  not input.spec.minReplicas == 1
  msg := sprintf(
    "[kubernetes.cis] Meta-swarm controller HPA `%s` minReplicas must be 1 in Phase 2 (PR #14 AC21).",
    [input.metadata.name],
  )
}

deny contains msg if {
  input.kind == "HorizontalPodAutoscaler"
  regex.match("-efe-meta-swarm-controller$", input.metadata.name)
  not input.spec.maxReplicas == 1
  msg := sprintf(
    "[kubernetes.cis] Meta-swarm controller HPA `%s` maxReplicas must be 1 in Phase 2 (PR #14 AC21).",
    [input.metadata.name],
  )
}

# ─── PodDisruptionBudget must declare maxUnavailable OR minAvailable ──
deny contains msg if {
  input.kind == "PodDisruptionBudget"
  not input.spec.maxUnavailable
  not input.spec.minAvailable
  msg := sprintf(
    "[kubernetes.cis] PodDisruptionBudget `%s` has neither maxUnavailable nor minAvailable.",
    [input.metadata.name],
  )
}

# ─── ConfigMap content sanity : reject empty `data` ───────────────
deny contains msg if {
  input.kind == "ConfigMap"
  not input.data
  not input.binaryData
  msg := sprintf(
    "[kubernetes.cis] ConfigMap `%s` has empty `data` and `binaryData` (template-render bug?).",
    [input.metadata.name],
  )
}

# ─── Job restartPolicy : must be Never or OnFailure ───────────────
deny contains msg if {
  input.kind == "Job"
  not input.spec.template.spec.restartPolicy in {"Never", "OnFailure"}
  msg := sprintf(
    "[kubernetes.cis] Job `%s` restartPolicy must be Never or OnFailure (got %v).",
    [input.metadata.name, input.spec.template.spec.restartPolicy],
  )
}

# ─── Helpers ───────────────────────────────────────────────────────
pod_spec_of(resource) := resource.spec if resource.kind == "Pod"
pod_spec_of(resource) := resource.spec.template.spec if {
  resource.kind in {"Deployment", "StatefulSet", "DaemonSet", "ReplicaSet", "Job"}
}
