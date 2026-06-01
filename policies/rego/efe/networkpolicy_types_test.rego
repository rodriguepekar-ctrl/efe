# PR #15 — Rego unit tests : networkpolicy_types.rego.

package efe.networkpolicy_types

import future.keywords.if

test_allow_ingress_egress_explicit if {
  count(deny) == 0 with input as {
    "kind": "NetworkPolicy",
    "metadata": {"name": "deny-all"},
    "spec": {
      "podSelector": {},
      "policyTypes": ["Ingress", "Egress"],
    },
  }
}

test_allow_egress_only if {
  count(deny) == 0 with input as {
    "kind": "NetworkPolicy",
    "metadata": {"name": "controller-egress"},
    "spec": {
      "podSelector": {"matchLabels": {"app": "controller"}},
      "policyTypes": ["Egress"],
      "egress": [
        {"to": [{"podSelector": {"matchLabels": {"app": "participant"}}}],
         "ports": [{"protocol": "TCP", "port": 50080}]},
      ],
    },
  }
}

test_deny_missing_policytypes if {
  count(deny) > 0 with input as {
    "kind": "NetworkPolicy",
    "metadata": {"name": "policy"},
    "spec": {"podSelector": {}},
  }
}

test_deny_empty_policytypes if {
  count(deny) > 0 with input as {
    "kind": "NetworkPolicy",
    "metadata": {"name": "policy"},
    "spec": {
      "podSelector": {},
      "policyTypes": [],
    },
  }
}

test_deny_invalid_policytype_value if {
  count(deny) > 0 with input as {
    "kind": "NetworkPolicy",
    "metadata": {"name": "policy"},
    "spec": {
      "podSelector": {},
      "policyTypes": ["Outgoing"],
    },
  }
}

test_deny_egress_rule_without_egress_type if {
  count(deny) > 0 with input as {
    "kind": "NetworkPolicy",
    "metadata": {"name": "policy"},
    "spec": {
      "podSelector": {},
      "policyTypes": ["Ingress"],
      "egress": [
        {"to": [], "ports": [{"protocol": "TCP", "port": 443}]},
      ],
    },
  }
}

test_deny_ingress_rule_without_ingress_type if {
  count(deny) > 0 with input as {
    "kind": "NetworkPolicy",
    "metadata": {"name": "policy"},
    "spec": {
      "podSelector": {},
      "policyTypes": ["Egress"],
      "ingress": [
        {"from": [], "ports": [{"protocol": "TCP", "port": 50080}]},
      ],
    },
  }
}
