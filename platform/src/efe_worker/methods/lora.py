"""Method D — LoRA PEFT.

We add :class:`peft.LoraConfig` adapters on top of a frozen base
model and forward cross-entropy on a classification head. The
artifact we save is **only the adapters**, not the full model —
which is the entire point of LoRA.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from efe_worker.data.samplers.base import BaseSampler, Batch, label_index_map
from efe_worker.data.samplers.raw import RawSampler
from efe_worker.data.snapshot import Row, Snapshot
from efe_worker.data.tokenizer import Tokenizer
from efe_worker.errors import (
    InvalidHyperparameters,
    TrainingDiverged,
)
from efe_worker.methods.base import (
    LossOutput,
    Model,
    ensure_peft,
    ensure_torch,
    ensure_transformers,
)


class LoRAMethod:
    name = "lora"

    def __init__(
        self, hyperparameters: dict[str, Any] | None = None,
    ) -> None:
        hp = dict(hyperparameters or {})
        self.r = int(hp.get("r", 8))
        if not (1 <= self.r <= 64):
            raise InvalidHyperparameters(
                f"LoRA r must be 1..64, got {self.r}",
            )
        self.alpha = int(hp.get("alpha", 2 * self.r))
        self.lora_dropout = float(hp.get("lora_dropout", 0.05))
        tm = hp.get("target_modules", ("q_proj", "v_proj"))
        if isinstance(tm, str):
            if tm != "all-linear":
                raise InvalidHyperparameters(
                    f"target_modules string {tm!r} is not 'all-linear'",
                )
            self.target_modules: Any = tm
        else:
            self.target_modules = tuple(tm)
            if not self.target_modules:
                raise InvalidHyperparameters(
                    "target_modules must contain ≥ 1 entry",
                )
        self._label_map: dict[str, int] = {}

    def build_model(
        self, base_model: str, hyperparameters: dict[str, Any],
    ) -> Model:
        torch = ensure_torch()
        transformers = ensure_transformers()
        peft = ensure_peft()
        n_classes = int(hyperparameters.get("n_classes", 0))
        if n_classes < 2:
            raise InvalidHyperparameters(
                f"n_classes must be >= 2 for LoRA classification head, "
                f"got {n_classes}",
            )
        try:
            encoder = transformers.AutoModel.from_pretrained(base_model)
        except Exception as exc:  # pragma: no cover — network
            raise InvalidHyperparameters(
                f"could not load {base_model!r}: {exc}",
            ) from exc
        cfg_kwargs: dict[str, Any] = {
            "r": self.r,
            "lora_alpha": self.alpha,
            "lora_dropout": self.lora_dropout,
            "bias": "none",
            "task_type": "FEATURE_EXTRACTION",
        }
        if self.target_modules == "all-linear":
            cfg_kwargs["target_modules"] = "all-linear"
        else:
            cfg_kwargs["target_modules"] = list(self.target_modules)
        lora_cfg = peft.LoraConfig(**cfg_kwargs)
        wrapped = peft.get_peft_model(encoder, lora_cfg)
        head = torch.nn.Linear(
            int(wrapped.base_model.config.hidden_size), n_classes,
        )
        return _LoRAWrappedModel(wrapped, head)

    def build_sampler(
        self, snapshot: Snapshot, tokenizer: Tokenizer,
    ) -> BaseSampler:
        del tokenizer
        self._label_map = label_index_map(snapshot.rows)
        return RawSampler(
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
                "LoRA (classification flavour) requires labels",
            )
        enc = tokenizer.encode_batch(batch.anchor_texts)
        ids = torch.tensor(enc.input_ids, device=device, dtype=torch.long)
        mask = torch.tensor(enc.attention_mask, device=device, dtype=torch.long)
        labels = torch.tensor(batch.label_ids, device=device, dtype=torch.long)
        logits = model(ids, mask)  # type: ignore[operator]
        loss = F.cross_entropy(logits, labels)
        if not torch.isfinite(loss):
            raise TrainingDiverged(
                f"LoRA loss became {loss.item()} at step {step}",
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
        # Reuse the classif eval logic — same shape.
        from efe_worker.methods.classif import ClassifMethod
        c = ClassifMethod({
            "dropout": 0.0, "label_smoothing": 0.0, "pool": "cls",
        })
        c._label_map = self._label_map
        return c.eval_metrics(model, val_rows, tokenizer, device=device)

    def save_artifact(
        self, model: Model, out_dir: Path, *,
        extra: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        """Persist the LoRA adapters AND the classification head.

        Two-file layout when the PEFT path is taken:

            out_dir/
              lora_adapter/    (saved by peft.save_pretrained)
              head.pt          (classification head state_dict)
              meta.json

        Without ``head.pt`` downstream inference (M06) would see the
        adapters but have no way to reproduce the classification head
        — a silent miss that would return random-looking logits.
        """
        out_dir.mkdir(parents=True, exist_ok=True)
        torch = ensure_torch()
        inner = getattr(model, "_wrapped", None)
        head = getattr(model, "_head", None)
        saved_to: str
        if inner is not None and hasattr(inner, "save_pretrained"):
            # PEFT only saves adapters; base model stays remote.
            inner.save_pretrained(str(out_dir / "lora_adapter"))
            saved_to = "peft.save_pretrained"
            # Persist the classification head next to the adapters.
            # Without this the inference pipeline can't reconstruct the
            # full model.
            if head is not None:
                torch.save(head.state_dict(), str(out_dir / "head.pt"))
        else:
            torch.save(model.state_dict(), str(out_dir / "model.pt"))
            saved_to = "state_dict"
        meta: dict[str, Any] = {
            "method": "lora",
            "format": saved_to,
            "head_file": "head.pt" if (
                saved_to == "peft.save_pretrained" and head is not None
            ) else None,
            "r": self.r,
            "alpha": self.alpha,
            "target_modules": (
                list(self.target_modules)
                if isinstance(self.target_modules, tuple)
                else self.target_modules
            ),
            "label_map": dict(self._label_map),
        }
        if extra:
            meta.update(extra)
        (out_dir / "meta.json").write_text(
            json.dumps(meta, indent=2), encoding="utf-8",
        )
        return meta


class _LoRAWrappedModel:
    """PEFT wrapped encoder + classification head."""

    def __init__(self, wrapped: Any, head: Any) -> None:
        self._wrapped = wrapped
        self._head = head

    def __call__(self, input_ids: Any, attention_mask: Any) -> Any:
        out = self._wrapped(
            input_ids=input_ids, attention_mask=attention_mask,
        )
        # CLS pool (first token).
        pooled = out.last_hidden_state[:, 0, :]
        return self._head(pooled)

    def to(self, device: Any) -> _LoRAWrappedModel:
        self._wrapped.to(device)
        self._head.to(device)
        return self

    def train(self, mode: bool = True) -> _LoRAWrappedModel:
        self._wrapped.train(mode)
        self._head.train(mode)
        return self

    def eval(self) -> _LoRAWrappedModel:
        self._wrapped.eval()
        self._head.eval()
        return self

    def parameters(self) -> Any:
        return [*self._wrapped.parameters(), *self._head.parameters()]

    def trainable_parameters(self) -> list[Any]:
        """Only return params with requires_grad=True (the adapter weights)."""
        out = []
        for p in self._wrapped.parameters():
            if getattr(p, "requires_grad", False):
                out.append(p)
        out.extend(self._head.parameters())
        return out

    def state_dict(self) -> dict[str, Any]:
        return {
            "wrapped": self._wrapped.state_dict(),
            "head": self._head.state_dict(),
        }

    def load_state_dict(
        self, state: dict[str, Any], strict: bool = True,
    ) -> Any:
        self._wrapped.load_state_dict(state["wrapped"], strict=strict)
        self._head.load_state_dict(state["head"], strict=strict)


__all__ = ["LoRAMethod"]
