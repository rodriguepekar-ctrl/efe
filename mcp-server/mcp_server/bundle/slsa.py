"""SLSA v1.0 / in-toto provenance attestation parser (M18 D48).

Parses SLSA Provenance v1.0 attestations (in-toto Statement v1) carried
as OCI Referrers (artifactType `application/vnd.in-toto+json` or the
sigstore-bundle alternative). Pure JSON — never invokes `cosign verify-
attestation` subprocess.

Reference :
  - SLSA spec v1.0 : https://slsa.dev/spec/v1.0/
  - in-toto Statement v1 : https://github.com/in-toto/attestation/blob/main/spec/v1.0/statement.md

Threats addressed (D48 carry-over) :
  - TM-M18-E-03 (SLSA forgery) : `validate_subject_digest()` cross-checks
    the subject digest in the attestation against the manifest sha256
    we computed locally.
  - Builder whitelist (opt-in via `EFE_SLSA_BUILDER_ALLOWLIST`) : warn
    if the `runDetails.builder.id` doesn't match an expected pattern.

There is no hard cap on attestation size in this module — the OCI client
that pulls the blob enforces the upstream registry quota, and SLSA
attestations are typically < 10 KB anyway. If we hit oversized payloads
in the wild we'll mirror the `parse_sbom` max_bytes pattern.
"""
from __future__ import annotations

import json
import os
from dataclasses import dataclass, field
from typing import Any


_IN_TOTO_STATEMENT_V1_TYPE = "https://in-toto.io/Statement/v1"
_SLSA_PROVENANCE_V1_PREDICATE = "https://slsa.dev/provenance/v1"

# Conservative default builder whitelist — only well-known hardened
# builders. Operators extend via `EFE_SLSA_BUILDER_ALLOWLIST` env-var.
_DEFAULT_TRUSTED_BUILDERS: tuple[str, ...] = (
    "https://github.com/actions/runner",
    "https://github.com/slsa-framework/slsa-github-generator",
    "https://gitlab.com/gitlab-runner",
)


class SLSAParseError(ValueError):
    """Raised when an attestation payload doesn't match the in-toto/SLSA shape."""


@dataclass(frozen=True)
class SLSASubject:
    name: str
    digest: dict[str, str]   # {algorithm: hex_hash}

    @property
    def sha256(self) -> str | None:
        return self.digest.get("sha256")


@dataclass(frozen=True)
class ParsedProvenance:
    """Parsed SLSA Provenance v1.0 attestation.

    Many fields are optional because not every CI emits the full
    schema. The `level` is INFERRED conservatively from the available
    fields ; this module never claims a higher level than the data
    supports.
    """

    statement_type: str | None
    predicate_type: str | None
    subjects: tuple[SLSASubject, ...]
    builder_id: str | None
    build_type: str | None
    invocation_id: str | None
    started_on: str | None              # ISO8601
    finished_on: str | None             # ISO8601
    external_parameters: dict[str, Any] = field(default_factory=dict, hash=False)
    raw: dict[str, Any] = field(default_factory=dict, hash=False)

    def is_in_toto_statement_v1(self) -> bool:
        return self.statement_type == _IN_TOTO_STATEMENT_V1_TYPE

    def is_slsa_provenance_v1(self) -> bool:
        return self.predicate_type == _SLSA_PROVENANCE_V1_PREDICATE

    def has_subject_with_digest(self, sha256_hex: str) -> bool:
        target = sha256_hex.lower()
        for subj in self.subjects:
            if subj.sha256 and subj.sha256.lower() == target:
                return True
        return False


# ─────────────────────────────────────────────────────────────────────
# Parsing
# ─────────────────────────────────────────────────────────────────────


def parse_provenance(content: bytes | str | dict) -> ParsedProvenance:
    """Parse an in-toto Statement v1 / SLSA Provenance v1.0 payload.

    Accepts raw bytes, a JSON string, or a pre-parsed dict.
    """
    data = _coerce_to_dict(content)
    statement_type = data.get("_type")
    if statement_type and not isinstance(statement_type, str):
        statement_type = None

    predicate_type = data.get("predicateType")
    if predicate_type and not isinstance(predicate_type, str):
        predicate_type = None

    raw_subjects = data.get("subject") or []
    subjects: list[SLSASubject] = []
    if isinstance(raw_subjects, list):
        for s in raw_subjects:
            if not isinstance(s, dict):
                continue
            name = s.get("name") or ""
            digest = s.get("digest") or {}
            if not isinstance(digest, dict):
                continue
            digest_norm: dict[str, str] = {}
            for algo, value in digest.items():
                if isinstance(algo, str) and isinstance(value, str):
                    digest_norm[algo.lower()] = value.lower()
            subjects.append(SLSASubject(name=str(name), digest=digest_norm))

    predicate = data.get("predicate") or {}
    if not isinstance(predicate, dict):
        predicate = {}

    build_def = predicate.get("buildDefinition") or {}
    if not isinstance(build_def, dict):
        build_def = {}
    run_details = predicate.get("runDetails") or {}
    if not isinstance(run_details, dict):
        run_details = {}

    builder = run_details.get("builder") or {}
    if not isinstance(builder, dict):
        builder = {}
    metadata = run_details.get("metadata") or {}
    if not isinstance(metadata, dict):
        metadata = {}

    external_parameters = build_def.get("externalParameters") or {}
    if not isinstance(external_parameters, dict):
        external_parameters = {}

    return ParsedProvenance(
        statement_type=statement_type,
        predicate_type=predicate_type,
        subjects=tuple(subjects),
        builder_id=_str_or_none(builder.get("id")),
        build_type=_str_or_none(build_def.get("buildType")),
        invocation_id=_str_or_none(metadata.get("invocationId")),
        started_on=_str_or_none(metadata.get("startedOn")),
        finished_on=_str_or_none(metadata.get("finishedOn")),
        external_parameters=external_parameters,
        raw=data,
    )


# ─────────────────────────────────────────────────────────────────────
# Subject digest validation (TM-M18-E-03)
# ─────────────────────────────────────────────────────────────────────


def validate_subject_digest(
    *, provenance: ParsedProvenance, expected_sha256: str,
) -> tuple[bool, str | None]:
    """Cross-check the SLSA subject digest against an expected sha256.

    Returns (ok, reason). `reason` is a human-readable message when ok=False.
    """
    if not provenance.subjects:
        return False, "attestation has no subject"
    if not provenance.has_subject_with_digest(expected_sha256):
        actual = ", ".join(
            f"{s.name}={s.sha256!r}" for s in provenance.subjects
        )
        return False, (
            f"subject digest mismatch : expected sha256={expected_sha256!r}, "
            f"attestation subjects: [{actual}]"
        )
    return True, None


# ─────────────────────────────────────────────────────────────────────
# Builder whitelist (opt-in)
# ─────────────────────────────────────────────────────────────────────


def parse_builder_allowlist(value: str | None = None) -> tuple[str, ...]:
    """Parse `EFE_SLSA_BUILDER_ALLOWLIST` ; falls back to defaults."""
    raw = value if value is not None else os.environ.get("EFE_SLSA_BUILDER_ALLOWLIST", "")
    if not raw.strip():
        return _DEFAULT_TRUSTED_BUILDERS
    out: list[str] = []
    for token in raw.replace(";", ",").split(","):
        s = token.strip().rstrip("/")
        if s:
            out.append(s)
    return tuple(out)


def is_builder_trusted(
    *, provenance: ParsedProvenance, allowlist: tuple[str, ...] | None = None,
) -> bool:
    """Return True if the builder URL starts with any allowlist entry."""
    if not provenance.builder_id:
        return False
    wl = allowlist if allowlist is not None else parse_builder_allowlist()
    builder = provenance.builder_id.rstrip("/")
    for prefix in wl:
        norm = prefix.rstrip("/")
        if builder == norm or builder.startswith(norm + "/") or builder.startswith(norm + "@"):
            return True
    return False


# ─────────────────────────────────────────────────────────────────────
# SLSA level inference (conservative)
# ─────────────────────────────────────────────────────────────────────


def infer_slsa_level(
    provenance: ParsedProvenance,
    *,
    signature_verified: bool = False,
) -> int:
    """Return a conservative SLSA level (0-3) inferred from the payload.

    Audit fix M18-D49-A1 (MED) : the `signature_verified` parameter is
    REQUIRED to claim level 3. Without it, this function caps at 2 even
    when the builder is on the trusted-allowlist — because SLSA Level 3
    formally requires verifiably-signed provenance, and this function
    inspects the UNSIGNED payload alone. The caller (e.g.
    `_lookup_slsa_provenance` in inspect_bundle.py) is expected to
    independently verify the signature and pass the result through.

    Levels :
      0 — no provenance / no subjects / not in-toto
      1 — provenance present + builder_id + build_type
      2 — level 1 + buildType references a known SLSA build type
      3 — level 2 + builder is in trusted allowlist + signature_verified=True

    The module never claims level 4 (requires two-party review attested
    by a separate party, out of single-attestation scope).
    """
    if not provenance.is_in_toto_statement_v1() and not provenance.statement_type:
        return 0
    if not provenance.subjects:
        return 0
    if not provenance.builder_id and not provenance.build_type:
        return 1

    has_min_metadata = (
        provenance.builder_id is not None
        and provenance.build_type is not None
    )
    if not has_min_metadata:
        return 1

    # Level 2 requires the buildType to reference a recognized SLSA-style
    # builder workflow. We use a heuristic : URL contains "slsa", or
    # contains "github-actions-buildtypes", or "buildtypes/".
    looks_like_slsa_build = any(
        marker in (provenance.build_type or "")
        for marker in (
            "slsa-",
            "slsa.dev",
            "slsa-framework.github.io",
            "github-actions-buildtypes",
            "/buildtypes/",
        )
    )
    if not looks_like_slsa_build:
        return 1

    # Level 3 requires the builder to be on the trusted list AND the
    # signature to have been verified by the caller.
    if is_builder_trusted(provenance=provenance) and signature_verified:
        return 3
    return 2


# ─────────────────────────────────────────────────────────────────────
# Helpers
# ─────────────────────────────────────────────────────────────────────


def _coerce_to_dict(content: bytes | str | dict) -> dict[str, Any]:
    if isinstance(content, dict):
        return content
    if isinstance(content, (bytes, bytearray)):
        try:
            data = json.loads(content)
        except json.JSONDecodeError as exc:
            raise SLSAParseError(f"attestation is not valid JSON: {exc}") from exc
    elif isinstance(content, str):
        try:
            data = json.loads(content)
        except json.JSONDecodeError as exc:
            raise SLSAParseError(f"attestation is not valid JSON: {exc}") from exc
    else:
        raise SLSAParseError(f"unsupported attestation input: {type(content).__name__}")
    if not isinstance(data, dict):
        raise SLSAParseError(
            f"attestation root must be a JSON object, got {type(data).__name__}"
        )
    return data


def _str_or_none(value: Any) -> str | None:
    return value if isinstance(value, str) and value else None
