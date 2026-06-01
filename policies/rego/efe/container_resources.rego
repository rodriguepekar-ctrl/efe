# PR #15 — Rego rule : container resources.requests AND resources.limits required.
# Cf. docs/EFE_A1_PHASE2_PR15_ARCHITECTURE.md § 26.9.

package efe.container_resources

import future.keywords.in
import future.keywords.if
import future.keywords.contains

pod_kinds := {"Deployment", "StatefulSet", "DaemonSet", "Job", "ReplicaSet", "Pod", "CronJob"}

# requests required (object-level).
deny contains msg if {
  input.kind in pod_kinds
  some container in containers_of(input)
  not object.get(container, "resources", {}).requests
  msg := sprintf(
    "[container_resources] %s/%s container `%s` has NO resources.requests.",
    [input.kind, input.metadata.name, container.name],
  )
}

# limits required (object-level).
deny contains msg if {
  input.kind in pod_kinds
  some container in containers_of(input)
  not object.get(container, "resources", {}).limits
  msg := sprintf(
    "[container_resources] %s/%s container `%s` has NO resources.limits.",
    [input.kind, input.metadata.name, container.name],
  )
}

# requests.cpu required.
deny contains msg if {
  input.kind in pod_kinds
  some container in containers_of(input)
  requests := object.get(object.get(container, "resources", {}), "requests", {})
  requests != {}
  not requests.cpu
  msg := sprintf(
    "[container_resources] %s/%s container `%s` requests.cpu is missing.",
    [input.kind, input.metadata.name, container.name],
  )
}

# requests.memory required.
deny contains msg if {
  input.kind in pod_kinds
  some container in containers_of(input)
  requests := object.get(object.get(container, "resources", {}), "requests", {})
  requests != {}
  not requests.memory
  msg := sprintf(
    "[container_resources] %s/%s container `%s` requests.memory is missing.",
    [input.kind, input.metadata.name, container.name],
  )
}

# limits.cpu required.
deny contains msg if {
  input.kind in pod_kinds
  some container in containers_of(input)
  limits := object.get(object.get(container, "resources", {}), "limits", {})
  limits != {}
  not limits.cpu
  msg := sprintf(
    "[container_resources] %s/%s container `%s` limits.cpu is missing.",
    [input.kind, input.metadata.name, container.name],
  )
}

# limits.memory required.
deny contains msg if {
  input.kind in pod_kinds
  some container in containers_of(input)
  limits := object.get(object.get(container, "resources", {}), "limits", {})
  limits != {}
  not limits.memory
  msg := sprintf(
    "[container_resources] %s/%s container `%s` limits.memory is missing.",
    [input.kind, input.metadata.name, container.name],
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
