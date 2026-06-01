# Tests for metric_label_lowercase.rego — PR #17 T49.

package efe.metric_label_lowercase

valid_deployment := {
    "apiVersion": "apps/v1",
    "kind": "Deployment",
    "metadata": {
        "namespace": "efe",
        "name": "controller",
        "labels": {
            "app.kubernetes.io/part-of": "efe-meta-swarm",
            "efe.io/participant-id": "supplier-scoring",
        },
    },
    "spec": {"template": {"spec": {"containers": [{
        "env": [
            {"name": "EFE_META_SWARM_TLS_ALLOWED_PEER_SANS", "value": "participant-1.efe.svc"},
        ],
    }]}}},
}

test_valid_deployment_passes {
    count(deny) == 0 with input as valid_deployment
    count(warn) == 0 with input as valid_deployment
}

test_uppercase_participant_id_denied {
    bad := json.patch(valid_deployment, [
        {"op": "replace", "path": "/metadata/labels/efe.io~1participant-id", "value": "Supplier-Scoring"},
    ])
    count(deny) > 0 with input as bad
}

test_email_like_participant_id_denied {
    bad := json.patch(valid_deployment, [
        {"op": "replace", "path": "/metadata/labels/efe.io~1participant-id", "value": "alice@example.com"},
    ])
    count(deny) > 0 with input as bad
}

test_uppercase_san_in_env_denied {
    bad := json.patch(valid_deployment, [
        {"op": "replace",
         "path": "/spec/template/spec/containers/0/env/0/value",
         "value": "Participant-1.efe.svc"},
    ])
    count(deny) > 0 with input as bad
}

test_short_participant_id_warns {
    bad := json.patch(valid_deployment, [
        {"op": "replace", "path": "/metadata/labels/efe.io~1participant-id", "value": "a"},
    ])
    count(warn) > 0 with input as bad
}

test_non_meta_swarm_deployment_ignored {
    other := json.patch(valid_deployment, [
        {"op": "replace", "path": "/metadata/labels/app.kubernetes.io~1part-of", "value": "other"},
        {"op": "replace", "path": "/metadata/labels/efe.io~1participant-id", "value": "INVALID"},
    ])
    count(deny) == 0 with input as other
}
