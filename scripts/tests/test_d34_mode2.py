"""Sprint **D34** (M4 § 13) — pin tests for the Mode-2 service.

Verifies :
  * proto declares Mode2Service with 3 RPCs (Execute, ExecuteStreaming,
    RouterHealth).
  * `LlmRouter` trait + 3 built-in implementations (mock, openai,
    anthropic).
  * Mode2ServiceImpl wires the cache (xxhash64 key on
    (query, top_k, router_id)).
  * 5-minute default cache TTL.
  * Cache flips `from_cache=true` on hit.
  * Service is registered in runtime.rs.
"""
from __future__ import annotations

from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
MODE2_PROTO = REPO_ROOT / "protos" / "efe" / "v1" / "mode2.proto"
ROUTER_RS = REPO_ROOT / "crates" / "efe-server" / "src" / "services" / "mode2" / "router.rs"
MODE2_RS = REPO_ROOT / "crates" / "efe-server" / "src" / "services" / "mode2" / "mod.rs"
RUNTIME_RS = REPO_ROOT / "crates" / "efe-server" / "src" / "runtime.rs"
SCOPES_RS = REPO_ROOT / "crates" / "efe-server" / "src" / "auth" / "scopes.rs"
LIB_RS = REPO_ROOT / "crates" / "efe-server" / "src" / "lib.rs"
BUILD_RS = REPO_ROOT / "crates" / "efe-server" / "build.rs"


def _read(p: Path) -> str:
    return p.read_text(encoding="utf-8")


def test_proto_declares_mode2_service_3_rpcs() -> None:
    proto = _read(MODE2_PROTO)
    assert "service Mode2Service" in proto
    for rpc in ("Execute", "ExecuteStreaming", "RouterHealth"):
        assert f"rpc {rpc}(" in proto, f"Mode2Service.{rpc} missing"


def test_proto_namespace_is_efe_v1_mode2() -> None:
    proto = _read(MODE2_PROTO)
    assert "package efe.v1.mode2;" in proto


def test_router_trait_has_id_and_generate_anchors() -> None:
    rs = _read(ROUTER_RS)
    assert "pub trait LlmRouter:" in rs
    assert "fn id(&self)" in rs
    assert "async fn generate_anchors(" in rs


def test_three_built_in_routers_present() -> None:
    rs = _read(ROUTER_RS)
    for impl in ("MockRouter", "OpenAIRouter", "AnthropicRouter"):
        assert f"pub struct {impl}" in rs, f"router {impl} missing"
        assert f"impl LlmRouter for {impl}" in rs, f"trait impl for {impl} missing"


def test_router_ids_are_distinct_strings() -> None:
    rs = _read(ROUTER_RS)
    # Each router defines a stable string id ; the test asserts these
    # are present. Distinctness is enforced by the unit tests inside
    # the module (`three_routers_have_distinct_ids`).
    for id_str in ('"mock"', '"openai"', '"anthropic"'):
        assert id_str in rs, f"router id {id_str} missing"


def test_mock_router_is_deterministic_unit_norm() -> None:
    """Pin the deterministic + unit-norm contract by checking the impl
    has the expected helper + L2 normalisation."""
    rs = _read(ROUTER_RS)
    assert "fn synth_centroid" in rs
    assert "L2-normalise" in rs or "L2-norm" in rs.lower()


def test_mode2_service_uses_xxhash_cache_key() -> None:
    rs = _read(MODE2_RS)
    assert "fn cache_key(" in rs
    assert "xxhash_rust::xxh64" in rs


def test_mode2_default_cache_ttl_is_5_minutes() -> None:
    rs = _read(MODE2_RS)
    assert "DEFAULT_CACHE_TTL" in rs
    assert "Duration::from_secs(5 * 60)" in rs


def test_mode2_cache_hit_flips_from_cache_flag() -> None:
    rs = _read(MODE2_RS)
    body = rs[rs.find("fn cache_get(") :]
    body = body[: body.find("\n    fn ")]
    assert "from_cache = true" in body


def test_mode2_default_routers_factory_present() -> None:
    rs = _read(MODE2_RS)
    assert "fn default_routers(dim: usize)" in rs


def test_runtime_wires_mode2_service() -> None:
    rs = _read(RUNTIME_RS)
    assert "Mode2ServiceServer" in rs
    assert "Mode2ServiceImpl::new(" in rs
    assert "Mode2ServiceImpl::default_routers" in rs


def test_lib_rs_exposes_v1_mode2_module() -> None:
    rs = _read(LIB_RS)
    assert "pub mod mode2 {" in rs
    assert 'tonic::include_proto!("efe.v1.mode2")' in rs


def test_build_rs_compiles_mode2_proto() -> None:
    rs = _read(BUILD_RS)
    assert 'efe_v1.join("mode2.proto")' in rs


def test_scopes_rs_maps_three_mode2_rpcs() -> None:
    rs = _read(SCOPES_RS)
    for rpc in ("Execute", "ExecuteStreaming", "RouterHealth"):
        assert f"/efe.v1.mode2.Mode2Service/{rpc}" in rs, f"path mapping missing for Mode2Service.{rpc}"


def test_mode2_handlers_enforce_data_broad_scope() -> None:
    rs = _read(MODE2_RS)
    assert "BroadScope::DATA" in rs


if __name__ == "__main__":
    raise SystemExit(pytest.main([__file__, "-v"]))
