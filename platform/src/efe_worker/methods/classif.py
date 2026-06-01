"""Method C — Classification with cross-entropy.

Architecture: a HuggingFace auto-encoder produces ``[CLS]`` or mean
pooled embeddings; a linear head projects to ``n_classes`` logits;
we optimise with cross-entropy + label smoothing.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from efe_worker.data.samplers.base import BaseSampler, Batch, label_index_map
from efe_worker.data.samplers.class_balanced import ClassBalancedSampler
from efe_worker.data.snapshot import Row, Snapshot
from efe_worker.data.tokenizer import Tokenizer
from efe_worker.errors import (
    InvalidHyperparameters,
    TrainingDiverged,
)
from efe_worker.methods.base import (
    LossOutput,
    Model,
    ensure_torch,
    ensure_transformers,
)


class ClassifMethod:
    name = "classif"

    def __init__(
        self, hyperparameters: dict[str, Any] | None = None,
    ) -> None:
        hp = dict(hyperparameters or {})
        self.dropout = float(hp.get("dropout", 0.1))
        self.label_smoothing = float(hp.get("label_smoothing", 0.1))
        self.pool = str(hp.get("pool", "cls"))
        if self.pool not in ("cls", "mean", "max"):
            raise InvalidHyperparameters(
                f"unknown pool strategy {self.pool!r}",
            )
        self._hparams = hp
        self._label_map: dict[str, int] = {}

    def build_model(
        self, base_model: str, hyperparameters: dict[str, Any],
    ) -> Model:
        torch = ensure_torch()
        transformers = ensure_transformers()
        n_classes = int(hyperparameters.get("n_classes", 0))
        if n_classes < 2:
            raise InvalidHyperparameters(
                f"n_classes must be >= 2, got {n_classes}",
            )
        try:
            encoder = transformers.AutoModel.from_pretrained(base_model)
        except Exception as exc:  # pragma: no cover — network
            raise InvalidHyperparameters(
                f"could not load {base_model!r}: {exc}",
            ) from exc
        hidden = int(encoder.config.hidden_size)
        head = torch.nn.Sequential(
            torch.nn.Dropout(self.dropout),
            torch.nn.Linear(hidden, n_classes),
        )
        return _EncoderPlusHead(encoder, head, pool=self.pool)

    def build_sampler(
        self, snapshot: Snapshot, tokenizer: Tokenizer,
    ) -> BaseSampler:
        del tokenizer
        self._label_map = label_index_map(snapshot.rows)
        return ClassBalancedSampler(
            snapshot.rows, seed=snapshot.manifest.partitioning.seed,
        )

    def compute_loss(
        self, model: Model, batch: Batch, *, step: int, device: Any,
        tokenizer: Tokenizer,
    ) -> LossOutput:
        torch = ensure_torch()
        F = torch.nn.functional
        if not batch.label_ids:
            raise InvalidHyperparameters(
                "Classif method requires batch.label_ids",
            )
        enc = tokenizer.encode_batch(batch.anchor_texts)
        ids = torch.tensor(enc.input_ids, device=device, dtype=torch.long)
        mask = torch.tensor(enc.attention_mask, device=device, dtype=torch.long)
        labels = torch.tensor(batch.label_ids, device=device, dtype=torch.long)
        logits = model(ids, mask)  # type: ignore[operator]
        loss = F.cross_entropy(
            logits, labels, label_smoothing=self.label_smoothing,
        )
        if not torch.isfinite(loss):
            raise TrainingDiverged(
                f"Classif loss became {loss.item()} at step {step}",
            )
        acc = (logits.argmax(-1) == labels).float().mean().item()
        return LossOutput(
            loss=loss,
            scalar=float(loss.item()),
            metrics={"accuracy": float(acc)},
        )

    def eval_metrics(
        self, model: Model, val_rows: list[Row], tokenizer: Tokenizer,
        *, device: Any,
    ) -> dict[str, float]:
        if not val_rows:
            return {"accuracy": 0.0, "macro_f1": 0.0}
        torch = ensure_torch()
        correct = 0
        total = 0
        # For macro F1 we need per-class TP, FP, FN.
        n_classes = len(self._label_map)
        tp = [0] * n_classes
        fp = [0] * n_classes
        fn = [0] * n_classes
        # Batch of 32 for eval.
        batch = 32
        with torch.no_grad():
            for i in range(0, len(val_rows), batch):
                chunk = val_rows[i : i + batch]
                texts = [r.text for r in chunk]
                labels = [
                    self._label_map.get(r.label or "", 0) for r in chunk
                ]
                enc = tokenizer.encode_batch(texts)
                ids = torch.tensor(enc.input_ids, device=device, dtype=torch.long)
                mask = torch.tensor(
                    enc.attention_mask, device=device, dtype=torch.long,
                )
                logits = model(ids, mask)  # type: ignore[operator]
                preds = logits.argmax(-1).tolist()
                for p, y in zip(preds, labels, strict=False):
                    total += 1
                    if p == y:
                        correct += 1
                        tp[y] += 1
                    else:
                        fp[p] += 1
                        fn[y] += 1
        acc = correct / max(1, total)
        f1_per = []
        for c in range(n_classes):
            pr = tp[c] / max(1, tp[c] + fp[c])
            rc = tp[c] / max(1, tp[c] + fn[c])
            f1 = 0.0 if pr + rc == 0 else 2 * pr * rc / (pr + rc)
            f1_per.append(f1)
        macro_f1 = sum(f1_per) / max(1, len(f1_per))
        return {"accuracy": float(acc), "macro_f1": float(macro_f1)}

    def save_artifact(
        self, model: Model, out_dir: Path, *,
        extra: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        torch = ensure_torch()
        out_dir.mkdir(parents=True, exist_ok=True)
        torch.save(model.state_dict(), str(out_dir / "model.pt"))
        meta: dict[str, Any] = {
            "method": "classif",
            "dropout": self.dropout,
            "label_smoothing": self.label_smoothing,
            "pool": self.pool,
            "label_map": dict(self._label_map),
        }
        if extra:
            meta.update(extra)
        (out_dir / "meta.json").write_text(
            json.dumps(meta, indent=2), encoding="utf-8",
        )
        return meta


class _EncoderPlusHead:
    """Encoder + linear head; implements our :class:`Model` protocol."""

    def __init__(self, encoder: Any, head: Any, *, pool: str) -> None:
        self._encoder = encoder
        self._head = head
        self._pool = pool

    def __call__(self, input_ids: Any, attention_mask: Any) -> Any:
        torch = ensure_torch()
        out = self._encoder(input_ids=input_ids, attention_mask=attention_mask)
        hidden = out.last_hidden_state  # (N, T, d)
        if self._pool == "cls":
            pooled = hidden[:, 0, :]
        elif self._pool == "mean":
            m = attention_mask.unsqueeze(-1).float()
            pooled = (hidden * m).sum(dim=1) / m.sum(dim=1).clamp(min=1.0)
        else:  # max
            m = attention_mask.unsqueeze(-1).bool()
            hidden = hidden.masked_fill(~m, float("-inf"))
            pooled = hidden.max(dim=1).values
        del torch
        return self._head(pooled)

    def to(self, device: Any) -> _EncoderPlusHead:
        self._encoder.to(device)
        self._head.to(device)
        return self

    def train(self, mode: bool = True) -> _EncoderPlusHead:
        self._encoder.train(mode)
        self._head.train(mode)
        return self

    def eval(self) -> _EncoderPlusHead:
        self._encoder.eval()
        self._head.eval()
        return self

    def parameters(self) -> Any:
        torch = ensure_torch()
        return [*self._encoder.parameters(), *self._head.parameters()]

    def state_dict(self) -> dict[str, Any]:
        return {
            "encoder": self._encoder.state_dict(),
            "head": self._head.state_dict(),
        }

    def load_state_dict(
        self, state: dict[str, Any], strict: bool = True,
    ) -> Any:
        self._encoder.load_state_dict(state["encoder"], strict=strict)
        self._head.load_state_dict(state["head"], strict=strict)


__all__ = ["ClassifMethod"]
