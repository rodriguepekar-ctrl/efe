# PR #15 — Rego unit tests : no_host_namespace.rego.

package efe.no_host_namespace

import future.keywords.if

# ─── allow case ──────────────────────────────────────────────────────

test_allow_no_host_namespaces if {
  count(deny) == 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "ctrl"},
    "spec": {"template": {"spec": {
      "containers": [{
        "name": "main",
        "image": "ghcr.io/efe-ai/srv:0.3.0",
        "ports": [{"containerPort": 50080}],
      }],
    }}},
  }
}

# ─── deny cases ──────────────────────────────────────────────────────

test_deny_hostNetwork_true if {
  count(deny) > 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "ctrl"},
    "spec": {"template": {"spec": {
      "hostNetwork": true,
      "containers": [{"name": "main", "image": "ghcr.io/efe-ai/srv:0.3.0"}],
    }}},
  }
}

test_deny_hostPID_true if {
  count(deny) > 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "ctrl"},
    "spec": {"template": {"spec": {
      "hostPID": true,
      "containers": [{"name": "main", "image": "ghcr.io/efe-ai/srv:0.3.0"}],
    }}},
  }
}

test_deny_hostIPC_true if {
  count(deny) > 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "ctrl"},
    "spec": {"template": {"spec": {
      "hostIPC": true,
      "containers": [{"name": "main", "image": "ghcr.io/efe-ai/srv:0.3.0"}],
    }}},
  }
}

test_deny_hostPort_on_container_port if {
  count(deny) > 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "ctrl"},
    "spec": {"template": {"spec": {
      "containers": [{
        "name": "main",
        "image": "ghcr.io/efe-ai/srv:0.3.0",
        "ports": [{"containerPort": 50080, "hostPort": 50080}],
      }],
    }}},
  }
}
