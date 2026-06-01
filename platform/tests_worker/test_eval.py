"""Tests for :func:`efe_worker.training.eval.evaluate`."""

from __future__ import annotations

from typing import Any

from efe_worker.data.snapshot import Row
from efe_worker.data.tokenizer import FakeTokenizer
from efe_worker.methods.base import Model
from efe_worker.training.eval import evaluate


class _M:
    name = "m"
    switched: list[str] = []

    def __init__(self) -> None:
        self.switched = []

    def build_model(self, *a: Any, **k: Any) -> Any: raise NotImplementedError
    def build_sampler(self, *a: Any, **k: Any) -> Any: raise NotImplementedError
    def compute_loss(self, *a: Any, **k: Any) -> Any: raise NotImplementedError

    def eval_metrics(
        self, model: Model, val_rows: list[Row], tokenizer: FakeTokenizer,
        *, device: Any,
    ) -> dict[str, float]:
        return {"accuracy": 0.9, "val/f1": 0.88}

    def save_artifact(self, *a: Any, **k: Any) -> Any: raise NotImplementedError


class _Mdl:
    def __init__(self) -> None:
        self.switched: list[str] = []

    def to(self, d: Any) -> _Mdl: return self

    def train(self, mode: bool = True) -> _Mdl:
        self.switched.append(f"train({mode})")
        return self

    def eval(self) -> _Mdl:
        self.switched.append("eval()")
        return self

    def parameters(self) -> Any: return []
    def state_dict(self) -> dict[str, Any]: return {}
    def load_state_dict(self, state: Any, strict: bool = True) -> Any: return None


def test_evaluate_empty_returns_empty() -> None:
    m = evaluate(_M(), _Mdl(), [], FakeTokenizer(), device="cpu")
    assert m == {}


def test_evaluate_normalises_keys() -> None:
    rows = [Row(text="t", label="a")]
    out = evaluate(_M(), _Mdl(), rows, FakeTokenizer(), device="cpu")
    # ``accuracy`` should get ``val/`` prefix, ``val/f1`` kept as-is.
    assert "val/accuracy" in out
    assert "val/f1" in out
    assert out["val/accuracy"] == 0.9


def test_evaluate_switches_to_eval_and_back() -> None:
    model = _Mdl()
    rows = [Row(text="t", label="a")]
    evaluate(_M(), model, rows, FakeTokenizer(), device="cpu")
    assert "eval()" in model.switched
    assert any(s.startswith("train(") for s in model.switched)
