"""Protobuf ↔ Python contract bridge for M12.

This package is the **only** layer allowed to touch ``efe.v1`` wire
formats. The rest of the platform passes :mod:`efe_swarm_integration.types`
dataclasses to :class:`ContractAdapter` methods.
"""

from __future__ import annotations

from efe_swarm_integration.contract.adapter_admin import (
    admin_cone_handle_from_dict,
    admin_register_cone_to_dict,
)
from efe_swarm_integration.contract.adapter_result import (
    parse_describe_spec_reply,
    parse_execute_response,
    parse_stream_chunk,
)
from efe_swarm_integration.contract.adapter_spec import (
    ContractAdapter,
    serialise_execute_request,
    serialise_swarm_spec,
)
from efe_swarm_integration.contract.snapshot import (
    CONTRACT_SNAPSHOT,
    assert_snapshot_stable,
)
from efe_swarm_integration.contract.validator import (
    validate_execute_request,
    validate_swarm_spec,
)

__all__ = [
    "CONTRACT_SNAPSHOT",
    "ContractAdapter",
    "admin_cone_handle_from_dict",
    "admin_register_cone_to_dict",
    "assert_snapshot_stable",
    "parse_describe_spec_reply",
    "parse_execute_response",
    "parse_stream_chunk",
    "serialise_execute_request",
    "serialise_swarm_spec",
    "validate_execute_request",
    "validate_swarm_spec",
]
