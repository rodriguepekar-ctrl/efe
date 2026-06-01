# PR #15 — Rego unit tests : prometheusrule_runbook.rego.

package efe.prometheusrule_runbook

import future.keywords.if

test_allow_alert_with_runbook_and_severity if {
  count(deny) == 0 with input as {
    "kind": "PrometheusRule",
    "metadata": {"name": "alerts"},
    "spec": {"groups": [{
      "name": "efe-meta-swarm",
      "rules": [{
        "alert": "MetaSwarmReloadFailureRate",
        "expr": "rate(efe_meta_swarm_reload_failures_total[5m]) > 0.1",
        "for": "5m",
        "labels": {"severity": "warning", "component": "meta-swarm"},
        "annotations": {
          "summary": "Reload failure rate elevated",
          "runbook_url": "https://efe.example.com/runbook/meta-swarm#reload-failures",
        },
      }],
    }]},
  }
}

# Recording rule (no `alert` field) — must be ignored by all denies.
test_allow_recording_rule if {
  count(deny) == 0 with input as {
    "kind": "PrometheusRule",
    "metadata": {"name": "recordings"},
    "spec": {"groups": [{
      "name": "rec",
      "rules": [{
        "record": "efe:meta_swarm_reload_failures:rate5m",
        "expr": "rate(efe_meta_swarm_reload_failures_total[5m])",
      }],
    }]},
  }
}

test_deny_alert_without_runbook if {
  count(deny) > 0 with input as {
    "kind": "PrometheusRule",
    "metadata": {"name": "alerts"},
    "spec": {"groups": [{
      "name": "g",
      "rules": [{
        "alert": "MyAlert",
        "expr": "up == 0",
        "labels": {"severity": "warning"},
        "annotations": {"summary": "..."},
      }],
    }]},
  }
}

test_deny_empty_runbook if {
  count(deny) > 0 with input as {
    "kind": "PrometheusRule",
    "metadata": {"name": "alerts"},
    "spec": {"groups": [{
      "name": "g",
      "rules": [{
        "alert": "MyAlert",
        "expr": "up == 0",
        "labels": {"severity": "warning"},
        "annotations": {"summary": "...", "runbook_url": ""},
      }],
    }]},
  }
}

test_deny_non_http_runbook if {
  count(deny) > 0 with input as {
    "kind": "PrometheusRule",
    "metadata": {"name": "alerts"},
    "spec": {"groups": [{
      "name": "g",
      "rules": [{
        "alert": "MyAlert",
        "expr": "up == 0",
        "labels": {"severity": "warning"},
        "annotations": {"summary": "...", "runbook_url": "ftp://example.com"},
      }],
    }]},
  }
}

test_deny_alert_without_severity if {
  count(deny) > 0 with input as {
    "kind": "PrometheusRule",
    "metadata": {"name": "alerts"},
    "spec": {"groups": [{
      "name": "g",
      "rules": [{
        "alert": "MyAlert",
        "expr": "up == 0",
        "labels": {},
        "annotations": {"summary": "...", "runbook_url": "https://efe.example.com/r"},
      }],
    }]},
  }
}

test_deny_alert_invalid_severity if {
  count(deny) > 0 with input as {
    "kind": "PrometheusRule",
    "metadata": {"name": "alerts"},
    "spec": {"groups": [{
      "name": "g",
      "rules": [{
        "alert": "MyAlert",
        "expr": "up == 0",
        "labels": {"severity": "emergency"},
        "annotations": {"summary": "...", "runbook_url": "https://efe.example.com/r"},
      }],
    }]},
  }
}

test_deny_alert_no_summary if {
  count(deny) > 0 with input as {
    "kind": "PrometheusRule",
    "metadata": {"name": "alerts"},
    "spec": {"groups": [{
      "name": "g",
      "rules": [{
        "alert": "MyAlert",
        "expr": "up == 0",
        "labels": {"severity": "warning"},
        "annotations": {"runbook_url": "https://efe.example.com/r"},
      }],
    }]},
  }
}
