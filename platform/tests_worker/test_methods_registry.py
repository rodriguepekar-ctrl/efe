"""Tests for :mod:`efe_worker.methods.registry` + method instantiation."""

from __future__ import annotations

import pytest

from efe_worker.errors import InvalidHyperparameters, UnknownRecipe
from efe_worker.methods import METHOD_REGISTRY, register_method, resolve_method


def test_registry_has_five_methods() -> None:
    assert set(METHOD_REGISTRY.keys()) == {
        "infonce", "triplet", "classif", "lora", "distill",
    }


def test_resolve_unknown() -> None:
    with pytest.raises(UnknownRecipe):
        resolve_method("galaxy_brain")


def test_resolve_classif_defaults() -> None:
    m = resolve_method("classif")
    assert m.name == "classif"
    assert m.dropout == 0.1
    assert m.label_smoothing == 0.1


def test_resolve_triplet_invalid_margin() -> None:
    with pytest.raises(InvalidHyperparameters):
        resolve_method("triplet", {"margin": 5.0})


def test_resolve_triplet_invalid_mining() -> None:
    with pytest.raises(InvalidHyperparameters):
        resolve_method("triplet", {"mining": "galaxy_brain"})


def test_resolve_infonce_invalid_temperature() -> None:
    with pytest.raises(InvalidHyperparameters):
        resolve_method("infonce", {"temperature": 5.0})


def test_resolve_lora_r_bounds() -> None:
    with pytest.raises(InvalidHyperparameters):
        resolve_method("lora", {"r": 200})


def test_resolve_lora_invalid_target_string() -> None:
    with pytest.raises(InvalidHyperparameters):
        resolve_method("lora", {"target_modules": "only-q"})


def test_resolve_lora_empty_targets() -> None:
    with pytest.raises(InvalidHyperparameters):
        resolve_method("lora", {"target_modules": []})


def test_resolve_distill_requires_teacher() -> None:
    with pytest.raises(InvalidHyperparameters):
        resolve_method("distill", {})


def test_resolve_distill_invalid_temperature() -> None:
    with pytest.raises(InvalidHyperparameters):
        resolve_method(
            "distill", {"temperature": 50.0, "teacher_model": "t"},
        )


def test_resolve_distill_invalid_alpha() -> None:
    with pytest.raises(InvalidHyperparameters):
        resolve_method(
            "distill", {"alpha": 1.5, "teacher_model": "t"},
        )


def test_register_method_custom() -> None:
    class _Fake:
        name = "fake"

    def factory(hp: dict) -> _Fake:
        return _Fake()

    register_method("fake", factory)
    m = resolve_method("fake")
    assert m.name == "fake"
    del METHOD_REGISTRY["fake"]


def test_register_empty_name_rejected() -> None:
    with pytest.raises(ValueError):
        register_method("", lambda _: None)


def test_classif_invalid_pool() -> None:
    with pytest.raises(InvalidHyperparameters):
        resolve_method("classif", {"pool": "galaxy_brain"})
