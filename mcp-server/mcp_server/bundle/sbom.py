"""CycloneDX 1.4/1.5 SBOM parsing — pure JSON, no third-party dep.

Minimum surface for D47 (counts + filtered listing). D48 may extend
with full vulnerability remediation context.
"""
from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Iterable


# ─────────────────────────────────────────────────────────────────────
# Data classes
# ─────────────────────────────────────────────────────────────────────


@dataclass(frozen=True)
class Component:
    name: str
    version: str | None
    type: str            # library | application | container | data | …
    purl: str | None     # package URL (e.g. pkg:pypi/torch@2.1.0)


@dataclass(frozen=True)
class Vulnerability:
    id: str
    severity: str        # LOW | MEDIUM | HIGH | CRITICAL | UNKNOWN
    affects: tuple[str, ...]


@dataclass(frozen=True)
class ParsedSBOM:
    spec_version: str
    components: tuple[Component, ...]
    vulnerabilities: tuple[Vulnerability, ...]
    raw: dict = field(default_factory=dict, hash=False)

    @property
    def components_count(self) -> int:
        return len(self.components)

    @property
    def vulnerabilities_count(self) -> int:
        return len(self.vulnerabilities)

    def by_severity(self) -> dict[str, int]:
        out: dict[str, int] = {}
        for v in self.vulnerabilities:
            out[v.severity] = out.get(v.severity, 0) + 1
        return out

    def dependencies_summary(self) -> dict[str, int]:
        """Bucket components by `type` for the inspect_bundle summary."""
        out: dict[str, int] = {}
        for c in self.components:
            t = c.type or "unknown"
            out[t] = out.get(t, 0) + 1
        return out


# ─────────────────────────────────────────────────────────────────────
# Errors
# ─────────────────────────────────────────────────────────────────────


class SBOMParseError(ValueError):
    """Raised when a payload doesn't look like a CycloneDX SBOM."""


# ─────────────────────────────────────────────────────────────────────
# Parsing
# ─────────────────────────────────────────────────────────────────────


_SUPPORTED_SPEC_VERSIONS = ("1.4", "1.5")
_SEVERITY_ORDER = ("LOW", "MEDIUM", "HIGH", "CRITICAL")

# Audit fix M18-D47-A3 (MED) : hard cap on SBOM input size to prevent
# OOM via a malicious or mis-configured registry serving a multi-GB
# CycloneDX file. 10 MB is generous (real-world EFE bundle SBOMs are
# < 200 KB).
DEFAULT_MAX_SBOM_BYTES = 10 * 1024 * 1024


def parse_sbom(
    content: bytes | str | dict,
    *,
    max_bytes: int = DEFAULT_MAX_SBOM_BYTES,
) -> ParsedSBOM:
    """Parse a CycloneDX 1.4/1.5 SBOM.

    Accepts raw bytes, a JSON string, or a pre-parsed dict.
    Inputs larger than `max_bytes` are rejected (TM-M18-D-01 mitigation).
    """
    if isinstance(content, (bytes, bytearray)):
        if len(content) > max_bytes:
            raise SBOMParseError(
                f"SBOM exceeds max_bytes limit of {max_bytes} "
                f"(got {len(content)} bytes)"
            )
        try:
            data = json.loads(content)
        except json.JSONDecodeError as exc:
            raise SBOMParseError(f"SBOM is not valid JSON: {exc}") from exc
    elif isinstance(content, str):
        if len(content.encode("utf-8")) > max_bytes:
            raise SBOMParseError(
                f"SBOM exceeds max_bytes limit of {max_bytes}"
            )
        try:
            data = json.loads(content)
        except json.JSONDecodeError as exc:
            raise SBOMParseError(f"SBOM is not valid JSON: {exc}") from exc
    elif isinstance(content, dict):
        data = content
    else:
        raise SBOMParseError(f"unsupported SBOM input type: {type(content).__name__}")

    if data.get("bomFormat") != "CycloneDX":
        raise SBOMParseError(
            f"unexpected bomFormat: {data.get('bomFormat')!r} (expected 'CycloneDX')"
        )
    spec = str(data.get("specVersion", "")).strip()
    if spec and spec not in _SUPPORTED_SPEC_VERSIONS:
        # Don't fail outright — just record it and best-effort parse.
        # D48+ may extend support.
        pass

    raw_components = data.get("components") or []
    components = tuple(
        Component(
            name=str(c.get("name", "")),
            version=c.get("version"),
            type=str(c.get("type", "library")),
            purl=c.get("purl"),
        )
        for c in raw_components
        if isinstance(c, dict)
    )

    raw_vulns = data.get("vulnerabilities") or []
    vulns: list[Vulnerability] = []
    for v in raw_vulns:
        if not isinstance(v, dict):
            continue
        sev = "UNKNOWN"
        ratings = v.get("ratings") or []
        if isinstance(ratings, list) and ratings:
            first = ratings[0]
            if isinstance(first, dict):
                sev = (first.get("severity") or "UNKNOWN").upper()
        affects: list[str] = []
        for a in v.get("affects") or []:
            if isinstance(a, dict) and isinstance(a.get("ref"), str):
                affects.append(a["ref"])
        vulns.append(Vulnerability(
            id=str(v.get("id", "")),
            severity=sev,
            affects=tuple(affects),
        ))

    return ParsedSBOM(
        spec_version=spec,
        components=components,
        vulnerabilities=tuple(vulns),
        raw=data if isinstance(data, dict) else {},
    )


def filter_by_severity(
    vulns: Iterable[Vulnerability], min_severity: str,
) -> list[Vulnerability]:
    """Return vulns with severity ≥ `min_severity`."""
    if min_severity.upper() not in _SEVERITY_ORDER:
        return list(vulns)
    threshold = _SEVERITY_ORDER.index(min_severity.upper())
    keep = set(_SEVERITY_ORDER[threshold:])
    return [v for v in vulns if v.severity in keep]
