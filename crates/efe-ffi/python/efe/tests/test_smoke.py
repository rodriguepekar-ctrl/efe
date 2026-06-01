"""Smoke tests — does the wheel load + can we call basic functions?"""

from __future__ import annotations

import efe


def test_version_is_string() -> None:
    v = efe.version()
    assert isinstance(v, str)
    assert v


def test_module_exports_classes() -> None:
    assert hasattr(efe, "Verdict")
    assert hasattr(efe, "Bundle")
    assert hasattr(efe, "EfeNativeClient")
    assert hasattr(efe, "SwarmController")
    assert hasattr(efe, "SwarmSpec")
    assert hasattr(efe, "EpistemicLatentSpace")


def test_module_exports_functions() -> None:
    assert callable(efe.classify_zero_copy)
    assert callable(efe.aclassify_zero_copy)
    assert callable(efe.load_bundle)
    assert callable(efe.version)


def test_module_exports_exceptions() -> None:
    assert issubclass(efe.EfeError, Exception)
    assert issubclass(efe.BundleError, efe.EfeError)
    assert issubclass(efe.AlignmentBreach, efe.EfeError)
    assert issubclass(efe.NumericalError, efe.EfeError)
    assert issubclass(efe.ConfigError, efe.EfeError)
    assert issubclass(efe.SwarmError, efe.EfeError)


def test_dunder_version_set() -> None:
    assert isinstance(efe.__version__, str)
    assert efe.__version__ == efe.version()
