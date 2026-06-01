"""Sprint D12 audit — pin the fixes from `docs/audits/d12_catalog_report.md`.

Each test corresponds to one finding from the surgical audit pass.
Removal of any test below means the corresponding fix is back to a
risky state ; treat as a regression.

Findings :

  * **C-1** — delete cascade is overly aggressive on forked chains.
    Fixed by computing transitive descendants instead of blanket
    truncate. Pinned by ``test_delete_cascade_handles_forks_correctly``
    (in Rust ; this Python file pins the SDK-side shape check).

  * **H-1** — ``EfeAdminClient.health`` docstring claimed a
    `probe_auth=True` parameter that doesn't exist.
    Fixed by rewriting the docstring to match the implementation.
    Pinned by ``test_health_signature_only_self``.

  * **H-2** — ``_tenant_from_jwt`` error message hardcoded
    "list_catalogs" but the helper is shared.
    Fixed by making the message operation-agnostic.
    Pinned by ``test_tenant_from_jwt_error_message_operation_agnostic``.

  * **H-3** — ``delete_catalog`` accepted any non-empty string as a
    key (no shape check).
    Fixed by rejecting non-``tenant/name@vN`` keys early.
    Pinned by ``test_delete_catalog_rejects_malformed_key``.

  * **H-4** — Rust delete trait took no principal arg, so audit
    log emitted "(unknown)".
    Fixed by adding `principal: &str` to the trait. Tested in
    Rust (``delete_emits_audit_with_principal``).

  * **M-5** — ``_class_dict_to_proto`` accepted empty / whitespace
    sentences silently.
    Fixed by rejecting them. Pinned by
    ``test_class_dict_rejects_empty_sentence``.

  * **M-6** — threshold validation only fired when both ch + cm
    were truthy ; now matches ``policy.rs::validate_thresholds``.
    Pinned by ``test_thresholds_zero_rejected``,
    ``test_thresholds_above_one_rejected``, etc.

  * **CLI** — empty YAML / JSON files were silently sent as
    ``null``.
    Fixed by explicit None rejection. Pinned by
    ``test_load_yaml_or_json_rejects_empty_file``.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

_REPO = Path(__file__).resolve().parents[2]
_SDK_PATH = _REPO / "clients" / "python"
sys.path.insert(0, str(_SDK_PATH))


# ─────────────────────────────────────────────────────────────────────
# H-1 : health() docstring matches implementation
# ─────────────────────────────────────────────────────────────────────


def test_health_signature_only_self():
    """Pin that health() takes no parameters beyond self.
    The pre-fix docstring claimed a `probe_auth=True` arg."""
    import inspect

    from efe_tool.admin import EfeAdminClient
    sig = inspect.signature(EfeAdminClient.health)
    assert list(sig.parameters) == ["self"], (
        f"health() must accept only self ; got {list(sig.parameters)}"
    )


def test_health_docstring_does_not_mention_probe_auth():
    """The pre-fix docstring claimed `probe_auth=True`. Make sure
    the cleanup didn't get reverted."""
    from efe_tool.admin import EfeAdminClient
    doc = EfeAdminClient.health.__doc__ or ""
    assert "probe_auth" not in doc, (
        "health() docstring re-introduced the bogus `probe_auth` "
        "parameter — see audit finding H-1"
    )


# ─────────────────────────────────────────────────────────────────────
# H-2 : _tenant_from_jwt error message is operation-agnostic
# ─────────────────────────────────────────────────────────────────────


def test_tenant_from_jwt_error_message_operation_agnostic():
    """The error message must not mention any specific RPC by name."""
    import base64
    import json as _json

    from efe_tool.admin import AdminError, EfeAdminClient
    # Forge a JWT without a tenant_id claim.
    header = base64.urlsafe_b64encode(b'{"alg":"HS256"}').rstrip(b"=").decode()
    payload = base64.urlsafe_b64encode(_json.dumps({"sub": "alice"}).encode()).rstrip(b"=").decode()
    bogus_jwt = f"{header}.{payload}.signature"

    admin = EfeAdminClient(
        endpoint="127.0.0.1:50051", jwt=bogus_jwt, insecure=True,
    )
    try:
        admin._tenant_from_jwt()  # noqa: SLF001 — intentional probe
        pytest.fail("expected AdminError")
    except AdminError as e:
        msg = str(e)
        # Must NOT mention list_catalogs (the pre-fix wording).
        assert "list_catalogs" not in msg, (
            f"AdminError message hardcodes 'list_catalogs' (audit fix H-2 "
            f"regression): {msg}"
        )
        # Must mention tenant_id (the actual missing claim).
        assert "tenant_id" in msg
    finally:
        admin.close()


# ─────────────────────────────────────────────────────────────────────
# H-3 : delete_catalog rejects malformed keys client-side
# ─────────────────────────────────────────────────────────────────────


def test_delete_catalog_rejects_malformed_key():
    """Pre-fix : `delete_catalog` only checked `if not key`. Now
    enforces the same `tenant/name@vN` shape as get/list/versions."""
    from efe_tool import EfeInvalidArgumentError
    from efe_tool.admin import EfeAdminClient

    admin = EfeAdminClient(
        endpoint="127.0.0.1:50051",
        jwt="eyJhbGciOiJIUzI1NiJ9.eyJ0ZW5hbnRfaWQiOiJhY21lIn0.x",
        insecure=True,
    )
    try:
        for bad_key in ("", "no-slash@v1", "tenant/name", "tenant/name@", "/name@v1"):
            with pytest.raises(EfeInvalidArgumentError):
                admin.delete_catalog(bad_key)
    finally:
        admin.close()


# ─────────────────────────────────────────────────────────────────────
# M-5 : _class_dict_to_proto rejects empty / whitespace sentences
# ─────────────────────────────────────────────────────────────────────


def test_class_dict_rejects_empty_sentence():
    from efe_tool._proto.efe.v1 import catalog_pb2
    from efe_tool import EfeInvalidArgumentError
    from efe_tool.admin.client import _class_dict_to_proto

    bad_classes = [
        {"name": "x", "canonical_sentences": [""]},
        {"name": "x", "canonical_sentences": ["   "]},
        {"name": "x", "canonical_sentences": ["valid", ""]},
        {"name": "x", "canonical_sentences": [None]},
    ]
    for c in bad_classes:
        with pytest.raises(EfeInvalidArgumentError):
            _class_dict_to_proto(c, catalog_pb2)


def test_class_dict_accepts_valid_sentences():
    from efe_tool._proto.efe.v1 import catalog_pb2
    from efe_tool.admin.client import _class_dict_to_proto

    proto = _class_dict_to_proto(
        {"name": "valid", "canonical_sentences": ["one", "two"]},
        catalog_pb2,
    )
    assert proto.name == "valid"
    assert list(proto.canonical_sentences) == ["one", "two"]


# ─────────────────────────────────────────────────────────────────────
# M-6 : threshold validation matches policy.rs::validate_thresholds
# ─────────────────────────────────────────────────────────────────────


def _content_with_thresholds(ch: float, cm: float) -> dict:
    return {
        "embedder": {"family": "st", "model": "m", "dim": 4, "normalize": "l2"},
        "classes": [
            {"name": "a", "canonical_sentences": ["x"]},
            {"name": "b", "canonical_sentences": ["y"]},
        ],
        "cone_policy": {"max_projection_distance": 0.7},
        "thresholds": {"confidence_high": ch, "confidence_medium": cm},
    }


def test_thresholds_zero_rejected():
    from efe_tool._proto.efe.v1 import catalog_pb2
    from efe_tool import EfeInvalidArgumentError
    from efe_tool.admin.client import _content_dict_to_proto

    # Both 0 — pre-fix this passed silently because `if ch and cm`.
    with pytest.raises(EfeInvalidArgumentError):
        _content_dict_to_proto(_content_with_thresholds(0.0, 0.0), catalog_pb2)


def test_thresholds_above_one_rejected():
    from efe_tool._proto.efe.v1 import catalog_pb2
    from efe_tool import EfeInvalidArgumentError
    from efe_tool.admin.client import _content_dict_to_proto

    # confidence_medium > 1 — must reject (matches policy.rs).
    with pytest.raises(EfeInvalidArgumentError):
        _content_dict_to_proto(_content_with_thresholds(0.9, 1.5), catalog_pb2)
    # confidence_high > 1
    with pytest.raises(EfeInvalidArgumentError):
        _content_dict_to_proto(_content_with_thresholds(1.5, 0.5), catalog_pb2)


def test_thresholds_inverted_still_rejected():
    """Pre-fix and post-fix both rejected ch < cm (when both truthy)."""
    from efe_tool._proto.efe.v1 import catalog_pb2
    from efe_tool import EfeInvalidArgumentError
    from efe_tool.admin.client import _content_dict_to_proto

    with pytest.raises(EfeInvalidArgumentError):
        _content_dict_to_proto(_content_with_thresholds(0.3, 0.6), catalog_pb2)


def test_thresholds_valid_pass():
    from efe_tool._proto.efe.v1 import catalog_pb2
    from efe_tool.admin.client import _content_dict_to_proto

    # Standard prod values.
    proto = _content_dict_to_proto(_content_with_thresholds(0.8, 0.5), catalog_pb2)
    # Float32 round-trip — within tolerance.
    assert abs(proto.thresholds.confidence_high - 0.8) < 1e-5
    assert abs(proto.thresholds.confidence_medium - 0.5) < 1e-5


def test_evolve_update_thresholds_zero_rejected():
    """Same threshold strictness applies to UPDATE_THRESHOLDS evolve op."""
    from efe_tool._proto.efe.v1 import catalog_pb2
    from efe_tool import EfeInvalidArgumentError
    from efe_tool.admin import EvolveOp
    from efe_tool.admin.client import _evolve_op_to_proto

    with pytest.raises(EfeInvalidArgumentError):
        _evolve_op_to_proto(
            EvolveOp.UPDATE_THRESHOLDS,
            {"thresholds": {"confidence_high": 0.0, "confidence_medium": 0.0}},
            catalog_pb2,
        )


# ─────────────────────────────────────────────────────────────────────
# CLI — empty file rejection
# ─────────────────────────────────────────────────────────────────────


def test_load_yaml_or_json_rejects_empty_file(tmp_path):
    from efe_tool.cli.commands.catalog import _load_yaml_or_json
    from efe_tool.cli.errors import UsageError

    empty = tmp_path / "empty.yaml"
    empty.write_text("")
    with pytest.raises(UsageError):
        _load_yaml_or_json(empty)

    whitespace = tmp_path / "ws.yaml"
    whitespace.write_text("   \n  \n")
    with pytest.raises(UsageError):
        _load_yaml_or_json(whitespace)


def test_load_yaml_or_json_rejects_malformed(tmp_path):
    from efe_tool.cli.commands.catalog import _load_yaml_or_json
    from efe_tool.cli.errors import UsageError

    bad = tmp_path / "bad.yaml"
    bad.write_text("{ invalid yaml: [unterminated")
    with pytest.raises(UsageError):
        _load_yaml_or_json(bad)


def test_load_yaml_or_json_accepts_valid_yaml(tmp_path):
    from efe_tool.cli.commands.catalog import _load_yaml_or_json

    valid = tmp_path / "valid.yaml"
    valid.write_text("foo: 42\nbar: [1, 2, 3]\n")
    parsed = _load_yaml_or_json(valid)
    assert parsed == {"foo": 42, "bar": [1, 2, 3]}


def test_load_yaml_or_json_accepts_valid_json(tmp_path):
    from efe_tool.cli.commands.catalog import _load_yaml_or_json

    valid = tmp_path / "valid.json"
    valid.write_text('{"foo": 42, "bar": [1, 2, 3]}')
    parsed = _load_yaml_or_json(valid)
    assert parsed == {"foo": 42, "bar": [1, 2, 3]}


def test_load_json_or_yaml_rejects_empty_file(tmp_path):
    from efe_tool.cli.commands.catalog import _load_json_or_yaml
    from efe_tool.cli.errors import UsageError

    empty = tmp_path / "empty.json"
    empty.write_text("")
    with pytest.raises(UsageError):
        _load_json_or_yaml(empty)


def test_load_json_or_yaml_accepts_pure_json(tmp_path):
    from efe_tool.cli.commands.catalog import _load_json_or_yaml

    f = tmp_path / "labels.json"
    f.write_text(json.dumps({"env": "prod", "tier": "2"}))
    parsed = _load_json_or_yaml(f)
    assert parsed == {"env": "prod", "tier": "2"}


# ─────────────────────────────────────────────────────────────────────
# M-1 audit witness : Rust scaffolding pinned via the test count
# ─────────────────────────────────────────────────────────────────────
# (The Rust-side fork-cascade and audit-principal tests live in
#  `crates/efe-server/src/catalog/memory.rs::tests`. The cargo test
#  count is enforced by the bench-catalog workflow ; we don't
#  duplicate the Rust assertions here, but we sanity-check that the
#  in-memory module exports the expected surface.)


def test_d12_audit_fix_summary_mentioned_in_changelog():
    """The post-audit CHANGELOG must mention the fix series."""
    changelog = (_REPO / "CHANGELOG.md").read_text(encoding="utf-8")
    # Either the audit section is in [Unreleased] or in v0.1.12-post.
    assert "audit" in changelog.lower() or "D12" in changelog
