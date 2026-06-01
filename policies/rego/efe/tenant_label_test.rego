# PR #15 — Rego unit tests : tenant_label.rego.
# Per-rule package : isolates THIS rule's denies from neighbours'.

package efe.tenant_label

import future.keywords.if

# ─── allow cases ─────────────────────────────────────────────────────

test_allow_deployment_with_tenant if {
  count(deny) == 0 with input as {
    "apiVersion": "apps/v1",
    "kind": "Deployment",
    "metadata": {
      "name": "controller",
      "labels": {"efe.io/tenant": "kovant"},
    },
    "spec": {"template": {"spec": {"containers": []}}},
  }
}

test_allow_priorityclass_without_tenant if {
  # PriorityClass is cluster-scoped → exempt.
  count(deny) == 0 with input as {
    "apiVersion": "scheduling.k8s.io/v1",
    "kind": "PriorityClass",
    "metadata": {"name": "high"},
    "value": 100000,
  }
}

test_allow_clusterrole_without_tenant if {
  count(deny) == 0 with input as {
    "apiVersion": "rbac.authorization.k8s.io/v1",
    "kind": "ClusterRole",
    "metadata": {"name": "viewer"},
  }
}

# ─── deny cases ──────────────────────────────────────────────────────

test_deny_deployment_missing_tenant if {
  count(deny) > 0 with input as {
    "apiVersion": "apps/v1",
    "kind": "Deployment",
    "metadata": {
      "name": "controller",
      "labels": {"app": "controller"},      # NO efe.io/tenant
    },
  }
}

test_deny_deployment_no_labels if {
  count(deny) > 0 with input as {
    "apiVersion": "apps/v1",
    "kind": "Deployment",
    "metadata": {"name": "controller"},     # NO labels at all
  }
}

test_deny_service_empty_tenant if {
  count(deny) > 0 with input as {
    "apiVersion": "v1",
    "kind": "Service",
    "metadata": {
      "name": "controller",
      "labels": {"efe.io/tenant": ""},      # empty string
    },
  }
}

test_deny_configmap_missing_tenant if {
  count(deny) > 0 with input as {
    "apiVersion": "v1",
    "kind": "ConfigMap",
    "metadata": {"name": "manifest", "labels": {}},
    "data": {"manifest.yaml": "..."},
  }
}
