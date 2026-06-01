# PR #15 — Rego unit tests : service_appprotocol.rego.

package efe.service_appprotocol

import future.keywords.if

test_allow_service_with_appprotocol if {
  count(deny) == 0 with input as {
    "kind": "Service",
    "metadata": {"name": "controller"},
    "spec": {"ports": [
      {"name": "grpc", "port": 50080, "protocol": "TCP", "appProtocol": "grpc"},
      {"name": "metrics", "port": 9091, "protocol": "TCP", "appProtocol": "http"},
    ]},
  }
}

test_deny_port_missing_appprotocol if {
  count(deny) > 0 with input as {
    "kind": "Service",
    "metadata": {"name": "controller"},
    "spec": {"ports": [
      {"name": "grpc", "port": 50080, "protocol": "TCP"},
    ]},
  }
}

test_deny_one_of_many_missing_appprotocol if {
  count(deny) > 0 with input as {
    "kind": "Service",
    "metadata": {"name": "controller"},
    "spec": {"ports": [
      {"name": "grpc", "port": 50080, "protocol": "TCP", "appProtocol": "grpc"},
      {"name": "metrics", "port": 9091, "protocol": "TCP"},
    ]},
  }
}

test_deny_empty_appprotocol if {
  count(deny) > 0 with input as {
    "kind": "Service",
    "metadata": {"name": "controller"},
    "spec": {"ports": [
      {"name": "grpc", "port": 50080, "protocol": "TCP", "appProtocol": ""},
    ]},
  }
}
