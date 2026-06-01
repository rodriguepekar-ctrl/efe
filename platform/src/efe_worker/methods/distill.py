"""Method E — Knowledge Distillation (teacher → student).

Loss:

    L = α · T² · KL(σ(z_t / T) || σ(z_s / T))  +  (1 − α) · CE(y, z_s)

Where the student model is the one we're fine-tuning and the teacher
is frozen.
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
    ensure_torch,
    ensure_transformers,
)


class DistillMethod:
    name = "distill"

    def __init__(
        self, hyperparameters: dict[str, Any] | None = None,
    ) -> None:
        hp = dict(hyperparameters or {})
        self.temperature = float(hp.get("temperature", 4.0))
        if not (1.0 <= self.temperature <= 20.0):
            raise InvalidHyperparameters(
                f"distill temperature must be in [1, 20], "
                f"got {self.temperature}",
            )
        self.alpha = float(hp.get("alpha", 0.7))
        if not (0.0 <= self.alpha <= 1.0):
            raise InvalidHyperparameters(
                f"alpha must be in [0, 1], got {self.alpha}",
            )
        self.teacher_model = str(hp.get("teacher_model", ""))
        if not self.teacher_model:
            raise InvalidHyperparameters(
                "distill requires hyperparameters['teacher_model']",
            )
        # Pointer to a pre-trained classification head for the teacher.
        # Without this the KL term is computed against a *randomly
        # initialised* head — the student learns from noise. We keep
        # it optional for backward compatibility but require it by
        # default via ``require_teacher_head``.
        self.teacher_head_uri: str | None = hp.get("teacher_head_uri")
        self.require_teacher_head = bool(
            hp.get("require_teacher_head", True),
        )
        if self.require_teacher_head and not self.teacher_head_uri:
            raise InvalidHyperparameters(
                "distill requires hyperparameters['teacher_head_uri'] "
                "pointing to a pre-trained head (e.g. s3://.../head.pt). "
                "Set require_teacher_head=False only for integration "
                "tests — in production the teacher must be fully "
                "trained.",
            )
        self._label_map: dict[str, int] = {}

    def build_model(
        self, base_model: str, hyperparameters: dict[str, Any],
    ) -> Model:
        torch = ensure_torch()
        transformers = ensure_transformers()
        n_classes = int(hyperparameters.get("n_classes", 0))
        if n_classes < 2:
            raise InvalidHyperparameters(
                f"n_classes must be >= 2 for distill, got {n_classes}",
            )
        teacher = transformers.AutoModel.from_pretrained(self.teacher_model)
        student = transformers.AutoModel.from_pretrained(base_model)
        if teacher is student:
            raise InvalidHyperparameters(
                "teacher and student are the same object",
            )
        teacher.eval()
        for p in teacher.parameters():
            p.requires_grad_(False)
        t_head = torch.nn.Linear(
            int(teacher.config.hidden_size), n_classes,
        )
        s_head = torch.nn.Linear(
            int(student.config.hidden_size), n_classes,
        )
        # Optionally warm-load a pre-trained teacher head from a local
        # file. The URI is expected to be a plain filesystem path (the
        # orchestrator or the driver downloads the blob beforehand).
        # When ``require_teacher_head=False`` we tolerate a random
        # initialisation — useful only for unit tests that exercise
        # the loss machinery without caring about convergence.
        if self.teacher_head_uri:
            self._load_teacher_head(t_head, self.teacher_head_uri, torch)
        elif self.require_teacher_head:  # pragma: no cover — guarded in __init__
            raise InvalidHyperparameters(
                "teacher_head_uri missing and require_teacher_head=True",
            )
        return _TeacherStudent(teacher, t_head, student, s_head)

    @staticmethod
    def _load_teacher_head(
        head: Any, uri: str, torch_mod: Any,
    ) -> None:
        """Warm-load weights into the teacher head.

        Supports ``file://`` URIs and bare paths. Anything else must
        be resolved to a local path by the caller (driver pulls from
        S3 then passes a local path). Any load failure is surfaced as
        :class:`InvalidHyperparameters` so the run fails cleanly
        before spending GPU time on a random teacher.
        """
        path = uri
        if path.startswith("file://"):
            path = path[len("file://"):]
        try:
            state = torch_mod.load(path, map_location="cpu")
            if isinstance(state, dict) and "state_dict" in state:
                state = state["state_dict"]
            head.load_state_dict(state, strict=True)
        except Exception as exc:
            raise InvalidHyperparameters(
                f"failed to load teacher head from {uri!r}: {exc}",
            ) from exc

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
                "Distill requires batch.label_ids",
            )
        enc = tokenizer.encode_batch(batch.anchor_texts)
        ids = torch.tensor(enc.input_ids, device=device, dtype=torch.long)
        mask = torch.tensor(enc.attention_mask, device=device, dtype=torch.long)
        labels = torch.tensor(batch.label_ids, device=device, dtype=torch.long)
        with torch.no_grad():
            t_logits = model.teacher_logits(ids, mask)  # type: ignore[attr-defined]
        s_logits = model.student_logits(ids, mask)  # type: ignore[attr-defined]
        T = self.temperature
        log_s = F.log_softmax(s_logits / T, dim=-1)
        soft_t = F.softmax(t_logits / T, dim=-1)
        kl = F.kl_div(log_s, soft_t, reduction="batchmean")
        ce = F.cross_entropy(s_logits, labels)
        loss = self.alpha * (T * T) * kl + (1.0 - self.alpha) * ce
        if not torch.isfinite(loss):
            raise TrainingDiverged(
                f"distill loss became {loss.item()} at step {step}",
            )
        acc = (s_logits.argmax(-1) == labels).float().mean().item()
        teacher_conf = float(soft_t.max(dim=-1).values.mean().item())
        return LossOutput(
            loss=loss,
            scalar=float(loss.item()),
            metrics={
                "kl": float(kl.item()),
                "ce": float(ce.item()),
                "accuracy": float(acc),
                "teacher_conf": teacher_conf,
            },
        )

    def eval_metrics(
        self, model: Model, val_rows: list[Row], tokenizer: Tokenizer,
        *, device: Any,
    ) -> dict[str, float]:
        if not val_rows:
            return {"student_acc": 0.0, "teacher_acc": 0.0, "agreement": 0.0}
        torch = ensure_torch()
        total = 0
        s_correct = 0
        t_correct = 0
        agree = 0
        batch = 32
        with torch.no_grad():
            for i in range(0, len(val_rows), batch):
                chunk = val_rows[i : i + batch]
                texts = [r.text for r in chunk]
                labels = [
                    self._label_map.get(r.label or "", 0) for r in chunk
                ]
                enc = tokenizer.encode_batch(texts)
                ids = torch.tensor(
                    enc.input_ids, device=device, dtype=torch.long,
                )
                mask = torch.tensor(
                    enc.attention_mask, device=device, dtype=torch.long,
                )
                s_logits = model.student_logits(ids, mask)  # type: ignore[attr-defined]
                t_logits = model.teacher_logits(ids, mask)  # type: ignore[attr-defined]
                s_pred = s_logits.argmax(-1).tolist()
                t_pred = t_logits.argmax(-1).tolist()
                for sp, tp, y in zip(s_pred, t_pred, labels, strict=False):
                    total += 1
                    if sp == y:
                        s_correct += 1
                    if tp == y:
                        t_correct += 1
                    if sp == tp:
                        agree += 1
        return {
            "student_acc": float(s_correct) / max(1, total),
            "teacher_acc": float(t_correct) / max(1, total),
            "agreement": float(agree) / max(1, total),
        }

    def save_artifact(
        self, model: Model, out_dir: Path, *,
        extra: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        torch = ensure_torch()
        out_dir.mkdir(parents=True, exist_ok=True)
        torch.save(model.state_dict(), str(out_dir / "student.pt"))
        meta: dict[str, Any] = {
            "method": "distill",
            "format": "state_dict",
            "teacher_model": self.teacher_model,
            "temperature": self.temperature,
            "alpha": self.alpha,
            "label_map": dict(self._label_map),
        }
        if extra:
            meta.update(extra)
        (out_dir / "meta.json").write_text(
            json.dumps(meta, indent=2), encoding="utf-8",
        )
        return meta


class _TeacherStudent:
    """Teacher (frozen) + student pair; our :class:`Model` protocol."""

    def __init__(
        self, teacher: Any, t_head: Any, student: Any, s_head: Any,
    ) -> None:
        self._teacher = teacher
        self._t_head = t_head
        self._student = student
        self._s_head = s_head

    def teacher_logits(self, ids: Any, mask: Any) -> Any:
        out = self._teacher(input_ids=ids, attention_mask=mask)
        return self._t_head(out.last_hidden_state[:, 0, :])

    def student_logits(self, ids: Any, mask: Any) -> Any:
        out = self._student(input_ids=ids, attention_mask=mask)
        return self._s_head(out.last_hidden_state[:, 0, :])

    def to(self, device: Any) -> _TeacherStudent:
        self._teacher.to(device)
        self._t_head.to(device)
        self._student.to(device)
        self._s_head.to(device)
        return self

    def train(self, mode: bool = True) -> _TeacherStudent:
        # Teacher stays frozen in eval forever.
        self._teacher.eval()
        self._t_head.eval()
        self._student.train(mode)
        self._s_head.train(mode)
        return self

    def eval(self) -> _TeacherStudent:
        self._teacher.eval()
        self._t_head.eval()
        self._student.eval()
        self._s_head.eval()
        return self

    def parameters(self) -> Any:
        # Only return student + s_head (what gets optimised).
        return [*self._student.parameters(), *self._s_head.parameters()]

    def state_dict(self) -> dict[str, Any]:
        return {
            "student": self._student.state_dict(),
            "s_head": self._s_head.state_dict(),
        }

    def load_state_dict(
        self, state: dict[str, Any], strict: bool = True,
    ) -> Any:
        self._student.load_state_dict(state["student"], strict=strict)
        self._s_head.load_state_dict(state["s_head"], strict=strict)


__all__ = ["DistillMethod"]
