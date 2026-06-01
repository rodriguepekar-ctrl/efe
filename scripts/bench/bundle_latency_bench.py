#!/usr/bin/env python3
# SPDX-License-Identifier: UNLICENSED
"""Sprint **Φ-C D26 (Bundle embedder réel)** — latence benchmark.

Cf. `docs/EFE_BUNDLE_EMBEDDER_REAL_ARCHITECTURE.md` § 7.2 + § 6.4.

## Mission

Mesurer la latence d'inférence ONNX du bundle, sur 2 providers
(CPU + CUDA) et 5 longueurs de séquence canoniques (16, 64, 128,
256, 512). Source des SLO :

- **SLO-BUNDLE-INFERENCE-GPU** : p99 ≤ 26 ms (T4 fp16 seq=128 batch=1).
- **SLO-BUNDLE-INFERENCE-CPU** : p99 ≤ 180 ms (CPU fp32 seq=128 batch=1).
- **SLO-P1-BUDGET-5** : hérite directement de l'inférence GPU.

## Protocole

1. Charge le bundle (ONNX session + tokenizer).
2. Warmup : 100 inférences dummy de seq=128 pour amortir cold-start.
3. Pour chaque seq_len ∈ {16, 64, 128, 256, 512} :
   - Génère 1 000 inputs random (= tokens variés).
   - Exécute 1 000 inférences sequentielles.
   - Mesure wall-clock par inférence (= `time.perf_counter()`).
   - Calcule p50 / p95 / p99 / p99.9.
4. Output JSON détaillé avec compliance par SLO.

## Output

```json
{
  "bundle_id": "paraphrase-multilingual-MiniLM-L12-v2",
  "version": "1.0.0",
  "provider": "cuda",
  "warmup_iterations": 100,
  "iterations_per_seq": 1000,
  "results_by_seq_len": {
    "16":  {"p50_ms": 4.1,  "p95_ms": 6.8,  "p99_ms": 8.2},
    "64":  {"p50_ms": 8.5,  "p95_ms": 11.4, "p99_ms": 13.0},
    "128": {"p50_ms": 11.2, "p95_ms": 15.9, "p99_ms": 18.4},
    "256": {"p50_ms": 19.0, "p95_ms": 25.3, "p99_ms": 28.7},
    "512": {"p50_ms": 38.4, "p95_ms": 51.2, "p99_ms": 58.2}
  },
  "slo_compliance": {
    "SLO-BUNDLE-INFERENCE-GPU": {"measured_ns": 18400000, "threshold_ns": 26000000, "passed": true},
    "SLO-BUNDLE-INFERENCE-CPU": {"measured_ns": null, "threshold_ns": 180000000, "passed": null, "skipped": "provider=cuda not cpu"}
  }
}
```

## Usage

```bash
# Bench GPU (= self-hosted runner avec T4)
$ python scripts/bench/bundle_latency_bench.py \
    --bundle ./bundles/paraphrase-multilingual-MiniLM-L12-v2-1.0.0/ \
    --provider cuda \
    --iterations 1000 \
    --output target/bundle_latency_gpu_report.json

# Bench CPU (= compat tout host)
$ python scripts/bench/bundle_latency_bench.py \
    --bundle ./bundles/paraphrase-multilingual-MiniLM-L12-v2-1.0.0/ \
    --provider cpu \
    --output target/bundle_latency_cpu_report.json
```

## Pin tests

- `test_quantile_calculation_pure_function`.
- `test_seq_len_generation_correct`.
- `test_slo_compliance_logic`.
"""

from __future__ import annotations

import argparse
import json
import random
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any


# ──────────────────────────────────────────────────────────────────
# Buckets canoniques alignés avec p1_instruments.rs + bundle_slos.yaml
# ──────────────────────────────────────────────────────────────────

CANONICAL_SEQ_LENS = [16, 64, 128, 256, 512]
DEFAULT_WARMUP = 100
DEFAULT_ITERATIONS = 1000

# Seuils SLO du baseline v1.0.0 (= source de vérité)
SLO_THRESHOLDS_NS = {
    "SLO-BUNDLE-INFERENCE-GPU": 26_000_000,   # 26 ms
    "SLO-BUNDLE-INFERENCE-CPU": 180_000_000,  # 180 ms
}


# ──────────────────────────────────────────────────────────────────
# Pure-function helpers (testable sans ONNX)
# ──────────────────────────────────────────────────────────────────


def compute_quantiles(durations_seconds: list[float]) -> dict[str, float]:
    """Calcule p50 / p95 / p99 / p99.9 (en millisecondes).

    Pure function. Si numpy dispo l'utilise pour la précision sur
    > 1 000 samples ; sinon fallback Python pur."""
    if not durations_seconds:
        return {"p50_ms": 0.0, "p95_ms": 0.0, "p99_ms": 0.0, "p999_ms": 0.0}
    try:
        import numpy as np    # type: ignore
        arr = np.array(durations_seconds, dtype=np.float64) * 1000.0
        return {
            "p50_ms":  float(np.percentile(arr, 50)),
            "p95_ms":  float(np.percentile(arr, 95)),
            "p99_ms":  float(np.percentile(arr, 99)),
            "p999_ms": float(np.percentile(arr, 99.9)),
        }
    except ImportError:
        sorted_d = sorted(durations_seconds)
        n = len(sorted_d)
        # Linear interpolation pour les quantiles
        def pct(p: float) -> float:
            idx = max(0, min(n - 1, int(n * p / 100.0)))
            return sorted_d[idx] * 1000.0
        return {
            "p50_ms":  pct(50),
            "p95_ms":  pct(95),
            "p99_ms":  pct(99),
            "p999_ms": pct(99.9),
        }


def evaluate_slo_compliance(
    p99_ms_by_seq: dict[int, float],
    provider: str,
) -> dict[str, dict[str, Any]]:
    """Évalue les SLO bundle pour le provider donné.

    Pure function. Le seuil GPU s'applique seulement si provider=cuda ;
    le seuil CPU seulement si provider=cpu.

    **Audit fix Φ-C.A11** — si la seq=128 (= seq de référence
    canonique) n'est pas présente dans `p99_ms_by_seq`, on retourne
    `passed=None` avec `skipped="missing_seq_128_measure"` au lieu
    de `passed=False` (= ne pas fail un SLO qu'on n'a pas mesuré).
    """
    compliance: dict[str, dict[str, Any]] = {}
    # Le SLO se calibre sur seq=128 (= reference canonique)
    p99_128 = p99_ms_by_seq.get(128, None)

    gpu_threshold_ns = SLO_THRESHOLDS_NS["SLO-BUNDLE-INFERENCE-GPU"]
    cpu_threshold_ns = SLO_THRESHOLDS_NS["SLO-BUNDLE-INFERENCE-CPU"]

    if provider == "cuda":
        if p99_128 is None:
            compliance["SLO-BUNDLE-INFERENCE-GPU"] = {
                "measured_ns": None,
                "threshold_ns": gpu_threshold_ns,
                "passed": None,
                "skipped": "missing_seq_128_measure",
            }
        else:
            measured_ns = int(p99_128 * 1e6)
            compliance["SLO-BUNDLE-INFERENCE-GPU"] = {
                "measured_ns": measured_ns,
                "threshold_ns": gpu_threshold_ns,
                "passed": measured_ns <= gpu_threshold_ns,
            }
        compliance["SLO-BUNDLE-INFERENCE-CPU"] = {
            "measured_ns": None,
            "threshold_ns": cpu_threshold_ns,
            "passed": None,
            "skipped": "provider=cuda not cpu",
        }
    elif provider == "cpu":
        if p99_128 is None:
            compliance["SLO-BUNDLE-INFERENCE-CPU"] = {
                "measured_ns": None,
                "threshold_ns": cpu_threshold_ns,
                "passed": None,
                "skipped": "missing_seq_128_measure",
            }
        else:
            measured_ns = int(p99_128 * 1e6)
            compliance["SLO-BUNDLE-INFERENCE-CPU"] = {
                "measured_ns": measured_ns,
                "threshold_ns": cpu_threshold_ns,
                "passed": measured_ns <= cpu_threshold_ns,
            }
        compliance["SLO-BUNDLE-INFERENCE-GPU"] = {
            "measured_ns": None,
            "threshold_ns": gpu_threshold_ns,
            "passed": None,
            "skipped": "provider=cpu not cuda",
        }
    else:
        raise ValueError(f"Unknown provider: {provider}")

    return compliance


def generate_random_token_ids(seq_len: int, vocab_size: int = 250_002, seed: int = 42) -> list[int]:
    """Génère une séquence pseudo-random de token IDs de longueur
    `seq_len`. Vocab_size = celui de XLM-R (= 250k).

    Pure function ; deterministic via seed.
    """
    rng = random.Random(seed)
    return [rng.randint(0, vocab_size - 1) for _ in range(seq_len)]


# ──────────────────────────────────────────────────────────────────
# ONNX inference (= heavy, gated par environment)
# ──────────────────────────────────────────────────────────────────


def init_session(bundle_dir: Path, provider: str):
    """Initialise une session ONNX avec le provider demandé."""
    try:
        import onnxruntime as ort   # type: ignore
    except ImportError as e:  # pragma: no cover
        raise ImportError(
            f"onnxruntime missing: {e}. Install: pip install onnxruntime "
            f"(or onnxruntime-gpu pour CUDA)"
        )

    onnx_path = bundle_dir / "weights" / "model.onnx"
    if not onnx_path.exists():
        raise FileNotFoundError(f"Bundle ONNX missing: {onnx_path}")

    available = ort.get_available_providers()
    if provider == "cuda":
        if "CUDAExecutionProvider" not in available:
            raise RuntimeError(
                f"CUDA EP not available ; available providers: {available}"
            )
        providers = ["CUDAExecutionProvider", "CPUExecutionProvider"]
    elif provider == "cpu":
        providers = ["CPUExecutionProvider"]
    else:
        raise ValueError(f"Unknown provider: {provider}")

    sess_opts = ort.SessionOptions()
    sess_opts.intra_op_num_threads = 1
    sess_opts.inter_op_num_threads = 1
    sess_opts.graph_optimization_level = ort.GraphOptimizationLevel.ORT_ENABLE_EXTENDED

    sess = ort.InferenceSession(str(onnx_path), sess_options=sess_opts, providers=providers)
    return sess


def run_single_inference(sess, token_ids: list[int]) -> float:
    """Exécute une inférence et retourne la durée en secondes.

    `token_ids` est une liste plate ; on construit
    input_ids + attention_mask de shape (1, seq_len).
    """
    try:
        import numpy as np   # type: ignore
    except ImportError as e:  # pragma: no cover
        raise ImportError(f"numpy missing: {e}")

    input_ids = np.array([token_ids], dtype=np.int64)
    attention_mask = np.ones_like(input_ids)

    t0 = time.perf_counter()
    _ = sess.run(None, {"input_ids": input_ids, "attention_mask": attention_mask})
    return time.perf_counter() - t0


# ──────────────────────────────────────────────────────────────────
# Main bench
# ──────────────────────────────────────────────────────────────────


@dataclass
class LatencyReport:
    """Bench output."""
    bundle_id: str
    version: str
    manifest_sha256: str
    provider: str
    warmup_iterations: int
    iterations_per_seq: int
    results_by_seq_len: dict[int, dict[str, float]] = field(default_factory=dict)
    slo_compliance: dict[str, dict[str, Any]] = field(default_factory=dict)
    host_info: dict[str, str] = field(default_factory=dict)


def collect_host_info(provider: str) -> dict[str, str]:
    """Collecte les infos host pour le rapport (= traçabilité bench)."""
    import platform
    info = {
        "os": platform.system() + " " + platform.release(),
        "machine": platform.machine(),
        "python": platform.python_version(),
    }
    try:
        import onnxruntime as ort   # type: ignore
        info["onnxruntime"] = ort.__version__
    except ImportError:
        pass
    if provider == "cuda":
        try:
            import subprocess
            r = subprocess.run(
                ["nvidia-smi", "--query-gpu=name,driver_version", "--format=csv,noheader"],
                capture_output=True, text=True, timeout=5,
            )
            if r.returncode == 0:
                info["gpu"] = r.stdout.strip()
        except Exception:  # noqa: BLE001
            pass
    return info


def run_bench(
    bundle_dir: Path,
    provider: str,
    output_path: Path,
    iterations: int = DEFAULT_ITERATIONS,
    warmup: int = DEFAULT_WARMUP,
    seq_lens: list[int] | None = None,
) -> LatencyReport:
    """Pipeline complet."""
    if seq_lens is None:
        seq_lens = CANONICAL_SEQ_LENS

    # Read manifest pour le report
    manifest_path = bundle_dir / "manifest.json"
    manifest = json.loads(manifest_path.read_text()) if manifest_path.exists() else {}

    sys.stderr.write(f"Initializing ONNX session (provider={provider})...\n")
    sess = init_session(bundle_dir, provider)

    # Warmup
    sys.stderr.write(f"Warmup {warmup} inferences seq=128...\n")
    warmup_tokens = generate_random_token_ids(128, seed=0)
    for _ in range(warmup):
        run_single_inference(sess, warmup_tokens)

    # Mesures par seq_len
    results: dict[int, dict[str, float]] = {}
    p99_ms_by_seq: dict[int, float] = {}
    for seq in seq_lens:
        sys.stderr.write(f"Bench seq_len={seq} × {iterations} iterations...\n")
        durations: list[float] = []
        for i in range(iterations):
            tokens = generate_random_token_ids(seq, seed=i)
            d = run_single_inference(sess, tokens)
            durations.append(d)
        quantiles = compute_quantiles(durations)
        results[seq] = quantiles
        p99_ms_by_seq[seq] = quantiles["p99_ms"]
        sys.stderr.write(
            f"  seq={seq:>4}  p50={quantiles['p50_ms']:6.2f} ms  "
            f"p99={quantiles['p99_ms']:6.2f} ms\n"
        )

    # Compliance
    compliance = evaluate_slo_compliance(p99_ms_by_seq, provider)

    report = LatencyReport(
        bundle_id=manifest.get("bundle_id", "unknown"),
        version=manifest.get("version", "unknown"),
        manifest_sha256=manifest.get("manifest_sha256", ""),
        provider=provider,
        warmup_iterations=warmup,
        iterations_per_seq=iterations,
        results_by_seq_len={str(k): v for k, v in results.items()},
        slo_compliance=compliance,
        host_info=collect_host_info(provider),
    )

    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(json.dumps(report.__dict__, indent=2))
    sys.stderr.write(f"\n✓ Report written: {output_path}\n")

    return report


# ──────────────────────────────────────────────────────────────────
# CLI
# ──────────────────────────────────────────────────────────────────


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundle", type=Path, required=True)
    parser.add_argument("--provider", choices=["cpu", "cuda"], required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--iterations", type=int, default=DEFAULT_ITERATIONS)
    parser.add_argument("--warmup", type=int, default=DEFAULT_WARMUP)
    parser.add_argument(
        "--seq-lens",
        type=int,
        nargs="+",
        default=CANONICAL_SEQ_LENS,
        help=f"Seq lens canoniques (= {CANONICAL_SEQ_LENS}).",
    )
    args = parser.parse_args(argv)

    try:
        report = run_bench(
            bundle_dir=args.bundle,
            provider=args.provider,
            output_path=args.output,
            iterations=args.iterations,
            warmup=args.warmup,
            seq_lens=args.seq_lens,
        )
    except FileNotFoundError as e:
        sys.stderr.write(f"FATAL: {e}\n")
        return 2
    except (ImportError, RuntimeError) as e:
        sys.stderr.write(f"FATAL: {e}\n")
        return 3

    # Exit code reflète compliance
    failed = [
        sid for sid, r in report.slo_compliance.items()
        if r.get("passed") is False
    ]
    if failed:
        sys.stderr.write(f"❌ FAILED SLOs: {failed}\n")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
