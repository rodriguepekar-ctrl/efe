"""EFE bundle introspection (M18 D47).

Pure-Python helpers for the bundle/deploy MCP tools :
  - `oci.OCIRegistryClient` : read-only OCI Distribution v1.1 client.
  - `manifest.parse_manifest` : OCI manifest + EFE annotations parser.
  - `verify.verify_ed25519_signature` : signature verification.
  - `compat.check_compatibility` : EFE-server vs bundle compatibility.
  - `sbom.parse_sbom` : minimal CycloneDX 1.5 parser (extended in D48).

NEVER invokes subprocess / cosign / oras / docker. The MCP server
remains a pure-Python read+verify boundary (cf. M18 architecture §2.1).
"""

from .compat import CompatibilityCheck, check_compatibility
from .manifest import EFEBundleManifest, parse_manifest
from .registry_arg import InvalidRegistryArg, split_registry
from .oci import (
    OCIAccessForbidden,
    OCIArtifactNotFound,
    OCIRegistryClient,
    OCIRegistryError,
    OCIRegistryNotInAllowlist,
    check_allowlist,
    check_host_in_allowlist,
    parse_registry_allowlist,
)
from .sbom import (
    Component,
    ParsedSBOM,
    Vulnerability,
    filter_by_severity,
    parse_sbom,
)
from .slsa import (
    ParsedProvenance,
    SLSAParseError,
    SLSASubject,
    infer_slsa_level,
    is_builder_trusted,
    parse_builder_allowlist,
    parse_provenance,
    validate_subject_digest,
)
from .verify import VerificationResult, compute_key_id, verify_ed25519_signature

__all__ = [
    # OCI
    "OCIRegistryClient",
    "OCIRegistryError",
    "OCIRegistryNotInAllowlist",
    "OCIAccessForbidden",
    "OCIArtifactNotFound",
    "parse_registry_allowlist",
    "check_allowlist",
    "check_host_in_allowlist",
    # Manifest
    "EFEBundleManifest",
    "parse_manifest",
    # Verify
    "VerificationResult",
    "verify_ed25519_signature",
    "compute_key_id",
    # Compat
    "CompatibilityCheck",
    "check_compatibility",
    # Registry-arg helper
    "InvalidRegistryArg",
    "split_registry",
    # SBOM
    "ParsedSBOM",
    "Component",
    "Vulnerability",
    "parse_sbom",
    "filter_by_severity",
    # SLSA
    "ParsedProvenance",
    "SLSAParseError",
    "SLSASubject",
    "parse_provenance",
    "validate_subject_digest",
    "is_builder_trusted",
    "parse_builder_allowlist",
    "infer_slsa_level",
]
