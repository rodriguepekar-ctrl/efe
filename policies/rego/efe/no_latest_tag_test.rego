# PR #15 — Rego unit tests : no_latest_tag.rego.

package efe.no_latest_tag

import future.keywords.if

# ─── allow cases ─────────────────────────────────────────────────────

test_allow_pinned_tag if {
  count(deny) == 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "controller"},
    "spec": {"template": {"spec": {"containers": [
      {"name": "ctrl", "image": "ghcr.io/efe-ai/efe-meta-swarm-server:0.3.0"},
    ]}}},
  }
}

test_allow_digest_pin if {
  count(deny) == 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "controller"},
    "spec": {"template": {"spec": {"containers": [
      {"name": "ctrl", "image": "ghcr.io/efe-ai/srv@sha256:abc123def456"},
    ]}}},
  }
}

test_allow_pod_with_pinned_image if {
  count(deny) == 0 with input as {
    "kind": "Pod",
    "metadata": {"name": "probe"},
    "spec": {"containers": [
      {"name": "p", "image": "fullstorydev/grpcurl:v1.9.1"},
    ]},
  }
}

# ─── deny cases ──────────────────────────────────────────────────────

test_deny_latest_tag if {
  count(deny) > 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "controller"},
    "spec": {"template": {"spec": {"containers": [
      {"name": "ctrl", "image": "ghcr.io/efe-ai/srv:latest"},
    ]}}},
  }
}

test_deny_no_tag_at_all if {
  count(deny) > 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "controller"},
    "spec": {"template": {"spec": {"containers": [
      {"name": "ctrl", "image": "ghcr.io/efe-ai/srv"},     # no `:tag`
    ]}}},
  }
}

test_deny_initcontainer_latest if {
  count(deny) > 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "ctrl"},
    "spec": {"template": {"spec": {
      "containers": [
        {"name": "main", "image": "ghcr.io/efe-ai/srv:0.3.0"},
      ],
      "initContainers": [
        {"name": "init", "image": "busybox:latest"},
      ],
    }}},
  }
}

test_deny_cronjob_latest if {
  count(deny) > 0 with input as {
    "kind": "CronJob",
    "metadata": {"name": "rotate"},
    "spec": {"jobTemplate": {"spec": {"template": {"spec": {"containers": [
      {"name": "rotator", "image": "myregistry/rotator:latest"},
    ]}}}}},
  }
}
