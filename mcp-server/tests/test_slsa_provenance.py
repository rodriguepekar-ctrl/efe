"""SLSA v1.0 provenance parser pin tests (M18 D48)."""
from __future__ import annotations

import json

import pytest

from mcp_server.bundle.slsa import (
    SLSAParseError,
    infer_slsa_level,
    is_builder_trusted,
    parse_builder_allowlist,
    parse_provenance,
    validate_subject_digest,
)


_TRUSTED_BUILDER = "https://github.com/actions/runner/v2.319.0"
_SLSA_BUILD_TYPE = (
    "https://slsa-framework.github.io/github-actions-buildtypes/workflow/v1"
)
_GENERIC_BUILDER = "https://internal.example.com/jenkins/v9.9.9"


def _attestation(
    *,
    statement_type: str = "https://in-toto.io/Statement/v1",
    predicate_type: str = "https://slsa.dev/provenance/v1",
    subject_digest: str = "abc" + "0" * 61,
    builder_id: str | None = _TRUSTED_BUILDER,
    build_type: str | None = _SLSA_BUILD_TYPE,
) -> dict:
    body: dict = {
        "_type": statement_type,
        "subject": [{"name": "x", "digest": {"sha256": subject_digest}}],
        "predicateType": predicate_type,
        "predicate": {
            "buildDefinition": {
                "buildType": build_type,
                "externalParameters": {},
            },
            "runDetails": {
                "builder": {"id": builder_id} if builder_id else {},
                "metadata": {"invocationId": "run-42"},
            },
        },
    }
    return body


# ─────────────────────────────────────────────────────────────────────
# Parsing
# ─────────────────────────────────────────────────────────────────────


def test_parse_provenance_extracts_basic_fields():
    p = parse_provenance(_attestation())
    assert p.is_in_toto_statement_v1()
    assert p.is_slsa_provenance_v1()
    assert p.builder_id == _TRUSTED_BUILDER
    assert p.build_type == _SLSA_BUILD_TYPE
    assert p.invocation_id == "run-42"


def test_parse_provenance_accepts_bytes_input():
    raw = json.dumps(_attestation()).encode("utf-8")
    p = parse_provenance(raw)
    assert p.is_in_toto_statement_v1()


def test_parse_provenance_accepts_str_input():
    raw = json.dumps(_attestation())
    p = parse_provenance(raw)
    assert p.is_in_toto_statement_v1()


def test_parse_provenance_invalid_json_raises():
    with pytest.raises(SLSAParseError):
        parse_provenance(b"{not json")


def test_parse_provenance_non_object_root_raises():
    with pytest.raises(SLSAParseError):
        parse_provenance(b"[]")


def test_parse_provenance_missing_predicate_handled_gracefully():
    body = {"_type": "https://in-toto.io/Statement/v1", "subject": []}
    p = parse_provenance(body)
    assert p.builder_id is None
    assert p.build_type is None


def test_parse_provenance_normalises_digest_lowercase():
    body = _attestation(subject_digest="ABCDEF" + "0" * 58)
    p = parse_provenance(body)
    # digest is normalised to lowercase
    assert p.subjects[0].sha256.startswith("abcdef")


# ─────────────────────────────────────────────────────────────────────
# Subject digest validation (TM-M18-E-03 mitigation)
# ─────────────────────────────────────────────────────────────────────


def test_validate_subject_digest_match():
    sha = "deadbeef" + "0" * 56
    p = parse_provenance(_attestation(subject_digest=sha))
    ok, reason = validate_subject_digest(provenance=p, expected_sha256=sha)
    assert ok is True
    assert reason is None


def test_validate_subject_digest_mismatch_returns_reason():
    p = parse_provenance(_attestation(subject_digest="aa" + "0" * 62))
    ok, reason = validate_subject_digest(
        provenance=p, expected_sha256="bb" + "0" * 62,
    )
    assert ok is False
    assert "mismatch" in (reason or "")


def test_validate_subject_digest_no_subjects_fails():
    body = {"_type": "https://in-toto.io/Statement/v1", "subject": []}
    p = parse_provenance(body)
    ok, reason = validate_subject_digest(provenance=p, expected_sha256="x")
    assert ok is False


def test_validate_subject_digest_case_insensitive():
    sha = "AbCdEf" + "0" * 58
    p = parse_provenance(_attestation(subject_digest=sha))
    ok, _ = validate_subject_digest(
        provenance=p, expected_sha256=sha.upper(),
    )
    assert ok is True


# ─────────────────────────────────────────────────────────────────────
# Builder allowlist (TM-M18-E-03 mitigation)
# ─────────────────────────────────────────────────────────────────────


def test_parse_builder_allowlist_default_when_empty(monkeypatch):
    monkeypatch.delenv("EFE_SLSA_BUILDER_ALLOWLIST", raising=False)
    al = parse_builder_allowlist()
    assert "https://github.com/actions/runner" in al


def test_parse_builder_allowlist_custom_env(monkeypatch):
    monkeypatch.setenv(
        "EFE_SLSA_BUILDER_ALLOWLIST",
        "https://internal.example.com/jenkins, https://gitlab.com/runner",
    )
    al = parse_builder_allowlist()
    assert any("internal.example.com" in e for e in al)


def test_is_builder_trusted_default_github_actions():
    p = parse_provenance(_attestation(builder_id=_TRUSTED_BUILDER))
    assert is_builder_trusted(provenance=p) is True


def test_is_builder_trusted_unknown_builder_rejected():
    p = parse_provenance(_attestation(builder_id=_GENERIC_BUILDER))
    assert is_builder_trusted(provenance=p) is False


def test_is_builder_trusted_custom_allowlist():
    p = parse_provenance(_attestation(builder_id=_GENERIC_BUILDER))
    assert is_builder_trusted(
        provenance=p, allowlist=("https://internal.example.com/jenkins",),
    ) is True


def test_is_builder_trusted_no_builder_id_returns_false():
    p = parse_provenance(_attestation(builder_id=None))
    assert is_builder_trusted(provenance=p) is False


# ─────────────────────────────────────────────────────────────────────
# Level inference (conservative)
# ─────────────────────────────────────────────────────────────────────


def test_infer_level_zero_when_no_subjects():
    body = {"_type": "https://in-toto.io/Statement/v1", "subject": []}
    p = parse_provenance(body)
    assert infer_slsa_level(p) == 0


def test_infer_level_one_when_only_subject():
    body = _attestation(builder_id=None, build_type=None)
    p = parse_provenance(body)
    assert infer_slsa_level(p) == 1


def test_infer_level_two_when_slsa_build_type_but_unknown_builder():
    p = parse_provenance(_attestation(builder_id=_GENERIC_BUILDER))
    assert infer_slsa_level(p) == 2


def test_infer_level_three_when_trusted_builder_and_signed():
    """Audit fix M18-D49-A1 (MED) : level 3 now requires
    `signature_verified=True` in addition to a trusted builder."""
    p = parse_provenance(_attestation(builder_id=_TRUSTED_BUILDER))
    assert infer_slsa_level(p, signature_verified=True) == 3


def test_infer_level_capped_at_two_without_signature():
    """Without signature_verified=True, the function caps at 2 even
    when the builder is on the trusted allowlist (M18-D49-A1)."""
    p = parse_provenance(_attestation(builder_id=_TRUSTED_BUILDER))
    assert infer_slsa_level(p) == 2  # default signature_verified=False


def test_infer_level_capped_at_three():
    """We never claim level 4 from attestation alone (requires
    two-party review attestation)."""
    p = parse_provenance(_attestation(builder_id=_TRUSTED_BUILDER))
    assert infer_slsa_level(p, signature_verified=True) <= 3
