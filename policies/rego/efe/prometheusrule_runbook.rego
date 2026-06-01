# PR #15 — Rego rule : every PrometheusRule alert has annotations.runbook_url.
# Cf. docs/EFE_A1_PHASE2_PR15_ARCHITECTURE.md § 26.11.

package efe.prometheusrule_runbook

import future.keywords.in
import future.keywords.if
import future.keywords.contains

deny contains msg if {
  input.kind == "PrometheusRule"
  some group in input.spec.groups
  some rule in group.rules
  rule.alert
  not object.get(object.get(rule, "annotations", {}), "runbook_url", "") != ""
  msg := sprintf(
    "[prometheusrule_runbook] PrometheusRule `%s` alert `%s` (group `%s`) has no `annotations.runbook_url`.",
    [input.metadata.name, rule.alert, group.name],
  )
}

# runbook_url must look like a URL (http:// or https://).
deny contains msg if {
  input.kind == "PrometheusRule"
  some group in input.spec.groups
  some rule in group.rules
  rule.alert
  url := object.get(object.get(rule, "annotations", {}), "runbook_url", "")
  url != ""
  not startswith(url, "http://")
  not startswith(url, "https://")
  msg := sprintf(
    "[prometheusrule_runbook] PrometheusRule `%s` alert `%s` runbook_url is not http(s) (`%s`).",
    [input.metadata.name, rule.alert, url],
  )
}

# Every alerting rule MUST carry a `severity` label.
deny contains msg if {
  input.kind == "PrometheusRule"
  some group in input.spec.groups
  some rule in group.rules
  rule.alert
  not object.get(rule, "labels", {}).severity
  msg := sprintf(
    "[prometheusrule_runbook] PrometheusRule `%s` alert `%s` has no `labels.severity`.",
    [input.metadata.name, rule.alert],
  )
}

valid_severities := {"info", "warning", "critical"}

deny contains msg if {
  input.kind == "PrometheusRule"
  some group in input.spec.groups
  some rule in group.rules
  rule.alert
  sev := object.get(rule, "labels", {}).severity
  sev
  not sev in valid_severities
  msg := sprintf(
    "[prometheusrule_runbook] PrometheusRule `%s` alert `%s` has invalid severity `%v` (must be info/warning/critical).",
    [input.metadata.name, rule.alert, sev],
  )
}

# Every alerting rule MUST have `annotations.summary`.
deny contains msg if {
  input.kind == "PrometheusRule"
  some group in input.spec.groups
  some rule in group.rules
  rule.alert
  not object.get(rule, "annotations", {}).summary
  msg := sprintf(
    "[prometheusrule_runbook] PrometheusRule `%s` alert `%s` has no `annotations.summary` (no human-readable title).",
    [input.metadata.name, rule.alert],
  )
}
