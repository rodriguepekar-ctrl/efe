# PR #15 — Rego rule : image tags must be pinned (no :latest).
# Cf. docs/EFE_A1_PHASE2_PR15_ARCHITECTURE.md § 26.2.

package efe.no_latest_tag

import future.keywords.in
import future.keywords.if
import future.keywords.contains

pod_kinds := {"Deployment", "StatefulSet", "DaemonSet", "Job", "ReplicaSet", "Pod", "CronJob"}

# Reject :latest in main containers.
deny contains msg if {
  input.kind in pod_kinds
  some container in containers_of(input)
  endswith(container.image, ":latest")
  msg := sprintf(
    "[no_latest_tag] container %s/%s uses `:latest` image tag (PR #13 AD17 forbids this).",
    [input.metadata.name, container.name],
  )
}

# Reject :latest in initContainers.
deny contains msg if {
  input.kind in pod_kinds
  some container in init_containers_of(input)
  endswith(container.image, ":latest")
  msg := sprintf(
    "[no_latest_tag] initContainer %s/%s uses `:latest` image tag.",
    [input.metadata.name, container.name],
  )
}

# Reject images that have no tag at all (`foo/bar` instead of `foo/bar:1.2.3`).
# `foo/bar@sha256:...` with a digest contains `:` so is treated as pinned.
deny contains msg if {
  input.kind in pod_kinds
  some container in containers_of(input)
  not contains(container.image, ":")
  msg := sprintf(
    "[no_latest_tag] container %s/%s has no image tag at all (`%s`).",
    [input.metadata.name, container.name, container.image],
  )
}

# Helpers : walk into the appropriate spec path.
containers_of(resource) := containers if {
  resource.kind == "Pod"
  containers := object.get(resource.spec, "containers", [])
}

containers_of(resource) := containers if {
  resource.kind in {"Deployment", "StatefulSet", "DaemonSet", "ReplicaSet", "Job"}
  containers := object.get(resource.spec.template.spec, "containers", [])
}

containers_of(resource) := containers if {
  resource.kind == "CronJob"
  containers := object.get(resource.spec.jobTemplate.spec.template.spec, "containers", [])
}

init_containers_of(resource) := containers if {
  resource.kind == "Pod"
  containers := object.get(resource.spec, "initContainers", [])
}

init_containers_of(resource) := containers if {
  resource.kind in {"Deployment", "StatefulSet", "DaemonSet", "ReplicaSet", "Job"}
  containers := object.get(resource.spec.template.spec, "initContainers", [])
}

init_containers_of(resource) := containers if {
  resource.kind == "CronJob"
  containers := object.get(resource.spec.jobTemplate.spec.template.spec, "initContainers", [])
}
