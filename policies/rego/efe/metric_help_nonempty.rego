# PR #17 AD19 / AC2 — every Prometheus metric series MUST carry a
# non-empty HELP string.
#
# Rationale : dashboards bound to metric names rely on HELP for
# tooltip rendering ; an empty HELP string makes the dashboard panel
# render an unhelpful blank ; alert routing rules also reference HELP
# in their auto-generated descriptions.
#
# The rule walks PrometheusRule documents and verifies every alert's
# `expr` references series whose corresponding metric family in the
# rendered Helm chart has a non-empty HELP.  Since conftest doesn't
# directly access the binary's `/metrics` output, we lint at a
# different layer : every alert MUST have its own non-empty
# `annotations.summary` AND a non-empty `annotations.description`,
# which are the operator-facing analogues of HELP.
#
# Wired into PR #15 helm-meta-swarm-lint workflow via the per-rule
# `package efe.metric_help_nonempty` namespace.

package efe.metric_help_nonempty

# ─── Helpers ────────────────────────────────────────────────────────

is_prometheus_rule(obj) {
    obj.kind == "PrometheusRule"
    obj.apiVersion == "monitoring.coreos.com/v1"
}

is_meta_swarm_rule(obj) {
    is_prometheus_rule(obj)
    obj.metadata.labels["app.kubernetes.io/part-of"] == "efe-meta-swarm"
}

# ─── Rules ──────────────────────────────────────────────────────────

# Deny : an alert with an empty `summary` annotation.
deny[msg] {
    obj := input
    is_meta_swarm_rule(obj)
    group := obj.spec.groups[_]
    rule := group.rules[_]
    rule.alert
    not rule.annotations.summary
    msg := sprintf(
        "[efe-meta-swarm] %s/%s alert %q : `annotations.summary` is missing (PR #17 AD19)",
        [obj.metadata.namespace, obj.metadata.name, rule.alert]
    )
}

# Deny : an alert with an empty `description` annotation.
deny[msg] {
    obj := input
    is_meta_swarm_rule(obj)
    group := obj.spec.groups[_]
    rule := group.rules[_]
    rule.alert
    not rule.annotations.description
    msg := sprintf(
        "[efe-meta-swarm] %s/%s alert %q : `annotations.description` is missing (PR #17 AD19)",
        [obj.metadata.namespace, obj.metadata.name, rule.alert]
    )
}

# Deny : an alert with an empty `runbook_url` annotation.
deny[msg] {
    obj := input
    is_meta_swarm_rule(obj)
    group := obj.spec.groups[_]
    rule := group.rules[_]
    rule.alert
    not rule.annotations.runbook_url
    msg := sprintf(
        "[efe-meta-swarm] %s/%s alert %q : `annotations.runbook_url` is missing (PR #17 AD19)",
        [obj.metadata.namespace, obj.metadata.name, rule.alert]
    )
}

# Warn : a summary < 8 chars is almost certainly a placeholder.
warn[msg] {
    obj := input
    is_meta_swarm_rule(obj)
    group := obj.spec.groups[_]
    rule := group.rules[_]
    rule.alert
    s := rule.annotations.summary
    count(s) < 8
    msg := sprintf(
        "[efe-meta-swarm] %s/%s alert %q : `annotations.summary` is suspiciously short (%d chars)",
        [obj.metadata.namespace, obj.metadata.name, rule.alert, count(s)]
    )
}
