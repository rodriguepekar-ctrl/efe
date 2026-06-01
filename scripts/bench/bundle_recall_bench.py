#!/usr/bin/env python3
# SPDX-License-Identifier: UNLICENSED
"""Sprint **Φ-C D26 (Bundle embedder réel)** — recall@1 benchmark.

Cf. `docs/EFE_BUNDLE_EMBEDDER_REAL_ARCHITECTURE.md` § 7.1.

## Mission

Mesurer le recall@1 d'un bundle ONNX par rapport à la **baseline
sentence-transformers Python** (= ground truth). Source primaire de
2 SLO :

- **SLO-BUNDLE-RECALL** (cible ≥ 0.92 aggregate FR+EN+DE+ES)
- **SLO-BUNDLE-LINGUAL** (cible parité max/min ≤ 1.05×)

## Méthodologie

1. Charge l'eval set : 1 000 phrases × 4 langues = 4 000 entries
   (= `eval/golden/multilingual_recall_v1.0.jsonl`).
2. Pour chaque phrase, encode 2 fois :
   - Via le **bundle ONNX** (= ce qui sera shipped en prod)
   - Via **sentence-transformers Python** (= baseline ground-truth)
3. Pour chaque phrase i, trouve le top-1 nearest neighbor parmi
   les 4 000 phrases via cosine, **séparément** pour bundle et
   baseline.
4. `recall@1` = % des phrases où le top-1 NN bundle == top-1 NN
   baseline.

## Output

```json
{
  "bundle_id": "paraphrase-multilingual-MiniLM-L12-v2",
  "version": "1.0.0",
  "manifest_sha256": "abc123...",
  "eval_set": "eval/golden/multilingual_recall_v1.0.jsonl",
  "n_phrases": 4000,
  "recall_at_1": 0.927,
  "recall_at_1_per_lang": {
    "fr": 0.918,
    "en": 0.934,
    "de": 0.921,
    "es": 0.925
  },
  "parity_ratio": 1.017,
  "cosine_distribution": {
    "mean": 0.987,
    "p5": 0.962,
    "p50": 0.991,
    "p99": 0.998
  },
  "slo_compliance": {
    "SLO-BUNDLE-RECALL": {"measured_ratio": 0.927, "passed": true},
    "SLO-BUNDLE-LINGUAL": {"measured_ratio": 1.017, "passed": true}
  }
}
```

## Usage

```bash
# Bench end-to-end (= requires GPU optional, CPU OK)
$ python scripts/bench/bundle_recall_bench.py \
    --bundle ./bundles/paraphrase-multilingual-MiniLM-L12-v2-1.0.0/ \
    --eval-set eval/golden/multilingual_recall_v1.0.jsonl \
    --output target/bundle_recall_report.json

# Vs un model HF specific (= sanity-check sample)
$ python scripts/bench/bundle_recall_bench.py \
    --bundle ./bundles/all-MiniLM-L6-v2-en-1.0.0/ \
    --eval-set eval/golden/multilingual_recall_v1.0_sample.jsonl \
    --output /tmp/sample_recall.json \
    --langs en
```

## Pin tests

- `test_recall_at_1_calculation` — pure-function logic.
- `test_parity_ratio_calculation`.
- `test_skip_languages_filter`.
"""

from __future__ import annotations

import argparse
import json
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

# ──────────────────────────────────────────────────────────────────
# Types
# ──────────────────────────────────────────────────────────────────


@dataclass
class EvalEntry:
    """One row of the eval JSONL."""
    text: str
    language: str
    canonical_id: int


@dataclass
class RecallReport:
    """Aggregated bench output."""
    bundle_id: str
    version: str
    manifest_sha256: str
    eval_set: str
    n_phrases: int
    recall_at_1: float
    recall_at_1_per_lang: dict[str, float] = field(default_factory=dict)
    parity_ratio: float = 0.0
    cosine_mean: float = 0.0
    cosine_p5: float = 0.0
    cosine_p50: float = 0.0
    cosine_p99: float = 0.0
    slo_compliance: dict[str, dict[str, Any]] = field(default_factory=dict)


# ──────────────────────────────────────────────────────────────────
# Pure-function helpers (testable sans ONNX)
# ──────────────────────────────────────────────────────────────────


def compute_recall_at_1(
    bundle_neighbors: list[int],
    baseline_neighbors: list[int],
) -> float:
    """Pour chaque index i, vérifie que bundle_neighbors[i] ==
    baseline_neighbors[i]. Retourne le ratio des matches.

    Pure function — testable sans ONNX runtime.
    """
    if len(bundle_neighbors) != len(baseline_neighbors):
        raise ValueError(
            f"length mismatch: bundle={len(bundle_neighbors)} "
            f"baseline={len(baseline_neighbors)}"
        )
    if not bundle_neighbors:
        return 0.0
    matches = sum(1 for b, p in zip(bundle_neighbors, baseline_neighbors) if b == p)
    return matches / len(bundle_neighbors)


def compute_parity_ratio(recall_per_lang: dict[str, float]) -> float:
    """max(recall) / min(recall). Pin contractuel SLO-BUNDLE-LINGUAL.

    Pure function. Retourne `inf` si min == 0.
    """
    if not recall_per_lang:
        return 1.0
    values = list(recall_per_lang.values())
    mn, mx = min(values), max(values)
    if mn == 0.0:
        return float("inf")
    return mx / mn


def compute_cosine_distribution(cosines: list[float]) -> dict[str, float]:
    """Calcule mean / p5 / p50 / p99 d'un array de cosines.

    Pure function. Utilise numpy si disponible, sinon Python pur.
    """
    if not cosines:
        return {"mean": 0.0, "p5": 0.0, "p50": 0.0, "p99": 0.0}
    try:
        import numpy as np    # type: ignore
        arr = np.array(cosines, dtype=np.float64)
        return {
            "mean": float(np.mean(arr)),
            "p5":   float(np.percentile(arr, 5)),
            "p50":  float(np.percentile(arr, 50)),
            "p99":  float(np.percentile(arr, 99)),
        }
    except ImportError:
        sorted_c = sorted(cosines)
        n = len(sorted_c)
        return {
            "mean": sum(sorted_c) / n,
            "p5":   sorted_c[max(0, int(n * 0.05) - 1)],
            "p50":  sorted_c[n // 2],
            "p99":  sorted_c[max(0, int(n * 0.99) - 1)],
        }


def filter_by_languages(entries: list[EvalEntry], langs: list[str] | None) -> list[EvalEntry]:
    """Filtre par langue. Retourne tout si `langs` est None."""
    if langs is None:
        return entries
    keep = set(langs)
    return [e for e in entries if e.language in keep]


# ──────────────────────────────────────────────────────────────────
# I/O eval set
# ──────────────────────────────────────────────────────────────────


def load_eval_set(path: Path) -> list[EvalEntry]:
    """Lit un JSONL et retourne les entries.

    Schema attendu :
    ```json
    {"text": "...", "language": "fr", "canonical_id": 42}
    ```
    """
    out: list[EvalEntry] = []
    with path.open() as f:
        for i, line in enumerate(f):
            line = line.strip()
            if not line:
                continue
            try:
                obj = json.loads(line)
                out.append(
                    EvalEntry(
                        text=obj["text"],
                        language=obj["language"],
                        canonical_id=int(obj.get("canonical_id", i)),
                    )
                )
            except (json.JSONDecodeError, KeyError) as e:  # pragma: no cover
                sys.stderr.write(f"Skipping line {i}: {e}\n")
    return out


# ──────────────────────────────────────────────────────────────────
# ONNX inference (= heavy, gated par --skip-onnx pour tests)
# ──────────────────────────────────────────────────────────────────


def encode_via_bundle(bundle_dir: Path, texts: list[str]) -> list[list[float]]:
    """Encode via le bundle ONNX. Requires onnxruntime + tokenizers.

    Retourne `[embedding × N]` où chaque embedding est `List[float]`
    de dim manifest.dim (typiquement 384).
    """
    try:
        import onnxruntime as ort   # type: ignore
        from tokenizers import Tokenizer    # type: ignore
        import numpy as np    # type: ignore
    except ImportError as e:  # pragma: no cover
        raise ImportError(
            f"Required deps missing for ONNX bench: {e}. "
            f"Install: pip install onnxruntime tokenizers numpy"
        )

    onnx_path = bundle_dir / "weights" / "model.onnx"
    tok_path = bundle_dir / "tokenizer" / "tokenizer.json"
    if not onnx_path.exists():
        raise FileNotFoundError(f"Bundle ONNX missing: {onnx_path}")
    if not tok_path.exists():
        raise FileNotFoundError(f"Bundle tokenizer missing: {tok_path}")

    sess = ort.InferenceSession(str(onnx_path), providers=["CPUExecutionProvider"])
    tok = Tokenizer.from_file(str(tok_path))

    out: list[list[float]] = []
    for text in texts:
        enc = tok.encode(text)
        input_ids = np.array([enc.ids], dtype=np.int64)
        attention_mask = np.array([enc.attention_mask], dtype=np.int64)
        outputs = sess.run(
            None,
            {"input_ids": input_ids, "attention_mask": attention_mask},
        )
        # Mean pooling over seq dim + L2 normalize
        token_embeddings = outputs[0][0]    # shape (seq, dim)
        mask = np.expand_dims(enc.attention_mask, axis=-1).astype(np.float32)
        pooled = (token_embeddings * mask).sum(axis=0) / mask.sum(axis=0).clip(min=1e-9)
        norm = float(np.linalg.norm(pooled))
        if norm > 0:
            pooled = pooled / norm
        out.append(pooled.tolist())
    return out


def encode_via_baseline(model_id: str, texts: list[str]) -> list[list[float]]:
    """Encode via sentence-transformers Python.

    Requires sentence-transformers (= heavy ; CI mode only).
    """
    try:
        from sentence_transformers import SentenceTransformer    # type: ignore
    except ImportError as e:  # pragma: no cover
        raise ImportError(
            f"sentence-transformers missing: {e}. "
            f"Install: pip install sentence-transformers"
        )
    model = SentenceTransformer(model_id)
    embeddings = model.encode(texts, normalize_embeddings=True)
    return [e.tolist() for e in embeddings]


def top1_nearest(query: list[float], corpus: list[list[float]], exclude_self_idx: int) -> int:
    """Trouve l'index du top-1 nearest neighbor (cosine) dans
    `corpus`, en excluant `exclude_self_idx`. Retourne `-1` si vide.
    """
    try:
        import numpy as np    # type: ignore
        q = np.array(query)
        c = np.array(corpus)
        # Cosines (= dot product car déjà normalisé)
        sims = c @ q
        sims[exclude_self_idx] = -float("inf")    # exclude self
        return int(np.argmax(sims))
    except ImportError:
        # Fallback pure Python (= très lent pour > 100 corpus)
        best_idx = -1
        best_sim = -float("inf")
        for i, c in enumerate(corpus):
            if i == exclude_self_idx:
                continue
            sim = sum(a * b for a, b in zip(query, c))
            if sim > best_sim:
                best_sim = sim
                best_idx = i
        return best_idx


# ──────────────────────────────────────────────────────────────────
# Main bench
# ──────────────────────────────────────────────────────────────────


def run_bench(
    bundle_dir: Path,
    eval_set_path: Path,
    output_path: Path,
    langs: list[str] | None = None,
    baseline_model_id: str | None = None,
    skip_baseline: bool = False,
) -> RecallReport:
    """Pipeline bench complet."""
    entries = load_eval_set(eval_set_path)
    entries = filter_by_languages(entries, langs)
    if not entries:
        raise ValueError(f"No entries after filter (langs={langs})")

    texts = [e.text for e in entries]

    # Read bundle manifest pour le report
    manifest_path = bundle_dir / "manifest.json"
    manifest = json.loads(manifest_path.read_text()) if manifest_path.exists() else {}

    # Encode via bundle
    sys.stderr.write(f"Encoding {len(texts)} texts via bundle...\n")
    bundle_embs = encode_via_bundle(bundle_dir, texts)

    # Encode via baseline (= skip optionnellement pour smoke test)
    if skip_baseline:
        sys.stderr.write("--skip-baseline : using bundle as self-baseline\n")
        baseline_embs = bundle_embs
    else:
        model_id = baseline_model_id or manifest.get("model_origin", {}).get("repo", "sentence-transformers/paraphrase-multilingual-MiniLM-L12-v2")
        sys.stderr.write(f"Encoding {len(texts)} texts via baseline {model_id}...\n")
        baseline_embs = encode_via_baseline(model_id, texts)

    # Top-1 NN par approche
    sys.stderr.write("Computing top-1 nearest neighbors...\n")
    bundle_nn = [top1_nearest(e, bundle_embs, i) for i, e in enumerate(bundle_embs)]
    baseline_nn = [top1_nearest(e, baseline_embs, i) for i, e in enumerate(baseline_embs)]

    # Recall global
    recall_at_1 = compute_recall_at_1(bundle_nn, baseline_nn)

    # Recall par langue
    recall_per_lang: dict[str, float] = {}
    for lang in sorted({e.language for e in entries}):
        idxs = [i for i, e in enumerate(entries) if e.language == lang]
        if not idxs:
            continue
        b_nn = [bundle_nn[i] for i in idxs]
        p_nn = [baseline_nn[i] for i in idxs]
        recall_per_lang[lang] = compute_recall_at_1(b_nn, p_nn)

    # Parity ratio
    parity = compute_parity_ratio(recall_per_lang)

    # Cosines distribution (= sanity-check qualité encode)
    try:
        import numpy as np  # type: ignore
        cosines = [float(np.dot(b, p)) for b, p in zip(bundle_embs, baseline_embs)]
    except ImportError:
        cosines = [sum(a * b for a, b in zip(bu, ba)) for bu, ba in zip(bundle_embs, baseline_embs)]
    cos_dist = compute_cosine_distribution(cosines)

    # SLO compliance synthétique
    slo_compliance = {
        "SLO-BUNDLE-RECALL": {
            "measured_ratio": recall_at_1,
            "threshold_ratio": 0.92,
            "passed": recall_at_1 >= 0.92,
        },
        "SLO-BUNDLE-LINGUAL": {
            "measured_ratio": parity,
            "threshold_ratio": 1.05,
            "passed": parity <= 1.05,
        },
    }

    report = RecallReport(
        bundle_id=manifest.get("bundle_id", "unknown"),
        version=manifest.get("version", "unknown"),
        manifest_sha256=manifest.get("manifest_sha256", ""),
        eval_set=str(eval_set_path),
        n_phrases=len(entries),
        recall_at_1=recall_at_1,
        recall_at_1_per_lang=recall_per_lang,
        parity_ratio=parity,
        cosine_mean=cos_dist["mean"],
        cosine_p5=cos_dist["p5"],
        cosine_p50=cos_dist["p50"],
        cosine_p99=cos_dist["p99"],
        slo_compliance=slo_compliance,
    )

    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(report.__dict__, indent=2))
    sys.stderr.write(f"✓ Report written: {output_path}\n")
    sys.stderr.write(f"  recall@1 = {recall_at_1:.4f}\n")
    sys.stderr.write(f"  parity   = {parity:.4f}\n")

    return report


# ──────────────────────────────────────────────────────────────────
# CLI
# ──────────────────────────────────────────────────────────────────


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, required=True, help="Bundle dir path")
    parser.add_argument("--eval-set", type=Path, required=True, help="Eval JSONL path")
    parser.add_argument("--output", type=Path, required=True, help="Output JSON path")
    parser.add_argument(
        "--langs",
        nargs="+",
        default=None,
        help="Filter languages (e.g. --langs fr en). Default = all.",
    )
    parser.add_argument(
        "--baseline-model-id",
        type=str,
        default=None,
        help="HF model ID for baseline. Default = manifest.model_origin.repo",
    )
    parser.add_argument(
        "--skip-baseline",
        action="store_true",
        help="Skip baseline encoding (= sanity-check only ; recall = 1.0 trivial)",
    )
    args = parser.parse_args(argv)

    try:
        run_bench(
            bundle_dir=args.bundle,
            eval_set_path=args.eval_set,
            output_path=args.output,
            langs=args.langs,
            baseline_model_id=args.baseline_model_id,
            skip_baseline=args.skip_baseline,
        )
    except FileNotFoundError as e:
        sys.stderr.write(f"FATAL: {e}\n")
        return 2
    except ImportError as e:
        sys.stderr.write(f"FATAL: {e}\n")
        return 3
    return 0


if __name__ == "__main__":
    sys.exit(main())
