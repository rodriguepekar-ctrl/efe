"""Fine-tuning recipes for the embedder head.

Five methods, each producing an :class:`EmbedderTransform` that can be
plugged in front of the baseline and swarm classifiers:

* :func:`train_linear_probe` — frozen-backbone linear classifier, whose
  logits form the new embedding. Cheapest possible approach — no
  iterative training, analytical solution via ridge regression.

* :func:`train_supcon_mlp` — supervised contrastive learning on a small
  MLP projection head. Pulls same-class samples together, pushes
  different-class apart. Per-batch soft-margin cross-entropy.

* :func:`train_triplet_hnm` — triplet loss with hard-negative mining:
  for each anchor we pick the *hardest* negative = closest same-batch
  sample from a different class. This is the standard recipe that
  separates sibling classes well.

* :func:`train_arcface` — additive angular margin softmax. A tight
  angular margin m is enforced on the correct class. Classical face-
  recognition recipe, very effective on tight boundaries.

* :func:`train_multitask` — joint cross-entropy + supervised contrastive.
  Tests the hypothesis that the two losses together are stronger than
  either alone.

All trainers:

* share the same train/val split (80 / 20 of anchors),
* report training cost (seconds, samples seen),
* return a fully-serialised :class:`EmbedderTransform` (numpy arrays
  only, no torch artefacts leaked),
* are deterministic given a fixed seed.

PyTorch is used for gradient descent only. The resulting transform is
pure numpy so it plugs into the existing evaluation harness with no
runtime dep on torch.
"""

from __future__ import annotations

import time
from collections.abc import Iterable
from dataclasses import dataclass
from typing import Literal

import numpy as np
import torch
import torch.nn as nn
import torch.nn.functional as F

from benchmarks.dataset import CLASS_NAMES, Dataset
from benchmarks.embedders import EmbedderTransform

# Apple Silicon uses MPS, otherwise CPU — stays reproducible either way.
_DEVICE = torch.device(
    "mps" if torch.backends.mps.is_available() else "cpu"
)


# ---------------------------------------------------------------------------
#  Training-set construction
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class TrainSplit:
    X: np.ndarray        # (n_train, input_dim)
    y: np.ndarray        # (n_train,) int class indices
    X_val: np.ndarray    # (n_val, input_dim)
    y_val: np.ndarray    # (n_val,)
    class_names: tuple[str, ...]


def split_anchors(
    ds: Dataset,
    *,
    val_frac: float = 0.2,
    seed: int = 0,
    include_easy_queries: bool = False,
) -> TrainSplit:
    """80/20 split of anchors per class — stratified, deterministic.

    When ``include_easy_queries`` is true, the 250 ``easy`` queries are
    appended to the training set (**not** the val set). This models the
    realistic case where the client has logged, human-labelled examples
    beyond the canonical class anchors — the single biggest driver of
    fine-tuning ROI in practice.
    """
    class_to_idx = {c: i for i, c in enumerate(CLASS_NAMES)}
    rng = np.random.default_rng(seed)
    X_train, y_train, X_val, y_val = [], [], [], []
    for c in CLASS_NAMES:
        mat = ds.anchors[c]  # (n_anchors, dim)
        n = mat.shape[0]
        idx = rng.permutation(n)
        n_val = max(1, int(n * val_frac))
        X_val.append(mat[idx[:n_val]])
        y_val.append(np.full(n_val, class_to_idx[c]))
        X_train.append(mat[idx[n_val:]])
        y_train.append(np.full(n - n_val, class_to_idx[c]))

    if include_easy_queries:
        for q in ds.queries_easy:
            X_train.append(q.embedding[None, :])
            y_train.append(np.array([class_to_idx[q.label]]))

    return TrainSplit(
        X=np.concatenate(X_train, axis=0),
        y=np.concatenate(y_train, axis=0),
        X_val=np.concatenate(X_val, axis=0),
        y_val=np.concatenate(y_val, axis=0),
        class_names=CLASS_NAMES,
    )


# ---------------------------------------------------------------------------
#  Training report
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class TrainingReport:
    method: str
    n_params: int
    n_epochs: int
    n_samples_seen: int
    train_seconds: float
    final_train_loss: float
    final_val_acc: float
    #: Per-epoch diagnostics for plotting.
    history: tuple[dict[str, float], ...] = ()


# ---------------------------------------------------------------------------
#  1. Linear probe — analytic solution
# ---------------------------------------------------------------------------


def train_linear_probe(
    split: TrainSplit,
    *,
    ridge: float = 1.0,
) -> tuple[EmbedderTransform, TrainingReport]:
    """Multiclass ridge logistic via one-hot least-squares — closed form.

    Solves ``min ||XW − Y||² + λ||W||²`` where Y is the one-hot label
    matrix. The learned W acts as the embedder head: ``x → x @ W``.
    The output space has dimension = number of classes. Despite its
    simplicity this is usually the strongest cheap baseline.
    """
    t0 = time.perf_counter()
    X = split.X
    y = split.y
    n_classes = len(split.class_names)
    Y = np.eye(n_classes)[y]  # (n, n_classes)
    # Closed form: W = (XᵀX + λI)⁻¹ Xᵀ Y
    n_features = X.shape[1]
    G = X.T @ X + ridge * np.eye(n_features)
    W = np.linalg.solve(G, X.T @ Y).astype(np.float32)
    dt = time.perf_counter() - t0

    transform = EmbedderTransform(kind="linear", W=W)
    # Evaluate on val.
    Zv = split.X_val @ W
    pred = Zv.argmax(axis=1)
    val_acc = float((pred == split.y_val).mean())
    report = TrainingReport(
        method="linear_probe",
        n_params=int(W.size),
        n_epochs=0,
        n_samples_seen=len(X),
        train_seconds=dt,
        final_train_loss=0.0,
        final_val_acc=val_acc,
    )
    return transform, report


# ---------------------------------------------------------------------------
#  MLP head shared by SupCon / Triplet / ArcFace / Multi-task
# ---------------------------------------------------------------------------


class MlpHead(nn.Module):
    """2-layer MLP with ReLU — used for every torch-based trainer."""

    def __init__(self, input_dim: int, hidden_dim: int, output_dim: int) -> None:
        super().__init__()
        self.fc1 = nn.Linear(input_dim, hidden_dim)
        self.fc2 = nn.Linear(hidden_dim, output_dim)

    def forward(self, x: torch.Tensor) -> torch.Tensor:  # type: ignore[override]
        h = F.relu(self.fc1(x))
        z = self.fc2(h)
        return F.normalize(z, p=2, dim=-1)

    def to_transform(self) -> EmbedderTransform:
        W1 = self.fc1.weight.detach().cpu().numpy().T.astype(np.float32)
        b1 = self.fc1.bias.detach().cpu().numpy().astype(np.float32)
        W2 = self.fc2.weight.detach().cpu().numpy().T.astype(np.float32)
        b2 = self.fc2.bias.detach().cpu().numpy().astype(np.float32)
        return EmbedderTransform(
            kind="mlp", W1=W1, b1=b1, W2=W2, b2=b2, output_norm=True
        )


def _set_seed(seed: int) -> None:
    torch.manual_seed(seed)
    np.random.seed(seed)


def _evaluate_val(
    model: MlpHead, split: TrainSplit, *, anchors_per_class: int | None = None
) -> float:
    """Val accuracy by nearest-class-centroid in the learned space."""
    model.eval()
    with torch.no_grad():
        Xv = torch.tensor(split.X, device=_DEVICE, dtype=torch.float32)
        Xt = torch.tensor(split.X_val, device=_DEVICE, dtype=torch.float32)
        Zv = model(Xv)
        Zt = model(Xt)
        # Class centroids in the learned space.
        n_classes = len(split.class_names)
        centroids = torch.stack([
            Zv[torch.tensor(split.y == c)].mean(dim=0) for c in range(n_classes)
        ])
        centroids = F.normalize(centroids, p=2, dim=-1)
        scores = Zt @ centroids.T  # (n_val, n_classes)
        pred = scores.argmax(dim=1).cpu().numpy()
    model.train()
    return float((pred == split.y_val).mean())


# ---------------------------------------------------------------------------
#  2. Supervised Contrastive (SupCon) — Khosla et al. 2020
# ---------------------------------------------------------------------------


def supcon_loss(
    z: torch.Tensor, y: torch.Tensor, *, temperature: float = 0.1
) -> torch.Tensor:
    """Khosla 2020 — pull same-class together, push apart."""
    # Similarity matrix (n, n), minus diagonal.
    sim = z @ z.T / temperature
    # Mask self-similarities to -inf so they don't count.
    n = z.size(0)
    mask_self = torch.eye(n, dtype=torch.bool, device=z.device)
    sim = sim.masked_fill(mask_self, -1e9)
    # Positive mask: same class AND not self.
    pos_mask = (y.unsqueeze(0) == y.unsqueeze(1)) & ~mask_self
    # log-sum-exp denominator over all non-self samples.
    log_denom = torch.logsumexp(sim, dim=1, keepdim=True)  # (n, 1)
    # Numerator: sum over positives.
    log_prob = sim - log_denom  # (n, n)
    n_pos = pos_mask.sum(dim=1).clamp(min=1).float()  # (n,)
    per_sample = -(log_prob * pos_mask.float()).sum(dim=1) / n_pos
    return per_sample.mean()


def train_supcon_mlp(
    split: TrainSplit,
    *,
    hidden_dim: int = 128,
    output_dim: int = 64,
    epochs: int = 200,
    batch_size: int = 32,
    lr: float = 1e-3,
    weight_decay: float = 1e-4,
    temperature: float = 0.1,
    seed: int = 0,
) -> tuple[EmbedderTransform, TrainingReport]:
    _set_seed(seed)
    model = MlpHead(split.X.shape[1], hidden_dim, output_dim).to(_DEVICE)
    opt = torch.optim.AdamW(model.parameters(), lr=lr, weight_decay=weight_decay)
    t0 = time.perf_counter()
    Xt = torch.tensor(split.X, device=_DEVICE, dtype=torch.float32)
    yt = torch.tensor(split.y, device=_DEVICE, dtype=torch.long)
    n = Xt.size(0)
    history: list[dict[str, float]] = []
    n_samples_seen = 0
    final_loss = float("inf")

    for epoch in range(epochs):
        perm = torch.randperm(n, device=_DEVICE)
        losses = []
        for start in range(0, n, batch_size):
            idx = perm[start : start + batch_size]
            if idx.size(0) < 4:
                continue
            x_b = Xt[idx]
            y_b = yt[idx]
            z_b = model(x_b)
            loss = supcon_loss(z_b, y_b, temperature=temperature)
            opt.zero_grad()
            loss.backward()
            opt.step()
            losses.append(loss.item())
            n_samples_seen += x_b.size(0)
        final_loss = float(np.mean(losses)) if losses else 0.0
        if epoch % 20 == 0 or epoch == epochs - 1:
            val_acc = _evaluate_val(model, split)
            history.append({"epoch": epoch, "loss": final_loss, "val_acc": val_acc})

    dt = time.perf_counter() - t0
    val_acc = _evaluate_val(model, split)
    transform = model.to_transform()
    report = TrainingReport(
        method="supcon_mlp",
        n_params=sum(p.numel() for p in model.parameters()),
        n_epochs=epochs,
        n_samples_seen=n_samples_seen,
        train_seconds=dt,
        final_train_loss=final_loss,
        final_val_acc=val_acc,
        history=tuple(history),
    )
    return transform, report


# ---------------------------------------------------------------------------
#  3. Triplet loss with hard-negative mining
# ---------------------------------------------------------------------------


def _batch_triplets_with_hard_negatives(
    z: torch.Tensor, y: torch.Tensor
) -> tuple[torch.Tensor, torch.Tensor, torch.Tensor]:
    """For each sample pick the hardest negative + a random positive.

    Hardest negative = closest sample with a different class in the
    current batch. Works on the normalised embeddings so cosine=dot.
    """
    n = z.size(0)
    sim = z @ z.T  # (n, n) cosine
    same = y.unsqueeze(0) == y.unsqueeze(1)
    diff = ~same
    mask_self = torch.eye(n, dtype=torch.bool, device=z.device)
    # Hardest negative = argmax over different-class rows.
    neg_sim = sim.masked_fill(same | mask_self, -1e9)
    neg_idx = neg_sim.argmax(dim=1)
    # Positive = random sample with the same class (not self).
    pos_mask = same & ~mask_self
    pos_sim = sim.masked_fill(~pos_mask, -1e9)
    # Random positive per row: use a small noise to randomise ties,
    # then argmax — keeps things deterministic but diverse.
    noise = torch.rand_like(pos_sim) * 1e-3
    pos_idx = (pos_sim + noise).argmax(dim=1)
    # Skip samples whose class has no positive in the batch.
    has_pos = pos_mask.any(dim=1)
    keep = has_pos
    return z[keep], z[pos_idx[keep]], z[neg_idx[keep]]


def train_triplet_hnm(
    split: TrainSplit,
    *,
    hidden_dim: int = 128,
    output_dim: int = 64,
    epochs: int = 200,
    batch_size: int = 48,
    margin: float = 0.2,
    lr: float = 1e-3,
    weight_decay: float = 1e-4,
    seed: int = 0,
) -> tuple[EmbedderTransform, TrainingReport]:
    _set_seed(seed)
    model = MlpHead(split.X.shape[1], hidden_dim, output_dim).to(_DEVICE)
    opt = torch.optim.AdamW(model.parameters(), lr=lr, weight_decay=weight_decay)
    t0 = time.perf_counter()
    Xt = torch.tensor(split.X, device=_DEVICE, dtype=torch.float32)
    yt = torch.tensor(split.y, device=_DEVICE, dtype=torch.long)
    n = Xt.size(0)
    history: list[dict[str, float]] = []
    n_samples_seen = 0
    final_loss = float("inf")

    for epoch in range(epochs):
        perm = torch.randperm(n, device=_DEVICE)
        losses = []
        for start in range(0, n, batch_size):
            idx = perm[start : start + batch_size]
            if idx.size(0) < 6:
                continue
            x_b = Xt[idx]
            y_b = yt[idx]
            z_b = model(x_b)
            a, p, neg = _batch_triplets_with_hard_negatives(z_b, y_b)
            if a.size(0) == 0:
                continue
            # Cosine distance = 1 - cos sim.
            d_pos = 1.0 - (a * p).sum(dim=1)
            d_neg = 1.0 - (a * neg).sum(dim=1)
            loss = F.relu(d_pos - d_neg + margin).mean()
            opt.zero_grad()
            loss.backward()
            opt.step()
            losses.append(loss.item())
            n_samples_seen += x_b.size(0)
        final_loss = float(np.mean(losses)) if losses else 0.0
        if epoch % 20 == 0 or epoch == epochs - 1:
            val_acc = _evaluate_val(model, split)
            history.append({"epoch": epoch, "loss": final_loss, "val_acc": val_acc})

    dt = time.perf_counter() - t0
    val_acc = _evaluate_val(model, split)
    transform = model.to_transform()
    report = TrainingReport(
        method="triplet_hnm",
        n_params=sum(p.numel() for p in model.parameters()),
        n_epochs=epochs,
        n_samples_seen=n_samples_seen,
        train_seconds=dt,
        final_train_loss=final_loss,
        final_val_acc=val_acc,
        history=tuple(history),
    )
    return transform, report


# ---------------------------------------------------------------------------
#  4. ArcFace — additive angular margin softmax
# ---------------------------------------------------------------------------


class ArcFaceHead(nn.Module):
    """ArcFace classification head.

    Computes ``s * cos(θ_yi + m)`` for the correct class and ``s * cos(θ_j)``
    for others, then feeds into cross-entropy.
    """

    def __init__(
        self,
        feat_dim: int,
        n_classes: int,
        *,
        margin: float = 0.35,
        scale: float = 8.0,
    ) -> None:
        super().__init__()
        self.weight = nn.Parameter(torch.randn(n_classes, feat_dim) * 0.01)
        self.margin = margin
        self.scale = scale

    def forward(self, z: torch.Tensor, y: torch.Tensor) -> torch.Tensor:  # type: ignore[override]
        W = F.normalize(self.weight, p=2, dim=-1)
        cos = F.normalize(z, p=2, dim=-1) @ W.T  # (n, n_classes)
        # Clamp to avoid NaN in acos.
        cos = cos.clamp(-1 + 1e-6, 1 - 1e-6)
        theta = torch.acos(cos)
        margin_matrix = torch.zeros_like(cos)
        margin_matrix.scatter_(1, y.unsqueeze(1), self.margin)
        logits = torch.cos(theta + margin_matrix) * self.scale
        return logits


def train_arcface(
    split: TrainSplit,
    *,
    hidden_dim: int = 128,
    output_dim: int = 64,
    epochs: int = 200,
    batch_size: int = 32,
    lr: float = 1e-3,
    weight_decay: float = 1e-4,
    margin: float = 0.35,
    scale: float = 8.0,
    seed: int = 0,
) -> tuple[EmbedderTransform, TrainingReport]:
    _set_seed(seed)
    model = MlpHead(split.X.shape[1], hidden_dim, output_dim).to(_DEVICE)
    head = ArcFaceHead(
        output_dim, len(split.class_names), margin=margin, scale=scale
    ).to(_DEVICE)
    params = list(model.parameters()) + list(head.parameters())
    opt = torch.optim.AdamW(params, lr=lr, weight_decay=weight_decay)
    t0 = time.perf_counter()
    Xt = torch.tensor(split.X, device=_DEVICE, dtype=torch.float32)
    yt = torch.tensor(split.y, device=_DEVICE, dtype=torch.long)
    n = Xt.size(0)
    history: list[dict[str, float]] = []
    n_samples_seen = 0
    final_loss = float("inf")

    for epoch in range(epochs):
        perm = torch.randperm(n, device=_DEVICE)
        losses = []
        for start in range(0, n, batch_size):
            idx = perm[start : start + batch_size]
            if idx.size(0) < 2:
                continue
            x_b = Xt[idx]
            y_b = yt[idx]
            z_b = model(x_b)
            logits = head(z_b, y_b)
            loss = F.cross_entropy(logits, y_b)
            opt.zero_grad()
            loss.backward()
            opt.step()
            losses.append(loss.item())
            n_samples_seen += x_b.size(0)
        final_loss = float(np.mean(losses)) if losses else 0.0
        if epoch % 20 == 0 or epoch == epochs - 1:
            val_acc = _evaluate_val(model, split)
            history.append({"epoch": epoch, "loss": final_loss, "val_acc": val_acc})

    dt = time.perf_counter() - t0
    val_acc = _evaluate_val(model, split)
    transform = model.to_transform()
    total_params = sum(p.numel() for p in params)
    report = TrainingReport(
        method="arcface",
        n_params=total_params,
        n_epochs=epochs,
        n_samples_seen=n_samples_seen,
        train_seconds=dt,
        final_train_loss=final_loss,
        final_val_acc=val_acc,
        history=tuple(history),
    )
    return transform, report


# ---------------------------------------------------------------------------
#  5. Multi-task — joint SupCon + cross-entropy
# ---------------------------------------------------------------------------


def train_multitask(
    split: TrainSplit,
    *,
    hidden_dim: int = 128,
    output_dim: int = 64,
    epochs: int = 200,
    batch_size: int = 32,
    lr: float = 1e-3,
    weight_decay: float = 1e-4,
    temperature: float = 0.1,
    ce_weight: float = 0.5,
    seed: int = 0,
) -> tuple[EmbedderTransform, TrainingReport]:
    _set_seed(seed)
    model = MlpHead(split.X.shape[1], hidden_dim, output_dim).to(_DEVICE)
    classifier = nn.Linear(output_dim, len(split.class_names)).to(_DEVICE)
    params = list(model.parameters()) + list(classifier.parameters())
    opt = torch.optim.AdamW(params, lr=lr, weight_decay=weight_decay)
    t0 = time.perf_counter()
    Xt = torch.tensor(split.X, device=_DEVICE, dtype=torch.float32)
    yt = torch.tensor(split.y, device=_DEVICE, dtype=torch.long)
    n = Xt.size(0)
    history: list[dict[str, float]] = []
    n_samples_seen = 0
    final_loss = float("inf")

    for epoch in range(epochs):
        perm = torch.randperm(n, device=_DEVICE)
        losses = []
        for start in range(0, n, batch_size):
            idx = perm[start : start + batch_size]
            if idx.size(0) < 4:
                continue
            x_b = Xt[idx]
            y_b = yt[idx]
            z_b = model(x_b)
            supcon = supcon_loss(z_b, y_b, temperature=temperature)
            logits = classifier(z_b)
            ce = F.cross_entropy(logits, y_b)
            loss = supcon + ce_weight * ce
            opt.zero_grad()
            loss.backward()
            opt.step()
            losses.append(loss.item())
            n_samples_seen += x_b.size(0)
        final_loss = float(np.mean(losses)) if losses else 0.0
        if epoch % 20 == 0 or epoch == epochs - 1:
            val_acc = _evaluate_val(model, split)
            history.append({"epoch": epoch, "loss": final_loss, "val_acc": val_acc})

    dt = time.perf_counter() - t0
    val_acc = _evaluate_val(model, split)
    transform = model.to_transform()
    total_params = sum(p.numel() for p in params)
    report = TrainingReport(
        method="multitask",
        n_params=total_params,
        n_epochs=epochs,
        n_samples_seen=n_samples_seen,
        train_seconds=dt,
        final_train_loss=final_loss,
        final_val_acc=val_acc,
        history=tuple(history),
    )
    return transform, report


__all__ = [
    "TrainSplit",
    "TrainingReport",
    "split_anchors",
    "train_arcface",
    "train_linear_probe",
    "train_multitask",
    "train_supcon_mlp",
    "train_triplet_hnm",
]
