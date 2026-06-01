"""MCP Resources (M17 D44).

Six URI types are exposed :
  - efe://catalogs/{name}                    catalog summary
  - efe://catalogs/{name}/versions/{v}       version detail
  - efe://catalogs/{name}/classes/{class}    class detail
  - efe://audit/recent[?limit=N&since=T]    audit log paginated
  - efe://quotas                             D12 quota usage
  - efe://samples/{id}                       sample catalog (M15 mirror)

Resources are read-only ; the URI is the request. Tenant scoping is
enforced by the backend (D11 zero-trust).
"""
from .registry import (
    RESOURCE_DESCRIPTORS,
    ResourceDescriptor,
    parse_resource_uri,
    read_resource,
)

__all__ = [
    "RESOURCE_DESCRIPTORS",
    "ResourceDescriptor",
    "parse_resource_uri",
    "read_resource",
]
