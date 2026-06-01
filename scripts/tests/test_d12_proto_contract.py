"""Sprint D12 — pin the catalog.proto contract.

Catches:

  * Removed RPC, message, or field.
  * Renamed RPC/message/field.
  * Tag number drift (a wire-format break).

The proto bindings are the authoritative source ; this test reads
the regenerated Python stubs and asserts the expected shapes.
"""

from __future__ import annotations

import importlib
import sys
from pathlib import Path

_REPO = Path(__file__).resolve().parents[2]
_SDK_PATH = _REPO / "clients" / "python"
sys.path.insert(0, str(_SDK_PATH))


# ─────────────────────────────────────────────────────────────────────
# Proto module loadable
# ─────────────────────────────────────────────────────────────────────

def test_catalog_pb2_importable():
    pb2 = importlib.import_module("efe_tool._proto.efe.v1.catalog_pb2")
    assert pb2 is not None


def test_catalog_pb2_grpc_importable():
    grpc = importlib.import_module("efe_tool._proto.efe.v1.catalog_pb2_grpc")
    assert hasattr(grpc, "CatalogServiceStub")
    assert hasattr(grpc, "CatalogServiceServicer")


# ─────────────────────────────────────────────────────────────────────
# RPC inventory
# ─────────────────────────────────────────────────────────────────────

EXPECTED_RPCS = frozenset({
    "RegisterCatalog",
    "EvolveCatalog",
    "GetCatalog",
    "ListCatalogs",
    "ListCatalogVersions",
    "DeleteCatalog",
})


def test_catalog_service_has_6_rpcs():
    """The grpc-python servicer class exposes one method per RPC.
    We read those rather than poking the Stub (whose RPC bindings
    are instance-attributes not class-attributes)."""
    grpc = importlib.import_module("efe_tool._proto.efe.v1.catalog_pb2_grpc")
    servicer_cls = grpc.CatalogServiceServicer
    actual = {name for name in dir(servicer_cls) if not name.startswith("_")}
    # Drop any inherited noise that's not in the expected set.
    rpc_names = actual & EXPECTED_RPCS
    assert rpc_names == EXPECTED_RPCS, (
        f"CatalogService RPC inventory drift: missing "
        f"{EXPECTED_RPCS - actual}"
    )


# ─────────────────────────────────────────────────────────────────────
# Message inventory
# ─────────────────────────────────────────────────────────────────────

EXPECTED_MESSAGES = frozenset({
    "CatalogRef",
    "CatalogContent",
    "EmbedderSpec",
    "CatalogClass",
    "ConePolicy",
    "ThresholdPolicy",
    "CanonicalCatalog",
    "CatalogMetadata",
    "CompiledClass",
    "RegisterCatalogRequest",
    "CatalogHandle",
    "EvolveCatalogRequest",
    "CatalogEvolutionOp",
    "AppendClassOp",
    "RemoveClassOp",
    "UpdateThresholdsOp",
    "ReplaceClassAnchorsOp",
    "RecompileOp",
    "GetCatalogRequest",
    "ListCatalogsRequest",
    "ListCatalogsResponse",
    "CatalogRefSummary",
    "ListCatalogVersionsRequest",
    "ListCatalogVersionsResponse",
    "DeleteCatalogRequest",
})


def test_message_inventory_complete():
    pb2 = importlib.import_module("efe_tool._proto.efe.v1.catalog_pb2")
    missing = {name for name in EXPECTED_MESSAGES if not hasattr(pb2, name)}
    assert not missing, (
        f"catalog_pb2 missing messages: {sorted(missing)}"
    )


# ─────────────────────────────────────────────────────────────────────
# CatalogContent shape
# ─────────────────────────────────────────────────────────────────────

def test_catalog_content_has_4_fields():
    pb2 = importlib.import_module("efe_tool._proto.efe.v1.catalog_pb2")
    content = pb2.CatalogContent()
    fields = {f.name for f in content.DESCRIPTOR.fields}
    assert fields == {"embedder", "classes", "cone_policy", "thresholds"}


def test_catalog_class_has_5_fields():
    pb2 = importlib.import_module("efe_tool._proto.efe.v1.catalog_pb2")
    cls = pb2.CatalogClass()
    fields = {f.name for f in cls.DESCRIPTOR.fields}
    assert fields == {"name", "description", "canonical_sentences",
                      "color", "deprecated"}


def test_embedder_spec_has_4_fields():
    pb2 = importlib.import_module("efe_tool._proto.efe.v1.catalog_pb2")
    spec = pb2.EmbedderSpec()
    fields = {f.name for f in spec.DESCRIPTOR.fields}
    assert fields == {"family", "model", "dim", "normalize"}


def test_threshold_policy_has_2_fields():
    pb2 = importlib.import_module("efe_tool._proto.efe.v1.catalog_pb2")
    t = pb2.ThresholdPolicy()
    fields = {f.name for f in t.DESCRIPTOR.fields}
    assert fields == {"confidence_high", "confidence_medium"}


# ─────────────────────────────────────────────────────────────────────
# CatalogMetadata audit envelope shape
# ─────────────────────────────────────────────────────────────────────

EXPECTED_METADATA_FIELDS = frozenset({
    "created_at", "created_by", "description", "parent_key",
    "evolution_op", "labels", "content_sha256",
})


def test_catalog_metadata_has_audit_envelope():
    pb2 = importlib.import_module("efe_tool._proto.efe.v1.catalog_pb2")
    meta = pb2.CatalogMetadata()
    fields = {f.name for f in meta.DESCRIPTOR.fields}
    assert fields == EXPECTED_METADATA_FIELDS, (
        f"CatalogMetadata field drift: {fields}"
    )


# ─────────────────────────────────────────────────────────────────────
# EvolveOp oneof shape
# ─────────────────────────────────────────────────────────────────────

EXPECTED_EVOLVE_VARIANTS = frozenset({
    "append_class",
    "remove_class",
    "update_thresholds",
    "replace_class_anchors",
    "recompile",
})


def test_evolve_op_has_5_oneof_variants():
    pb2 = importlib.import_module("efe_tool._proto.efe.v1.catalog_pb2")
    op = pb2.CatalogEvolutionOp()
    # The oneof field on this message is named "op".
    oneofs = {o.name for o in op.DESCRIPTOR.oneofs}
    assert oneofs == {"op"}
    # Variants are fields belonging to the oneof.
    variants = {f.name for f in op.DESCRIPTOR.fields
                if f.containing_oneof and f.containing_oneof.name == "op"}
    assert variants == EXPECTED_EVOLVE_VARIANTS, (
        f"CatalogEvolutionOp variants drift: {variants}"
    )


# ─────────────────────────────────────────────────────────────────────
# Tag number stability (wire-format)
# ─────────────────────────────────────────────────────────────────────

def test_register_request_tag_numbers_stable():
    """Tag numbers are the wire format ; renumbering breaks rolling
    deploys. Pin them."""
    pb2 = importlib.import_module("efe_tool._proto.efe.v1.catalog_pb2")
    req = pb2.RegisterCatalogRequest()
    expected = {
        "tenant": 1, "name": 2, "version": 3, "content": 4,
        "created_by": 5, "description": 6, "labels": 7,
    }
    actual = {f.name: f.number for f in req.DESCRIPTOR.fields}
    assert actual == expected, f"RegisterCatalogRequest tag drift: {actual}"


def test_canonical_catalog_tag_numbers_stable():
    pb2 = importlib.import_module("efe_tool._proto.efe.v1.catalog_pb2")
    cc = pb2.CanonicalCatalog()
    expected = {"content": 1, "metadata": 2, "compiled": 3}
    actual = {f.name: f.number for f in cc.DESCRIPTOR.fields}
    assert actual == expected, f"CanonicalCatalog tag drift: {actual}"


def test_catalog_metadata_tag_numbers_stable():
    pb2 = importlib.import_module("efe_tool._proto.efe.v1.catalog_pb2")
    meta = pb2.CatalogMetadata()
    expected = {
        "created_at": 1, "created_by": 2, "description": 3,
        "parent_key": 4, "evolution_op": 5, "labels": 6,
        "content_sha256": 7,
    }
    actual = {f.name: f.number for f in meta.DESCRIPTOR.fields}
    assert actual == expected, f"CatalogMetadata tag drift: {actual}"


# ─────────────────────────────────────────────────────────────────────
# Round-trip via the SDK helpers
# ─────────────────────────────────────────────────────────────────────

def test_dict_to_proto_to_dict_round_trips():
    """The SDK's _content_dict_to_proto + _content_proto_to_dict
    helpers must round-trip without losing any field."""
    import math

    from efe_tool._proto.efe.v1 import catalog_pb2
    from efe_tool.admin.client import (
        _content_dict_to_proto, _content_proto_to_dict,
    )

    original = {
        "embedder": {"family": "st", "model": "mini",
                     "dim": 384, "normalize": "l2"},
        "classes": [
            {"name": "a", "description": "A class",
             "canonical_sentences": ["s1", "s2"],
             "color": "#000", "deprecated": False},
            {"name": "b", "description": "",
             "canonical_sentences": ["s3"],
             "color": "", "deprecated": True},
        ],
        "cone_policy": {"max_projection_distance": 0.7},
        "thresholds": {"confidence_high": 0.8, "confidence_medium": 0.5},
    }
    proto = _content_dict_to_proto(original, catalog_pb2)
    rt = _content_proto_to_dict(proto)

    assert rt["embedder"] == original["embedder"]
    assert len(rt["classes"]) == 2
    assert rt["classes"][0]["name"] == "a"
    assert rt["classes"][0]["canonical_sentences"] == ["s1", "s2"]
    assert rt["classes"][1]["deprecated"] is True
    # proto3 float (32-bit) ; tolerant comparisons.
    assert math.isclose(rt["cone_policy"]["max_projection_distance"], 0.7,
                        rel_tol=1e-5)
    assert math.isclose(rt["thresholds"]["confidence_high"], 0.8,
                        rel_tol=1e-5)
