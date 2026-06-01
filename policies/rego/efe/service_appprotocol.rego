# PR #15 — Rego rule : every Service port has appProtocol set.
# Cf. docs/EFE_A1_PHASE2_PR15_ARCHITECTURE.md § 26.8.

package efe.service_appprotocol

import future.keywords.in
import future.keywords.if
import future.keywords.contains

deny contains msg if {
  input.kind == "Service"
  some port in input.spec.ports
  not port.appProtocol
  msg := sprintf(
    "[service_appprotocol] Service `%s` port `%s` (%v/%v) has no appProtocol (Prometheus / Istio routing degrades).",
    [
      input.metadata.name,
      object.get(port, "name", "<unnamed>"),
      object.get(port, "protocol", "TCP"),
      port.port,
    ],
  )
}

deny contains msg if {
  input.kind == "Service"
  some port in input.spec.ports
  port.appProtocol == ""
  msg := sprintf(
    "[service_appprotocol] Service `%s` port `%s` has empty appProtocol.",
    [input.metadata.name, object.get(port, "name", "<unnamed>")],
  )
}
