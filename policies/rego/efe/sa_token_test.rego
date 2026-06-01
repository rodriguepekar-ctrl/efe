# PR #15 — Rego unit tests : sa_token.rego.

package efe.sa_token

import future.keywords.if

# Data fixture for the test : hook patterns.
_data := {"runtime_pods": {
  "hook_patterns": [
    "^.*-efe-meta-swarm-hook-pre-upgrade-snapshot$",
    "^.*-efe-meta-swarm-hook-post-upgrade-sighup$",
  ],
  "exempt_names": [],
}}

# ─── allow cases ─────────────────────────────────────────────────────

test_allow_runtime_with_explicit_false if {
  count(deny) == 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "tst-efe-meta-swarm-controller"},
    "spec": {"template": {"spec": {
      "automountServiceAccountToken": false,
      "containers": [{"name": "main", "image": "ghcr.io/efe-ai/srv:0.3.0"}],
    }}},
  }
  with data as _data
}

test_allow_hook_job_with_token_true if {
  # Note : Job is NOT in runtime_kinds (only Deployment/StatefulSet/
  # DaemonSet/Pod are).  Jobs are runtime-exempt by KIND.  So even
  # without name matching, this passes.
  count(deny) == 0 with input as {
    "kind": "Job",
    "metadata": {"name": "tst-efe-meta-swarm-hook-pre-upgrade-snapshot"},
    "spec": {"template": {"spec": {
      "automountServiceAccountToken": true,
      "containers": [{"name": "snap", "image": "bitnami/kubectl:1.28"}],
    }}},
  }
  with data as _data
}

# ─── deny cases ──────────────────────────────────────────────────────

test_deny_runtime_token_default if {
  count(deny) > 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "tst-efe-meta-swarm-controller"},
    "spec": {"template": {"spec": {
      "containers": [{"name": "main", "image": "ghcr.io/efe-ai/srv:0.3.0"}],
    }}},
  }
  with data as _data
}

test_deny_runtime_token_explicit_true if {
  count(deny) > 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "tst-efe-meta-swarm-participant-supplier-scoring"},
    "spec": {"template": {"spec": {
      "automountServiceAccountToken": true,
      "containers": [{"name": "main", "image": "ghcr.io/efe-ai/srv:0.3.0"}],
    }}},
  }
  with data as _data
}

test_deny_pod_token_default if {
  count(deny) > 0 with input as {
    "kind": "Pod",
    "metadata": {"name": "test-connection"},
    "spec": {
      "containers": [{"name": "p", "image": "fullstorydev/grpcurl:v1.9.1"}],
    },
  }
  with data as _data
}

test_allow_exempt_by_exact_name if {
  count(deny) == 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "explicitly-exempt-pod"},
    "spec": {"template": {"spec": {
      "containers": [{"name": "main", "image": "ghcr.io/efe-ai/srv:0.3.0"}],
    }}},
  }
  with data as {"runtime_pods": {
    "hook_patterns": [],
    "exempt_names": ["explicitly-exempt-pod"],
  }}
}
