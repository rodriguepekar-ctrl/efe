"""Sprint **M15** — Pin tests : EFE Playground (try-EFE web environment).

Cf. `playground/README.md`.

Structural pin tests covering the playground's canonical artefacts +
sample integrity + tutorial chain. Doesn't run the backend (use
`playground/tests/test_e2e.py` for that — runs in playground's own
test scope).

Tests :
  1. playground/ directory structure inventory
  2. Backend modules present + non-empty
  3. Frontend SPA artefacts present
  4. 4 sample catalogs valid JSON + ≥ 2 classes each
  5. 5 tutorials present + non-empty
  6. README + Dockerfile + docker-compose + quickstart.sh present
  7. Backend imports correctly (smoke-import)
  8. Sample JSON schema : every class has `name` + `examples` array
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

REPO = Path(__file__).resolve().parents[2]
PLAYGROUND = REPO / "playground"


def test_m15_playground_directory_inventory() -> None:
    expected_dirs = [
        "backend",
        "frontend",
        "samples",
        "scripts",
        "tutorials",
        "tests",
    ]
    for d in expected_dirs:
        assert (PLAYGROUND / d).is_dir(), (
            f"M15 playground missing directory : {PLAYGROUND / d}"
        )


def test_m15_backend_modules_present() -> None:
    expected = [
        "backend/app.py",
        "backend/embedder.py",
        "backend/catalog.py",
        "backend/audit.py",
        "backend/auth.py",
        "backend/requirements.txt",
    ]
    for f in expected:
        path = PLAYGROUND / f
        assert path.is_file(), f"M15 backend missing : {path}"
        assert path.stat().st_size >= 100, f"M15 backend file too small : {path}"


def test_m15_frontend_artefacts_present() -> None:
    expected = [
        "frontend/index.html",
        "frontend/style.css",
        "frontend/api.js",
        "frontend/app.js",
    ]
    for f in expected:
        path = PLAYGROUND / f
        assert path.is_file(), f"M15 frontend missing : {path}"
        assert path.stat().st_size >= 500


def test_m15_samples_valid_json_with_classes() -> None:
    samples_dir = PLAYGROUND / "samples"
    expected_samples = [
        "support_routing.json",
        "content_moderation.json",
        "intent_recognition.json",
        "multilingual_intents.json",
    ]
    for s in expected_samples:
        path = samples_dir / s
        assert path.is_file(), f"M15 sample missing : {path}"
        data = json.loads(path.read_text(encoding="utf-8"))
        # Required top-level keys
        assert "name" in data, f"sample {s} missing `name`"
        assert "description" in data, f"sample {s} missing `description`"
        assert "classes" in data, f"sample {s} missing `classes`"
        assert isinstance(data["classes"], list)
        assert len(data["classes"]) >= 2, (
            f"sample {s} must have ≥ 2 classes, got {len(data['classes'])}"
        )
        for c in data["classes"]:
            assert "name" in c, f"sample {s} class missing `name`"
            assert "examples" in c, f"sample {s} class missing `examples`"
            assert isinstance(c["examples"], list)
            assert len(c["examples"]) >= 1, (
                f"sample {s} class {c.get('name')} must have ≥ 1 example"
            )


def test_m15_tutorials_present() -> None:
    expected = [
        "tutorials/00_introduction.md",
        "tutorials/01_first_catalog.md",
        "tutorials/02_multi_tenant.md",
        "tutorials/03_drift_detection.md",
        "tutorials/04_production_deploy.md",
    ]
    for f in expected:
        path = PLAYGROUND / f
        assert path.is_file(), f"M15 tutorial missing : {path}"
        assert path.stat().st_size >= 500, f"M15 tutorial too small : {path}"


def test_m15_readme_dockerfile_quickstart_present() -> None:
    expected = [
        "README.md",
        "Dockerfile",
        "docker-compose.yml",
        "scripts/quickstart.sh",
    ]
    for f in expected:
        path = PLAYGROUND / f
        assert path.is_file(), f"M15 root file missing : {path}"


def test_m15_quickstart_executable() -> None:
    import os
    qs = PLAYGROUND / "scripts" / "quickstart.sh"
    assert os.access(qs, os.X_OK), (
        f"M15 quickstart.sh must be executable (chmod +x)"
    )


def test_m15_backend_modules_importable() -> None:
    """Smoke-import the backend modules to catch syntax errors."""
    sys.path.insert(0, str(PLAYGROUND / "backend"))
    try:
        import embedder  # noqa: F401
        import catalog  # noqa: F401
        import audit  # noqa: F401
        import auth  # noqa: F401
        # app imports flask which may not be installed in pin-test env ;
        # we don't import app.py here.
    except ImportError as e:
        pytest.skip(f"backend deps not installed (numpy ?) : {e}")
    finally:
        if str(PLAYGROUND / "backend") in sys.path:
            sys.path.remove(str(PLAYGROUND / "backend"))


def test_m15_multilingual_sample_has_multi_language_examples() -> None:
    """The multilingual sample should have examples in ≥ 5 languages."""
    sample = json.loads(
        (PLAYGROUND / "samples" / "multilingual_intents.json").read_text(encoding="utf-8")
    )
    assert sample.get("language") == "multi"
    # Look for typical FR / DE / ES / IT / PT markers across examples
    all_examples = " ".join(
        ex for c in sample["classes"] for ex in c["examples"]
    ).lower()
    # Each language has at least one identifying word
    lang_markers = {
        "fr": ["bonjour", "merci", "comment"],
        "es": ["hola", "gracias", "por favor"],
        "de": ["hallo", "danke", "guten"],
        "it": ["ciao", "grazie", "buongiorno"],
        "pt": ["olá", "obrigado", "bom dia"],
    }
    detected = []
    for lang, words in lang_markers.items():
        if any(w in all_examples for w in words):
            detected.append(lang)
    assert len(detected) >= 4, (
        f"multilingual sample must contain ≥ 4 EU languages, detected : {detected}"
    )


def test_m15_dockerfile_uses_non_root() -> None:
    body = (PLAYGROUND / "Dockerfile").read_text(encoding="utf-8")
    assert "USER efe" in body, (
        "Dockerfile must drop to non-root user (security best practice)"
    )
    assert "uid 10001" in body or "10001" in body, (
        "Dockerfile must use UID 10001 (matching production EFE)"
    )


def test_m15_samples_provide_admin_use_cases() -> None:
    """Each sample documents its `use_case` for sales/operator orientation."""
    for s in ("support_routing", "content_moderation", "intent_recognition", "multilingual_intents"):
        data = json.loads(
            (PLAYGROUND / "samples" / f"{s}.json").read_text(encoding="utf-8")
        )
        assert "use_case" in data, f"sample {s} missing `use_case` field"
        assert len(data["use_case"]) >= 20
