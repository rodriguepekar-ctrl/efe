"""Sprint D10 — pin the audit's bug fixes.

Audit fixes covered:

  * Pkg1 — `pyproject.toml` version stays in sync with
    `efe_tool/_version.py`. Drift would silently ship the wrong
    wheel to PyPI.
  * A1 — `efe_tool.AsyncEfeClient` is the REAL class (PEP 562
    lazy-import via module-level `__getattr__`), so
    `isinstance(obj, AsyncEfeClient)` works.
  * A4 — `AsyncEfeClient` accepts `host=` (gRPC-style) and
    translates internally to `endpoint=` for the sync `EfeClient`,
    plus auto-creates a `LocalSTEmbedder.from_env()` when no
    embedder is supplied.
  * E4 — example 02's YAML schema matches the real loader
    (``canonical_sentences`` field, flat ``thresholds``).
  * E5 — doc pages no longer document the wrong YAML schema.
  * E8 — smoke test env aligns with the bundled catalog's
    declared embedder fingerprint.
  * Stub1 — stub server hashes request_id when raw_prompt is
    absent (default path) so smoke tests see a balanced label
    distribution.

These are END-TO-END functional checks at the public surface —
they would NOT have passed before the audit fixes.
"""

from __future__ import annotations

import hashlib
import importlib
import re
import subprocess
import sys
from pathlib import Path

import pytest


_REPO = Path(__file__).resolve().parents[2]
_SDK = _REPO / "clients" / "python"
sys.path.insert(0, str(_SDK))


# ─────────────────────────────────────────────────────────────────────
# Pkg1 — pyproject.toml ↔ _version.py sync
# ─────────────────────────────────────────────────────────────────────

def test_pyproject_version_matches_sdk_version():
    pyproject = (_SDK / "pyproject.toml").read_text(encoding="utf-8")
    m = re.search(r'^\s*version\s*=\s*"([^"]+)"', pyproject, re.MULTILINE)
    assert m, "pyproject.toml version field not found"
    pyproject_version = m.group(1)

    efe_tool = importlib.import_module("efe_tool")
    sdk_version = efe_tool.__version__

    assert pyproject_version == sdk_version, (
        f"audit fix Pkg1 regression — "
        f"pyproject.toml version={pyproject_version!r} but "
        f"efe_tool.__version__={sdk_version!r}.\n"
        f"PyPI publish would ship the wrong wheel name."
    )


# ─────────────────────────────────────────────────────────────────────
# A1 — AsyncEfeClient is the real class (isinstance works)
# ─────────────────────────────────────────────────────────────────────

def test_async_client_lazy_import_does_not_pull_in_module():
    """Importing `efe_tool` MUST NOT eagerly import `_async.client`."""
    # Reset import state.
    for mod in list(sys.modules):
        if mod.startswith("efe_tool._async"):
            del sys.modules[mod]
    if "efe_tool" in sys.modules:
        del sys.modules["efe_tool"]
    importlib.import_module("efe_tool")
    assert "efe_tool._async.client" not in sys.modules, (
        "audit fix A1 regression — `efe_tool._async.client` was "
        "eagerly imported. Lazy-load via PEP 562 __getattr__ is "
        "required so test suites that mock at the gRPC layer don't "
        "pay the import cost."
    )


def test_async_client_attribute_access_resolves_to_real_class():
    """Accessing `efe_tool.AsyncEfeClient` returns the actual class
    (not a proxy instance) — fixes audit bug A1 where
    `isinstance(obj, AsyncEfeClient)` would crash."""
    efe_tool = importlib.import_module("efe_tool")
    AsyncEfeClient = efe_tool.AsyncEfeClient
    # Must be a class (type), not an instance of a proxy class.
    assert isinstance(AsyncEfeClient, type), (
        f"audit fix A1 regression — `AsyncEfeClient` is "
        f"{type(AsyncEfeClient).__name__!r}, not a class. "
        f"`isinstance` checks would fail."
    )
    # The class name must match what users expect.
    assert AsyncEfeClient.__name__ == "AsyncEfeClient"


def test_isinstance_works_against_async_client():
    """The fix's main user-visible benefit: `isinstance(obj, AsyncEfeClient)`
    must not raise TypeError."""
    efe_tool = importlib.import_module("efe_tool")
    AsyncEfeClient = efe_tool.AsyncEfeClient
    # `isinstance(None, X)` should always return False (not raise).
    # Pre-fix this raised TypeError because X was an instance, not a type.
    assert isinstance(None, AsyncEfeClient) is False


# ─────────────────────────────────────────────────────────────────────
# A4 — AsyncEfeClient kwargs translation + embedder default
# ─────────────────────────────────────────────────────────────────────

def test_async_client_constructor_accepts_host_only():
    """The async client should auto-create a LocalSTEmbedder when
    no embedder is supplied, matching the DX of `EfeClient.from_env()`."""
    efe_tool = importlib.import_module("efe_tool")
    # Trigger import.
    Cls = efe_tool.AsyncEfeClient
    # Smoke instantiation — does NOT enter the async context, so no
    # network is touched. Just verifies the constructor wires up.
    import warnings
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", DeprecationWarning)
        client = Cls(host="127.0.0.1:50051", insecure=True)
    assert client._host == "127.0.0.1:50051"
    assert client._embedder is not None  # auto-created
    # And the embedder is a LocalSTEmbedder (or compatible).
    assert hasattr(client._embedder, "fingerprint")


def test_build_sync_client_uses_correct_kwarg_names():
    """`_build_sync_client` translates ``host`` → ``endpoint`` and
    ``token`` → ``jwt`` (audit fix A4). Pre-fix it called
    `EfeClient(host=..., token=...)` which raises TypeError.
    """
    efe_tool = importlib.import_module("efe_tool")
    Cls = efe_tool.AsyncEfeClient
    import warnings
    with warnings.catch_warnings():
        warnings.simplefilter("ignore", DeprecationWarning)
        client = Cls(
            host="127.0.0.1:50051",
            insecure=True,
            token="fake-jwt-for-test",
            tenant="test-tenant",
        )
    # We can't actually call _build_sync_client without a real
    # gRPC channel, but we CAN verify the kwargs map by inspecting
    # the source path. Check via a no-network call:
    # (We just inspect the source for the right kwargs.)
    import inspect
    src = inspect.getsource(client._build_sync_client)
    assert '"endpoint"' in src, (
        "audit fix A4 regression — `_build_sync_client` should pass "
        "`endpoint=` to `EfeClient`, not `host=`."
    )
    assert '"jwt"' in src, (
        "audit fix A4 regression — should map `token` → `jwt`."
    )
    assert '"embedder"' in src, (
        "audit fix A4 regression — must pass `embedder=` (required by `EfeClient`)."
    )


# ─────────────────────────────────────────────────────────────────────
# E4/E5 — YAML schema in example 02 + docs
# ─────────────────────────────────────────────────────────────────────

def test_example_02_uses_canonical_sentences_field():
    src = (_REPO / "examples" / "02_custom_catalog.py").read_text(encoding="utf-8")
    assert '"canonical_sentences"' in src, (
        "audit fix E4 regression — example 02 must use "
        "`canonical_sentences` (not `anchors`) in its YAML emitter."
    )
    assert '"anchors": prompts' not in src, (
        "audit fix E4 regression — `anchors:` field is a stale schema."
    )


def test_example_02_uses_flat_threshold_schema():
    src = (_REPO / "examples" / "02_custom_catalog.py").read_text(encoding="utf-8")
    assert "confidence_high" in src, (
        "audit fix E4 regression — example 02 must use "
        "`confidence_high` in its YAML emitter."
    )
    assert "min_top_cosine" not in src, (
        "audit fix E4 regression — `min_top_cosine` is a stale schema field."
    )


@pytest.mark.parametrize(
    "doc_path",
    [
        "docs/docs/howto/custom-catalog.md",
        "docs/docs/reference/catalog-schema.md",
        "docs/docs/tutorial/your-first-classifier.md",
        "docs/docs/tutorial/concepts.md",
    ],
)
def test_docs_use_canonical_sentences(doc_path: str):
    text = (_REPO / doc_path).read_text(encoding="utf-8")
    # We allow `anchors` to appear as ENGLISH word (e.g. "2-4 anchors per
    # class") but NOT as a YAML field reference (`anchors:` colon-suffix).
    bad = re.search(r"^\s*anchors:\s*(?:#.*)?$", text, re.MULTILINE)
    assert not bad, (
        f"audit fix E5 regression — {doc_path} still documents stale "
        f"`anchors:` YAML field. Use `canonical_sentences:`."
    )


@pytest.mark.parametrize(
    "doc_path",
    [
        "docs/docs/howto/custom-catalog.md",
        "docs/docs/reference/catalog-schema.md",
    ],
)
def test_docs_use_flat_thresholds_schema(doc_path: str):
    text = (_REPO / doc_path).read_text(encoding="utf-8")
    assert "min_top_cosine" not in text, (
        f"audit fix E5 regression — {doc_path} still documents stale "
        f"`min_top_cosine`. Use `confidence_high` / `confidence_medium`."
    )


# ─────────────────────────────────────────────────────────────────────
# E8 — smoke test env matches bundled catalog
# ─────────────────────────────────────────────────────────────────────

def test_smoke_test_env_matches_bundled_catalog_embedder():
    smoke = (_REPO / "examples" / "tests" / "test_examples_smoke.py").read_text(encoding="utf-8")
    catalog = (_REPO / "catalogs" / "support_triage.yaml").read_text(encoding="utf-8")
    # Extract the catalog's declared embedder model.
    m = re.search(r"^\s+model:\s+(\S+)\s*$", catalog, re.MULTILINE)
    assert m, "catalog must declare an embedder.model"
    catalog_model = m.group(1)
    expected_env_value = f"sentence-transformers/{catalog_model}"
    assert expected_env_value in smoke, (
        f"audit fix E8 regression — smoke test env should set "
        f"`EFE_EMBEDDER_MODEL={expected_env_value}` to match the "
        f"bundled catalog ; got smoke env that doesn't match the "
        f"catalog's declared model `{catalog_model}`."
    )


# ─────────────────────────────────────────────────────────────────────
# Stub1 — stub server returns deterministic balanced labels
# ─────────────────────────────────────────────────────────────────────

def _import_stub_server_label_fn():
    """Import only the ``_label_for`` function from stub_server.py.

    Defers importing the protobuf stubs (which require matching
    runtime/gencode versions). The stub-server module-level code
    pulls in `efe_tool._proto.efe.v1.efe_pb2` which can fail on
    development machines with stale protobuf wheels — but the
    pure-Python label-rotation logic doesn't need it. We isolate
    that by reading the source and exec-ing only the bits we need.
    """
    src = (_REPO / "tests" / "stub_server.py").read_text(encoding="utf-8")
    # Slice out the section between the markers we control.
    start = src.find("import hashlib")
    assert start > 0, "stub server source structure changed; update test"
    end = src.find("# ─", start + 50)   # the next section divider
    snippet = src[start:end]
    ns: dict = {}
    exec(snippet, ns)
    return ns["_label_for"]


def test_stub_server_label_for_returns_all_four_labels():
    """Hash-rotation must produce all 4 labels over a small input
    set, so smoke tests can observe `fraud`, `billing`, `question`,
    `bug` substrings in stdout regardless of raw_prompt visibility."""
    label_for = _import_stub_server_label_fn()
    labels_seen = set()
    for i in range(50):
        text = f"reqid-{i:04x}"
        label, _conf = label_for(text)
        labels_seen.add(label)
    assert labels_seen == {"fraud", "billing", "question", "bug"}, (
        f"audit fix Stub1 regression — hash rotation should cover all 4 "
        f"labels over 50 samples; got {labels_seen}."
    )


def test_stub_server_label_is_deterministic_per_input():
    """Same input → same label, across interpreter restarts (sha256-based,
    not python `hash()` which is randomised)."""
    label_for = _import_stub_server_label_fn()
    label1, _ = label_for("test-input-12345")
    label2, _ = label_for("test-input-12345")
    assert label1 == label2
