# PR #15 — Rego rule : no privileged containers.
# Cf. docs/EFE_A1_PHASE2_PR15_ARCHITECTURE.md § 26.4.

package efe.no_privileged

import future.keywords.in
import future.keywords.if
import future.keywords.contains

pod_kinds := {"Deployment", "StatefulSet", "DaemonSet", "Job", "ReplicaSet", "Pod", "CronJob"}

# Main container is privileged.
deny contains msg if {
  input.kind in pod_kinds
  some container in containers_of(input)
  object.get(object.get(container, "securityContext", {}), "privileged", false) == true
  msg := sprintf(
    "[no_privileged] %s/%s container `%s` runs PRIVILEGED (forbidden).",
    [input.kind, input.metadata.name, container.name],
  )
}

# initContainer is privileged.
deny contains msg if {
  input.kind in pod_kinds
  some container in init_containers_of(input)
  object.get(object.get(container, "securityContext", {}), "privileged", false) == true
  msg := sprintf(
    "[no_privileged] %s/%s initContainer `%s` runs PRIVILEGED (forbidden).",
    [input.kind, input.metadata.name, container.name],
  )
}

# Privileged via securityContext.capabilities.add with dangerous caps.
dangerous_caps := {
  "SYS_ADMIN", "NET_ADMIN", "NET_RAW", "SYS_PTRACE", "SYS_MODULE",
  "SYS_RAWIO", "BPF", "DAC_READ_SEARCH", "DAC_OVERRIDE", "FOWNER",
  "FSETID", "KILL", "SETGID", "SETUID", "SETPCAP", "LINUX_IMMUTABLE",
  "NET_BIND_SERVICE", "NET_BROADCAST", "MAC_OVERRIDE", "MAC_ADMIN",
  "AUDIT_CONTROL", "AUDIT_WRITE", "SETFCAP",
}

deny contains msg if {
  input.kind in pod_kinds
  some container in containers_of(input)
  caps := object.get(object.get(container, "securityContext", {}), "capabilities", {})
  some cap in object.get(caps, "add", [])
  cap in dangerous_caps
  msg := sprintf(
    "[no_privileged] %s/%s container `%s` adds dangerous capability %v (forbidden).",
    [input.kind, input.metadata.name, container.name, cap],
  )
}

containers_of(resource) := object.get(resource.spec, "containers", []) if {
  resource.kind == "Pod"
}
containers_of(resource) := object.get(resource.spec.template.spec, "containers", []) if {
  resource.kind in {"Deployment", "StatefulSet", "DaemonSet", "ReplicaSet", "Job"}
}
containers_of(resource) := object.get(resource.spec.jobTemplate.spec.template.spec, "containers", []) if {
  resource.kind == "CronJob"
}

init_containers_of(resource) := object.get(resource.spec, "initContainers", []) if {
  resource.kind == "Pod"
}
init_containers_of(resource) := object.get(resource.spec.template.spec, "initContainers", []) if {
  resource.kind in {"Deployment", "StatefulSet", "DaemonSet", "ReplicaSet", "Job"}
}
init_containers_of(resource) := object.get(resource.spec.jobTemplate.spec.template.spec, "initContainers", []) if {
  resource.kind == "CronJob"
}
