# Tests for tls_protocol_version.rego — PR #16 AD6 / AC30.

package efe.tls_protocol_version

test_tls13_allowed {
    obj := {
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {
            "namespace": "efe",
            "name": "controller",
            "labels": {"app.kubernetes.io/part-of": "efe-meta-swarm"},
        },
        "spec": {"template": {"spec": {"containers": [
            {"env": [{"name": "EFE_META_SWARM_TLS_MIN_PROTOCOL", "value": "1.3"}]}
        ]}}},
    }
    count(deny) == 0 with input as obj
    count(warn) == 0 with input as obj
}

test_tls12_allowed_with_warning {
    obj := {
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {
            "namespace": "efe",
            "name": "controller",
            "labels": {"app.kubernetes.io/part-of": "efe-meta-swarm"},
        },
        "spec": {"template": {"spec": {"containers": [
            {"env": [{"name": "EFE_META_SWARM_TLS_MIN_PROTOCOL", "value": "1.2"}]}
        ]}}},
    }
    # 1.2 is allowed (no deny) but warns.
    count(deny) == 0 with input as obj
    count(warn) > 0 with input as obj
}

test_tls10_denied {
    obj := {
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {
            "namespace": "efe",
            "name": "controller",
            "labels": {"app.kubernetes.io/part-of": "efe-meta-swarm"},
        },
        "spec": {"template": {"spec": {"containers": [
            {"env": [{"name": "EFE_META_SWARM_TLS_MIN_PROTOCOL", "value": "1.0"}]}
        ]}}},
    }
    count(deny) > 0 with input as obj
}

test_tls11_denied {
    obj := {
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {
            "namespace": "efe",
            "name": "controller",
            "labels": {"app.kubernetes.io/part-of": "efe-meta-swarm"},
        },
        "spec": {"template": {"spec": {"containers": [
            {"env": [{"name": "EFE_META_SWARM_TLS_MIN_PROTOCOL", "value": "1.1"}]}
        ]}}},
    }
    count(deny) > 0 with input as obj
}

test_non_meta_swarm_deployment_ignored {
    # A Deployment from a different sub-chart : the rule should be a no-op.
    obj := {
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {
            "namespace": "efe",
            "name": "some-other-chart",
            "labels": {"app.kubernetes.io/part-of": "some-other-chart"},
        },
        "spec": {"template": {"spec": {"containers": [
            {"env": [{"name": "EFE_META_SWARM_TLS_MIN_PROTOCOL", "value": "1.0"}]}
        ]}}},
    }
    count(deny) == 0 with input as obj
    count(warn) == 0 with input as obj
}

test_no_tls_env_is_silent {
    # When TLS is off (env var unset), the rule produces no output.
    obj := {
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {
            "namespace": "efe",
            "name": "controller",
            "labels": {"app.kubernetes.io/part-of": "efe-meta-swarm"},
        },
        "spec": {"template": {"spec": {"containers": [
            {"env": [{"name": "EFE_META_SWARM_DISCOVERY", "value": "k8s-dns"}]}
        ]}}},
    }
    count(deny) == 0 with input as obj
    count(warn) == 0 with input as obj
}
