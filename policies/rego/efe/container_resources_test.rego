# PR #15 — Rego unit tests : container_resources.rego.

package efe.container_resources

import future.keywords.if

test_allow_complete_resources if {
  count(deny) == 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "ctrl"},
    "spec": {"template": {"spec": {"containers": [{
      "name": "main",
      "image": "ghcr.io/efe-ai/srv:0.3.0",
      "resources": {
        "requests": {"cpu": "500m", "memory": "512Mi"},
        "limits":   {"cpu": "2",    "memory": "2Gi"},
      },
    }]}}},
  }
}

test_deny_no_resources if {
  count(deny) > 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "ctrl"},
    "spec": {"template": {"spec": {"containers": [
      {"name": "main", "image": "ghcr.io/efe-ai/srv:0.3.0"},
    ]}}},
  }
}

test_deny_missing_requests if {
  count(deny) > 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "ctrl"},
    "spec": {"template": {"spec": {"containers": [{
      "name": "main",
      "image": "ghcr.io/efe-ai/srv:0.3.0",
      "resources": {"limits": {"cpu": "2", "memory": "2Gi"}},
    }]}}},
  }
}

test_deny_missing_limits if {
  count(deny) > 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "ctrl"},
    "spec": {"template": {"spec": {"containers": [{
      "name": "main",
      "image": "ghcr.io/efe-ai/srv:0.3.0",
      "resources": {"requests": {"cpu": "500m", "memory": "512Mi"}},
    }]}}},
  }
}

test_deny_requests_no_cpu if {
  count(deny) > 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "ctrl"},
    "spec": {"template": {"spec": {"containers": [{
      "name": "main",
      "image": "ghcr.io/efe-ai/srv:0.3.0",
      "resources": {
        "requests": {"memory": "512Mi"},
        "limits":   {"cpu": "2", "memory": "2Gi"},
      },
    }]}}},
  }
}

test_deny_limits_no_memory if {
  count(deny) > 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "ctrl"},
    "spec": {"template": {"spec": {"containers": [{
      "name": "main",
      "image": "ghcr.io/efe-ai/srv:0.3.0",
      "resources": {
        "requests": {"cpu": "500m", "memory": "512Mi"},
        "limits":   {"cpu": "2"},
      },
    }]}}},
  }
}
