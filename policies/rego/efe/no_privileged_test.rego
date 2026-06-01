# PR #15 — Rego unit tests : no_privileged.rego.

package efe.no_privileged

import future.keywords.if

# ─── allow cases ─────────────────────────────────────────────────────

test_allow_unprivileged if {
  count(deny) == 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "ctrl"},
    "spec": {"template": {"spec": {"containers": [
      {
        "name": "main",
        "image": "ghcr.io/efe-ai/srv:0.3.0",
        "securityContext": {"privileged": false},
      },
    ]}}},
  }
}

test_allow_no_securitycontext if {
  # No securityContext → privileged defaults to false (per K8s).  This
  # rule only fires on EXPLICIT privileged: true.  security_context.rego
  # is the rule that catches missing securityContext.
  count(deny) == 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "ctrl"},
    "spec": {"template": {"spec": {"containers": [
      {"name": "main", "image": "ghcr.io/efe-ai/srv:0.3.0"},
    ]}}},
  }
}

# ─── deny cases ──────────────────────────────────────────────────────

test_deny_privileged_container if {
  count(deny) > 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "ctrl"},
    "spec": {"template": {"spec": {"containers": [
      {
        "name": "main",
        "image": "ghcr.io/efe-ai/srv:0.3.0",
        "securityContext": {"privileged": true},
      },
    ]}}},
  }
}

test_deny_privileged_initcontainer if {
  count(deny) > 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "ctrl"},
    "spec": {"template": {"spec": {
      "containers": [
        {"name": "main", "image": "ghcr.io/efe-ai/srv:0.3.0"},
      ],
      "initContainers": [
        {
          "name": "init",
          "image": "busybox:1.36",
          "securityContext": {"privileged": true},
        },
      ],
    }}},
  }
}

test_deny_dangerous_capability_SYS_ADMIN if {
  count(deny) > 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "ctrl"},
    "spec": {"template": {"spec": {"containers": [
      {
        "name": "main",
        "image": "ghcr.io/efe-ai/srv:0.3.0",
        "securityContext": {"capabilities": {"add": ["SYS_ADMIN"]}},
      },
    ]}}},
  }
}

test_deny_dangerous_capability_NET_RAW if {
  count(deny) > 0 with input as {
    "kind": "Pod",
    "metadata": {"name": "p"},
    "spec": {"containers": [
      {
        "name": "main",
        "image": "ghcr.io/efe-ai/srv:0.3.0",
        "securityContext": {"capabilities": {"add": ["NET_RAW"]}},
      },
    ]},
  }
}
