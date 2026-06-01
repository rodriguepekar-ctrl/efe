"""Sprint **D37** (M4 § 14) — pin tests for the Mode-3 service.

Verifies :
  * proto declares Mode3Service with 3 RPCs (Execute, SubscribeVerdict,
    Cancel).
  * `Mode3Aggregator` trait + 3 built-in implementations
    (Mean, Weighted, Median).
  * Hard cap k = 5 (SLO S-12) enforced via `clamp_k`.
  * tokio::JoinSet used for parallel sub-swarms.
  * Service is registered in runtime.rs.
"""
from __future__ import annotations

from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
MODE3_PROTO = REPO_ROOT / "protos" / "efe" / "v1" / "mode3.proto"
AGG_RS = REPO_ROOT / "crates" / "efe-server" / "src" / "services" / "mode3" / "aggregator.rs"
MODE3_RS = REPO_ROOT / "crates" / "efe-server" / "src" / "services" / "mode3" / "mod.rs"
RUNTIME_RS = REPO_ROOT / "crates" / "efe-server" / "src" / "runtime.rs"
SCOPES_RS = REPO_ROOT / "crates" / "efe-server" / "src" / "auth" / "scopes.rs"
LIB_RS = REPO_ROOT / "crates" / "efe-server" / "src" / "lib.rs"
BUILD_RS = REPO_ROOT / "crates" / "efe-server" / "build.rs"


def _read(p: Path) -> str:
    return p.read_text(encoding="utf-8")


def test_proto_declares_mode3_service_3_rpcs() -> None:
    proto = _read(MODE3_PROTO)
    assert "service Mode3Service" in proto
    for rpc in ("Execute", "SubscribeVerdict", "Cancel"):
        assert f"rpc {rpc}(" in proto, f"Mode3Service.{rpc} missing"


def test_proto_namespace_is_efe_v1_mode3() -> None:
    proto = _read(MODE3_PROTO)
    assert "package efe.v1.mode3;" in proto


def test_aggregator_trait_present() -> None:
    rs = _read(AGG_RS)
    assert "pub trait Mode3Aggregator:" in rs
    assert "fn aggregate(" in rs


def test_three_built_in_aggregators_present() -> None:
    rs = _read(AGG_RS)
    for impl in ("MeanAggregator", "WeightedAggregator", "MedianAggregator"):
        assert f"pub struct {impl}" in rs, f"aggregator {impl} missing"
        assert f"impl Mode3Aggregator for {impl}" in rs, f"trait impl for {impl} missing"


def test_aggregator_ids_distinct() -> None:
    rs = _read(AGG_RS)
    for id_str in ('"mean"', '"weighted"', '"median"'):
        assert id_str in rs, f"aggregator id {id_str} missing"


def test_aggregation_metadata_carries_dispersion() -> None:
    proto = _read(MODE3_PROTO)
    body = proto[proto.find("message AggregationMetadata") :]
    body = body[: body.find("}") + 1]
    for f in ("strategy", "total_confidence", "dispersion", "effective_n"):
        assert f in body, f"AggregationMetadata.{f} missing"


def test_k_max_is_5() -> None:
    rs = _read(MODE3_RS)
    assert "pub const K_MAX: u32 = 5" in rs


def test_clamp_k_function_present() -> None:
    rs = _read(MODE3_RS)
    assert "fn clamp_k(k: u32) -> (u32, bool)" in rs


def test_mode3_uses_join_set() -> None:
    rs = _read(MODE3_RS)
    assert "tokio::task::JoinSet" in rs
    body = rs[rs.find("async fn execute(") :]
    assert "JoinSet" in body
    assert "set.spawn(" in body


def test_mode3_service_supports_cancel() -> None:
    rs = _read(MODE3_RS)
    assert "async fn cancel(" in rs
    assert "abort_handles" in rs


def test_runtime_wires_mode3_service() -> None:
    rs = _read(RUNTIME_RS)
    assert "Mode3ServiceServer" in rs
    assert "Mode3ServiceImpl::new(" in rs
    assert "Mode3ServiceImpl::default_aggregators" in rs


def test_lib_rs_exposes_v1_mode3_module() -> None:
    rs = _read(LIB_RS)
    assert "pub mod mode3 {" in rs
    assert 'tonic::include_proto!("efe.v1.mode3")' in rs


def test_build_rs_compiles_mode3_proto() -> None:
    rs = _read(BUILD_RS)
    assert 'efe_v1.join("mode3.proto")' in rs


def test_scopes_rs_maps_three_mode3_rpcs() -> None:
    rs = _read(SCOPES_RS)
    for rpc in ("Execute", "SubscribeVerdict", "Cancel"):
        assert f"/efe.v1.mode3.Mode3Service/{rpc}" in rs


def test_mode3_handlers_enforce_data_broad_scope() -> None:
    rs = _read(MODE3_RS)
    assert "BroadScope::DATA" in rs


def test_proto_k_clamped_field_present() -> None:
    """The response MUST surface `k_clamped` and `k_effective` so
    clients can detect when the hard cap was hit."""
    proto = _read(MODE3_PROTO)
    body = proto[proto.find("message Mode3ExecuteResponse") :]
    body = body[: body.find("}") + 1]
    assert "k_clamped" in body
    assert "k_effective" in body


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-v"]))
