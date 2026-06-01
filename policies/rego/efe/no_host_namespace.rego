# PR #15 — Rego rule : no hostNetwork / hostPID / hostIPC.
# Cf. docs/EFE_A1_PHASE2_PR15_ARCHITECTURE.md § 26.5.

package efe.no_host_namespace

import future.keywords.in
import future.keywords.if
import future.keywords.contains

pod_kinds := {"Deployment", "StatefulSet", "DaemonSet", "Job", "ReplicaSet", "Pod", "CronJob"}

deny contains msg if {
  input.kind in pod_kinds
  pod_spec_of(input).hostNetwork == true
  msg := sprintf(
    "[no_host_namespace] %s/%s sets hostNetwork=true (forbidden — CIS K8s 5.2.4).",
    [input.kind, input.metadata.name],
  )
}

deny contains msg if {
  input.kind in pod_kinds
  pod_spec_of(input).hostPID == true
  msg := sprintf(
    "[no_host_namespace] %s/%s sets hostPID=true (forbidden — CIS K8s 5.2.2).",
    [input.kind, input.metadata.name],
  )
}

deny contains msg if {
  input.kind in pod_kinds
  pod_spec_of(input).hostIPC == true
  msg := sprintf(
    "[no_host_namespace] %s/%s sets hostIPC=true (forbidden — CIS K8s 5.2.3).",
    [input.kind, input.metadata.name],
  )
}

# hostPort : binds a pod port to the node's network namespace.
deny contains msg if {
  input.kind in pod_kinds
  some container in containers_of(input)
  some port in object.get(container, "ports", [])
  port.hostPort
  msg := sprintf(
    "[no_host_namespace] %s/%s container `%s` declares hostPort %v (forbidden).",
    [input.kind, input.metadata.name, container.name, port.hostPort],
  )
}

# Helpers.
pod_spec_of(resource) := resource.spec if resource.kind == "Pod"
pod_spec_of(resource) := resource.spec.template.spec if {
  resource.kind in {"Deployment", "StatefulSet", "DaemonSet", "ReplicaSet", "Job"}
}
pod_spec_of(resource) := resource.spec.jobTemplate.spec.template.spec if {
  resource.kind == "CronJob"
}

containers_of(resource) := object.get(pod_spec_of(resource), "containers", [])
