"""Method B — Triplet loss with semi-hard / hardest mining.

Loss:

    L = (1/N) · Σᵢ max(0, d(aᵢ, pᵢ) − d(aᵢ, nᵢ) + m)

where ``d(x, y) = 1 − cos(x, y)``. The sampler provides raw
(anchor, positive, negative) triplets; the *hardness* refinement
happens in this module: we re-embed all three sets, compute the
pairwise distance matrix within the batch, then replace each sampled
negative by the strongest valid one per the chosen strategy.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

from efe_worker.data.samplers.base import BaseSampler, Batch
from efe_worker.data.samplers.triplet import TripletSampler
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
)
from efe_worker.methods.infonce import _encode

_STRATEGIES = frozenset({"random", "semi_hard", "hardest"})


class TripletMethod:
    name = "triplet"

    def __init__(
        self, hyperparameters: dict[str, Any] | None = None,
    ) -> None:
        hp = dict(hyperparameters or {})
        self.margin = float(hp.get("margin", 0.2))
        if not (0.0 < self.margin < 2.0):
            raise InvalidHyperparameters(
                f"margin must be in (0, 2), got {self.margin}",
            )
        self.strategy = str(hp.get("mining", "semi_hard"))
        if self.strategy not in _STRATEGIES:
            raise InvalidHyperparameters(
                f"unknown triplet mining {self.strategy!r}; "
                f"one of {sorted(_STRATEGIES)}",
            )
        self.sampler_strategy = str(hp.get("sampler_strategy", "random"))

    def build_model(
        self, base_model: str, hyperparameters: dict[str, Any],
    ) -> Model:
        from efe_worker.methods.infonce import InfoNCEMethod
        # Same embedder backend.
        return InfoNCEMethod(hyperparameters).build_model(
            base_model, hyperparameters,
        )

    def build_sampler(
        self, snapshot: Snapshot, tokenizer: Tokenizer,
    ) -> BaseSampler:
        del tokenizer
        return TripletSampler(
            snapshot.rows,
            strategy=self.sampler_strategy,
            seed=snapshot.manifest.partitioning.seed,
        )

    def compute_loss(
        self, model: Model, batch: Batch, *, step: int, device: Any,
        tokenizer: Tokenizer,
    ) -> LossOutput:
        del tokenizer
        torch = ensure_torch()
        F = torch.nn.functional
        if not batch.positive_texts or not batch.negative_texts:
            raise InvalidHyperparameters(
                "Triplet requires positive + negative text in batch",
            )
        a = F.normalize(_encode(model, batch.anchor_texts, device), dim=-1)
        p = F.normalize(_encode(model, batch.positive_texts, device), dim=-1)
        n = F.normalize(_encode(model, batch.negative_texts, device), dim=-1)
        # distances (cos → 1 - cos)
        d_ap = 1.0 - (a * p).sum(dim=-1)
        d_an = 1.0 - (a * n).sum(dim=-1)
        # hardness refinement
        if self.strategy != "random":
            # Among (all p, all n) in the batch, build full distance matrices.
            dm_ap = 1.0 - (a @ p.T)  # (N, N)
            dm_an = 1.0 - (a @ n.T)
            same_class = torch.zeros_like(dm_ap, dtype=torch.bool)
            labels = batch.labels
            if labels:
                tensor_labels = torch.tensor(
                    [hash(lab) for lab in labels],
                    device=dm_ap.device,
                )
                same_class = (
                    tensor_labels[:, None] == tensor_labels[None, :]
                )
            # hardest positive = largest d(a, p) for same class (incl self on diag)
            d_ap_mat = dm_ap.clone()
            d_ap_mat[~same_class] = -float("inf")
            hardest_pos = d_ap_mat.max(dim=1).values
            d_ap = torch.where(
                torch.isfinite(hardest_pos), hardest_pos, d_ap,
            )
            # For negatives: filter candidates by strategy
            d_an_mat = dm_an.clone()
            if self.strategy == "hardest":
                d_an = d_an_mat.min(dim=1).values
            else:  # semi_hard
                # negatives with d(a,n) > d(a,p); smallest among those
                cand = d_an_mat.clone()
                cand[cand <= d_ap.unsqueeze(1)] = float("inf")
                d_an = cand.min(dim=1).values
                # Some rows may have no valid semi-hard → fallback to raw
                fallback = ~torch.isfinite(d_an)
                if fallback.any():
                    raw_dan = 1.0 - (a * n).sum(dim=-1)
                    d_an = torch.where(fallback, raw_dan, d_an)
        loss = F.relu(d_ap - d_an + self.margin).mean()
        if not torch.isfinite(loss):
            raise TrainingDiverged(
                f"Triplet loss became {loss.item()} at step {step}",
            )
        # diagnostics
        violations = (d_ap + self.margin - d_an > 0).float().mean().item()
        avg_dap = float(d_ap.mean().item())
        avg_dan = float(d_an.mean().item())
        return LossOutput(
            loss=loss,
            scalar=float(loss.item()),
            metrics={
                "violation_rate": float(violations),
                "d_ap": avg_dap,
                "d_an": avg_dan,
            },
        )

    def eval_metrics(
        self, model: Model, val_rows: list[Row], tokenizer: Tokenizer,
        *, device: Any,
    ) -> dict[str, float]:
        del tokenizer
        if not val_rows:
            return {"triplet_acc": 0.0}
        torch = ensure_torch()
        from efe_worker.data.samplers.base import group_by_label
        groups = group_by_label(val_rows)
        triplets: list[tuple[str, str, str]] = []
        labels_list = [lab for lab in groups if lab and len(groups[lab]) >= 2]
        if len(labels_list) < 2:
            return {"triplet_acc": 0.0}
        import random as _r
        rng = _r.Random(1234)
        for lab in labels_list:
            idxs = groups[lab]
            for i in range(len(idxs) - 1):
                anchor = val_rows[idxs[i]].text
                positive = val_rows[idxs[i + 1]].text
                # negative from another label
                other_labels = [x for x in labels_list if x != lab]
                o = rng.choice(other_labels)
                negative = val_rows[rng.choice(groups[o])].text
                triplets.append((anchor, positive, negative))
        if not triplets:
            return {"triplet_acc": 0.0}
        with torch.no_grad():
            F = torch.nn.functional
            a = F.normalize(
                _encode(model, [t[0] for t in triplets], device), dim=-1,
            )
            p = F.normalize(
                _encode(model, [t[1] for t in triplets], device), dim=-1,
            )
            n = F.normalize(
                _encode(model, [t[2] for t in triplets], device), dim=-1,
            )
            d_ap = 1.0 - (a * p).sum(dim=-1)
            d_an = 1.0 - (a * n).sum(dim=-1)
            correct = (d_ap + self.margin < d_an).float().mean().item()
        return {"triplet_acc": float(correct)}

    def save_artifact(
        self, model: Model, out_dir: Path, *,
        extra: dict[str, Any] | None = None,
    ) -> dict[str, Any]:
        out_dir.mkdir(parents=True, exist_ok=True)
        inner = getattr(model, "_inner", None) or model
        meta: dict[str, Any] = {
            "method": "triplet",
            "margin": self.margin,
            "strategy": self.strategy,
        }
        if extra:
            meta.update(extra)
        if hasattr(inner, "save"):
            try:
                inner.save(str(out_dir))
                meta["format"] = "sentence_transformers.save"
            except Exception:  # pragma: no cover — defensive
                meta["format"] = "state_dict"
        else:
            meta["format"] = "state_dict"
        if meta.get("format") == "state_dict":
            torch = ensure_torch()
            torch.save(model.state_dict(), str(out_dir / "model.pt"))
        (out_dir / "meta.json").write_text(
            json.dumps(meta, indent=2), encoding="utf-8",
        )
        return meta


__all__ = ["TripletMethod"]
