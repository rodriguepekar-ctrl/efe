# Tests for metric_help_nonempty.rego — PR #17 AD19 / AC2.

package efe.metric_help_nonempty

valid_alert := {
    "apiVersion": "monitoring.coreos.com/v1",
    "kind": "PrometheusRule",
    "metadata": {
        "namespace": "efe",
        "name": "x",
        "labels": {"app.kubernetes.io/part-of": "efe-meta-swarm"},
    },
    "spec": {"groups": [{
        "name": "g",
        "rules": [{
            "alert": "MetaSwarmIterateDurationP99High",
            "expr": "test",
            "annotations": {
                "summary": "Iterate p99 latency above SLO budget",
                "description": "Detailed description here",
                "runbook_url": "https://example.com/runbook",
            },
        }],
    }]},
}

test_valid_alert_passes {
    count(deny) == 0 with input as valid_alert
    count(warn) == 0 with input as valid_alert
}

test_missing_summary_denied {
    bad := json.patch(valid_alert, [{"op": "remove", "path": "/spec/groups/0/rules/0/annotations/summary"}])
    count(deny) > 0 with input as bad
}

test_missing_description_denied {
    bad := json.patch(valid_alert, [{"op": "remove", "path": "/spec/groups/0/rules/0/annotations/description"}])
    count(deny) > 0 with input as bad
}

test_missing_runbook_url_denied {
    bad := json.patch(valid_alert, [{"op": "remove", "path": "/spec/groups/0/rules/0/annotations/runbook_url"}])
    count(deny) > 0 with input as bad
}

test_short_summary_warns {
    bad := json.patch(valid_alert, [{"op": "replace", "path": "/spec/groups/0/rules/0/annotations/summary", "value": "Bad"}])
    count(warn) > 0 with input as bad
}

test_non_meta_swarm_rule_ignored {
    other := json.patch(valid_alert, [{"op": "replace", "path": "/metadata/labels/app.kubernetes.io~1part-of", "value": "other-chart"}])
    other2 := json.patch(other, [{"op": "remove", "path": "/spec/groups/0/rules/0/annotations/summary"}])
    count(deny) == 0 with input as other2
}
