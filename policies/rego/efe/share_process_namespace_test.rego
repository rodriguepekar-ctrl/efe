# PR #15 — Rego unit tests : share_process_namespace.rego.

package efe.share_process_namespace

import future.keywords.if

# ─── allow cases ─────────────────────────────────────────────────────

test_allow_no_share_no_sidecar if {
  count(deny) == 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "ctrl"},
    "spec": {"template": {"spec": {
      "containers": [{"name": "main", "image": "ghcr.io/efe-ai/srv:0.3.0"}],
    }}},
  }
}

test_allow_share_with_sidecar if {
  count(deny) == 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "participant-supplier-scoring"},
    "spec": {"template": {"spec": {
      "shareProcessNamespace": true,
      "containers": [
        {"name": "main", "image": "ghcr.io/efe-ai/srv:0.3.0"},
        {"name": "bundle-loader", "image": "gcr.io/google-containers/pause:3.9"},
      ],
    }}},
  }
}

# ─── deny cases ──────────────────────────────────────────────────────

test_deny_share_without_sidecar if {
  count(deny) > 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "ctrl"},
    "spec": {"template": {"spec": {
      "shareProcessNamespace": true,
      "containers": [{"name": "main", "image": "ghcr.io/efe-ai/srv:0.3.0"}],
    }}},
  }
}

test_deny_sidecar_without_share if {
  count(deny) > 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "participant-supplier-scoring"},
    "spec": {"template": {"spec": {
      "containers": [
        {"name": "main", "image": "ghcr.io/efe-ai/srv:0.3.0"},
        {"name": "bundle-loader", "image": "gcr.io/google-containers/pause:3.9"},
      ],
      # shareProcessNamespace deliberately missing
    }}},
  }
}
