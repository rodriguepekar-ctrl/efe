"""Method A — InfoNCE (contrastive in-batch negatives).

Loss formula:

    L = − (1/N) · Σᵢ log [ exp(sim(aᵢ, pᵢ) / τ)
                          / Σⱼ exp(sim(aᵢ, pⱼ) / τ) ]

where ``sim(x, y) = x·y / (‖x‖·‖y‖)``. Positives sit on the diagonal
of the ``a @ p.T`` matrix; every other column is a negative.

The model is a :class:`sentence_transformers.SentenceTransformer`
(or any object exposing an ``encode`` method that returns a torch
tensor of shape ``(N, d)``). For portability we wrap the real
sentence-transformers class behind our :class:`Model` protocol via
:class:`_SBertModel`.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from efe_worker.data.samplers.base import BaseSampler, Batch
from efe_worker.data.samplers.pair import PairSampler
from efe_worker.data.snapshot import Row, Snapshot
from efe_worker.data.tokenizer import Tokenizer
from efe_worker.errors import (
    InvalidHyperparameters,
    MissingDependency,
    TrainingDiverged,
)
from efe_worker.methods.base import (
    LossOutput,
    Model,
    ensure_torch,
)


class InfoNCEMethod:
    name = "infonce"

    def __init__(self, hyperparameters: dict[str, Any] | None = None) -> None:
        hp = dict(hyperparameters or {})
        self.temperature = float(hp.get("temperature", 0.05))
        if not (0.0 < self.temperature <= 1.0):
            raise InvalidHyperparameters(
                f"temperature must be in (0, 1], got {self.temperature}",
            )
        self.max_seq_len = int(hp.get("max_seq_len", 128))
        self.normalize = bool(hp.get("normalize_embeddings", True))
        self._mining_strategy = str(hp.get("mining", "in_batch"))

    # ---- build_model -------------------------------------------------

    def build_model(
        self, base_model: str, hyperparameters: dict[str, Any],
    ) -> Model:
        """Load a sentence-transformer style encoder.

        We lazy-import the heavy dependency so the module can be
        imported on machines without it; tests pass their own
        :class:`Model` via the registry mock.
        """
        try:
            from sentence_transformers import SentenceTransformer
        except ImportError as exc:  # pragma: no cover
            raise MissingDependency(
                "sentence-transformers is required for InfoNCE",
            ) from exc
        try:
            inner = SentenceTransformer(base_model)
        except Exception as exc:  # pragma: no cover — network
            raise InvalidHyperparameters(
                f"failed to load base_model {base_model!r}: {exc}",
            ) from exc
        return _SBertModel(inner)

    # ---- build_sampler ----------------------------------------------

    def build_sampler(
        self, snapshot: Snapshot, tokenizer: Tokenizer,
    ) -> BaseSampler:
        del tokenizer  # sentence-transformers tokenises internally
        return PairSampler(snapshot.rows, seed=snapshot.manifest.partitioning.seed)

    # ---- compute_loss ------------------------------------------------

    def compute_loss(
        self, model: Model, batch: Batch, *, step: int, device: Any,
        tokenizer: Tokenizer,
    ) -> LossOutput:
        del tokenizer  # not needed for sentence-transformers
        torch = ensure_torch()
        if not batch.positive_texts:
            raise InvalidHyperparameters(
                "InfoNCE requires batch.positive_texts to be populated",
            )
        F = torch.nn.functional
        # Encode
        anchor = _encode(model, batch.anchor_texts, device)
        positive = _encode(model, batch.positive_texts, device)
        if self.normalize:
            anchor = F.normalize(anchor, dim=-1)
            positive = F.normalize(positive, dim=-1)
        logits = (anchor @ positive.T) / self.temperature
        target = torch.arange(anchor.size(0), device=logits.device)
        loss = F.cross_entropy(logits, target)
        if not torch.isfinite(loss):
            raise TrainingDiverged(
                f"InfoNCE loss became {loss.item()} at step {step}",
            )
        pred = logits.argmax(dim=-1)
        acc = (pred == target).float().mean().item()
        return LossOutput(
            loss=loss,
            scalar=float(loss.item()),
            metrics={"in_batch_acc": float(acc)},
        )

    # ---- eval --------------------------------------------------------

    def eval_metrics(
        self, model: Model, val_rows: list[Row], tokenizer: Tokenizer,
        *, device: Any,
    ) -> dict[str, float]:
        del tokenizer
        if not val_rows:
            return {"recall@1": 0.0, "recall@5": 0.0, "mrr": 0.0}
        torch = ensure_torch()
        # Build a dedicated pair set from val rows by label.
        from efe_worker.data.samplers.base import group_by_label
        groups = group_by_label(val_rows)
        pairs: list[tuple[str, str]] = []
        for lab, idxs in groups.items():
            if not lab or len(idxs) < 2:
                continue
            for i in range(len(idxs) - 1):
                pairs.append((val_rows[idxs[i]].text, val_rows[idxs[i + 1]].text))
        if not pairs:
            return {"recall@1": 0.0, "recall@5": 0.0, "mrr": 0.0}
        anchors = [a for a, _ in pairs]
        positives = [p for _, p in pairs]
        with torch.no_grad():
            a = _encode(model, anchors, device)
            p = _encode(model, positives, device)
            if self.normalize:
                F = torch.nn.functional
                a = F.normalize(a, dim=-1)
                p = F.normalize(p, dim=-1)
            sims = a @ p.T
            n = sims.size(0)
            ranks = []
            for i in range(n):
                diag = sims[i, i]
                rank = (sims[i] > diag).sum().item() + 1
                ranks.append(int(rank))
        r1 = sum(1 for r in ranks if r <= 1) / n
        r5 = sum(1 for r in ranks if r <= 5) / n
        mrr = sum(1.0 / r for r in ranks) / n
        return {
            "recall@1": float(r1),
            "recall@5": float(r5),
            "mrr": float(mrr),
        }

    # ---- save_artifact ----------------------------------------------

    def save_artifact(
        self, model: Model, out_dir: Path, *,
        extra: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        out_dir.mkdir(parents=True, exist_ok=True)
        inner = getattr(model, "_inner", None) or model
        saved_to = None
        if hasattr(inner, "save"):
            try:
                inner.save(str(out_dir))
                saved_to = "sentence_transformers.save"
            except Exception:  # pragma: no cover — defensive
                saved_to = None
        meta = {
            "method": "infonce",
            "format": saved_to or "state_dict",
            "temperature": self.temperature,
            "normalize": self.normalize,
        }
        if extra:
            meta.update(extra)
        (out_dir / "meta.json").write_text(
            json.dumps(meta, indent=2), encoding="utf-8",
        )
        if saved_to is None:
            # Fall back to raw state_dict on the wrapped torch module.
            torch = ensure_torch()
            torch.save(model.state_dict(), str(out_dir / "model.pt"))
        return meta


# ---------------------------------------------------------------------------
# thin adapter wrapping sentence-transformers → our Model Protocol
# ---------------------------------------------------------------------------


class _SBertModel:
    """Minimal wrapper around a SentenceTransformer instance."""

    def __init__(self, inner: Any) -> None:
        self._inner = inner

    def to(self, device: Any) -> _SBertModel:
        self._inner.to(device)
        return self

    def train(self, mode: bool = True) -> _SBertModel:
        self._inner.train(mode)
        return self

    def eval(self) -> _SBertModel:
        self._inner.eval()
        return self

    def parameters(self) -> Any:
        return self._inner.parameters()

    def state_dict(self) -> dict[str, Any]:
        out: dict[str, Any] = dict(self._inner.state_dict())
        return out

    def load_state_dict(
        self, state: dict[str, Any], strict: bool = True,
    ) -> Any:
        return self._inner.load_state_dict(state, strict=strict)


def _encode(model: Model, texts: list[str], device: Any) -> Any:
    """Best-effort encode call supporting both SBert and raw torch."""
    torch = ensure_torch()
    inner = getattr(model, "_inner", None) or model
    # sentence-transformers path
    if hasattr(inner, "encode") and not hasattr(inner, "encode_batch"):
        # Force torch tensor + gradients off when called during eval.
        emb = inner.encode(
            texts, convert_to_tensor=True, show_progress_bar=False,
        )
        return emb.to(device)
    # Generic Model — callers would have attached a ``.forward`` method
    # producing embeddings of shape (N, d).
    if hasattr(inner, "forward"):
        out = inner.forward(texts)
        if isinstance(out, torch.Tensor):
            return out.to(device)
    raise InvalidHyperparameters(
        "model has no .encode() or .forward() method",
    )


__all__ = ["InfoNCEMethod"]
