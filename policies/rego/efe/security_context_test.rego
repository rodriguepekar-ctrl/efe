# PR #15 — Rego unit tests : security_context.rego.

package efe.security_context

import future.keywords.if

# Helper : build a minimal Deployment with the given container security context.
_dep_with(sc) := {
  "kind": "Deployment",
  "metadata": {"name": "ctrl"},
  "spec": {"template": {"spec": {"containers": [
    {
      "name": "main",
      "image": "ghcr.io/efe-ai/srv:0.3.0",
      "securityContext": sc,
    },
  ]}}},
}

# ─── allow case ──────────────────────────────────────────────────────

test_allow_full_securitycontext if {
  count(deny) == 0 with input as _dep_with({
    "allowPrivilegeEscalation": false,
    "capabilities": {"drop": ["ALL"]},
    "readOnlyRootFilesystem": true,
    "runAsNonRoot": true,
    "runAsUser": 65532,
  })
}

# ─── deny cases ──────────────────────────────────────────────────────

test_deny_no_securitycontext_at_all if {
  count(deny) > 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "ctrl"},
    "spec": {"template": {"spec": {"containers": [
      {"name": "main", "image": "ghcr.io/efe-ai/srv:0.3.0"},
    ]}}},
  }
}

test_deny_capabilities_drop_missing_ALL if {
  count(deny) > 0 with input as _dep_with({
    "capabilities": {"drop": ["NET_RAW"]},   # missing ALL
    "runAsNonRoot": true,
    "readOnlyRootFilesystem": true,
    "allowPrivilegeEscalation": false,
  })
}

test_deny_capabilities_entirely_missing if {
  # Audit-pass regression guard : even when `capabilities` is undefined,
  # the deny must fire (previous code accessed `caps.drop` directly and
  # silently no-op'd).
  count(deny) > 0 with input as _dep_with({
    "runAsNonRoot": true,
    "readOnlyRootFilesystem": true,
    "allowPrivilegeEscalation": false,
    # NO capabilities at all.
  })
}

test_deny_runAsNonRoot_false if {
  count(deny) > 0 with input as _dep_with({
    "capabilities": {"drop": ["ALL"]},
    "runAsNonRoot": false,
    "readOnlyRootFilesystem": true,
    "allowPrivilegeEscalation": false,
  })
}

test_deny_runAsNonRoot_missing if {
  # Audit-pass regression guard : runAsNonRoot field missing entirely.
  count(deny) > 0 with input as _dep_with({
    "capabilities": {"drop": ["ALL"]},
    "readOnlyRootFilesystem": true,
    "allowPrivilegeEscalation": false,
  })
}

test_deny_allowPrivilegeEscalation_true if {
  count(deny) > 0 with input as _dep_with({
    "capabilities": {"drop": ["ALL"]},
    "runAsNonRoot": true,
    "readOnlyRootFilesystem": true,
    "allowPrivilegeEscalation": true,
  })
}

test_deny_writable_root_filesystem if {
  count(deny) > 0 with input as _dep_with({
    "capabilities": {"drop": ["ALL"]},
    "runAsNonRoot": true,
    "readOnlyRootFilesystem": false,
    "allowPrivilegeEscalation": false,
  })
}

test_deny_unrecognized_seccomp if {
  count(deny) > 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "ctrl"},
    "spec": {"template": {"spec": {
      "securityContext": {"seccompProfile": {"type": "Unconfined"}},
      "containers": [{
        "name": "main",
        "image": "ghcr.io/efe-ai/srv:0.3.0",
        "securityContext": {
          "allowPrivilegeEscalation": false,
          "capabilities": {"drop": ["ALL"]},
          "readOnlyRootFilesystem": true,
          "runAsNonRoot": true,
        },
      }],
    }}},
  }
}
