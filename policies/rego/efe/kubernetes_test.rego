# PR #15 — Rego unit tests : kubernetes.rego (CIS top-10 aggregator).

package efe.kubernetes

import future.keywords.if

test_allow_explicit_serviceaccount if {
  count(deny) == 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "ctrl", "labels": {"efe.io/tenant": "kovant"}},
    "spec": {"template": {"spec": {
      "serviceAccountName": "ctrl-sa",
      "containers": [{
        "name": "main",
        "image": "ghcr.io/efe-ai/srv:0.3.0",
        "resources": {
          "requests": {"cpu": "500m", "memory": "512Mi"},
          "limits":   {"cpu": "2", "memory": "2Gi"},
        },
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

test_allow_meta_swarm_controller_hpa_min_max_1 if {
  count(deny) == 0 with input as {
    "kind": "HorizontalPodAutoscaler",
    "metadata": {"name": "tst-efe-meta-swarm-controller"},
    "spec": {"minReplicas": 1, "maxReplicas": 1, "scaleTargetRef": {}},
  }
}

test_deny_default_serviceaccount if {
  count(deny) > 0 with input as {
    "kind": "Deployment",
    "metadata": {"name": "ctrl"},
    "spec": {"template": {"spec": {
      "containers": [{"name": "main", "image": "ghcr.io/efe-ai/srv:0.3.0"}],
    }}},
  }
}

test_deny_rolebinding_to_default_sa if {
  count(deny) > 0 with input as {
    "kind": "RoleBinding",
    "metadata": {"name": "rb"},
    "subjects": [
      {"kind": "ServiceAccount", "name": "default", "namespace": "efe"},
    ],
    "roleRef": {"kind": "Role", "name": "r", "apiGroup": "rbac.authorization.k8s.io"},
  }
}

test_deny_loadbalancer_without_annotation if {
  count(deny) > 0 with input as {
    "kind": "Service",
    "metadata": {"name": "public", "labels": {"efe.io/tenant": "kovant"}},
    "spec": {"type": "LoadBalancer", "ports": [{"port": 443, "appProtocol": "https"}]},
  }
}

test_deny_hpa_min_gt_max if {
  count(deny) > 0 with input as {
    "kind": "HorizontalPodAutoscaler",
    "metadata": {"name": "h"},
    "spec": {"minReplicas": 5, "maxReplicas": 2, "scaleTargetRef": {}},
  }
}

test_deny_meta_swarm_controller_hpa_min_not_1 if {
  count(deny) > 0 with input as {
    "kind": "HorizontalPodAutoscaler",
    "metadata": {"name": "tst-efe-meta-swarm-controller"},
    "spec": {"minReplicas": 2, "maxReplicas": 5, "scaleTargetRef": {}},
  }
}

test_deny_meta_swarm_controller_hpa_max_not_1 if {
  count(deny) > 0 with input as {
    "kind": "HorizontalPodAutoscaler",
    "metadata": {"name": "tst-efe-meta-swarm-controller"},
    "spec": {"minReplicas": 1, "maxReplicas": 3, "scaleTargetRef": {}},
  }
}

test_deny_pdb_no_bound if {
  count(deny) > 0 with input as {
    "kind": "PodDisruptionBudget",
    "metadata": {"name": "pdb"},
    "spec": {"selector": {}},
  }
}

test_deny_empty_configmap if {
  count(deny) > 0 with input as {
    "kind": "ConfigMap",
    "metadata": {"name": "empty"},
  }
}

test_deny_job_restart_always if {
  count(deny) > 0 with input as {
    "kind": "Job",
    "metadata": {"name": "j"},
    "spec": {"template": {"spec": {
      "restartPolicy": "Always",
      "containers": [{"name": "c", "image": "ghcr.io/efe-ai/srv:0.3.0"}],
    }}},
  }
}
