# SPDX-License-Identifier: UNLICENSED
"""Sprint **Φ-C D26 (Bundle embedder réel)** — pin tests pure-function
des benches recall + latency.

Cf. `docs/EFE_BUNDLE_EMBEDDER_REAL_ARCHITECTURE.md` § 7.

## Mission

Ces tests pin les **fonctions pures** des deux benches (= sans
ONNX runtime + sans HF model + sans GPU). Les benches end-to-end
sont exécutés par CI (workflow `bundle-release.yml`) avec
hardware réel.

Ce fichier contient :
- recall : compute_recall_at_1, compute_parity_ratio,
  filter_by_languages, compute_cosine_distribution
- latency : compute_quantiles, evaluate_slo_compliance,
  generate_random_token_ids
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest


# Ajoute le module bench au path
ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts" / "bench"))

import bundle_recall_bench as recall   # noqa: E402
import bundle_latency_bench as latency # noqa: E402


# ──────────────────────────────────────────────────────────────────
# Recall bench
# ──────────────────────────────────────────────────────────────────


def test_recall_at_1_perfect_match() -> None:
    bundle = [0, 1, 2, 3, 4]
    baseline = [0, 1, 2, 3, 4]
    assert recall.compute_recall_at_1(bundle, baseline) == 1.0


def test_recall_at_1_zero_match() -> None:
    bundle = [0, 1, 2, 3, 4]
    baseline = [5, 6, 7, 8, 9]
    assert recall.compute_recall_at_1(bundle, baseline) == 0.0


def test_recall_at_1_partial_match() -> None:
    bundle = [0, 1, 2, 3, 4]
    baseline = [0, 1, 99, 99, 4]
    assert recall.compute_recall_at_1(bundle, baseline) == 0.6     # 3/5


def test_recall_at_1_empty_returns_zero() -> None:
    assert recall.compute_recall_at_1([], []) == 0.0


def test_recall_at_1_length_mismatch_raises() -> None:
    with pytest.raises(ValueError, match="length mismatch"):
        recall.compute_recall_at_1([1, 2], [1])


def test_parity_ratio_perfect_parity() -> None:
    r = {"fr": 0.92, "en": 0.92}
    assert recall.compute_parity_ratio(r) == 1.0


def test_parity_ratio_below_threshold() -> None:
    r = {"fr": 0.91, "en": 0.93}
    ratio = recall.compute_parity_ratio(r)
    assert 1.02 < ratio < 1.03


def test_parity_ratio_above_threshold() -> None:
    r = {"fr": 0.70, "en": 0.92}
    ratio = recall.compute_parity_ratio(r)
    assert ratio > 1.05


def test_parity_ratio_zero_division() -> None:
    r = {"fr": 0.0, "en": 0.92}
    assert recall.compute_parity_ratio(r) == float("inf")


def test_parity_ratio_empty_returns_one() -> None:
    assert recall.compute_parity_ratio({}) == 1.0


def test_filter_by_languages_keeps_all_when_none() -> None:
    entries = [
        recall.EvalEntry("hi", "en", 1),
        recall.EvalEntry("bonjour", "fr", 2),
    ]
    assert len(recall.filter_by_languages(entries, None)) == 2


def test_filter_by_languages_filters_correctly() -> None:
    entries = [
        recall.EvalEntry("hi", "en", 1),
        recall.EvalEntry("bonjour", "fr", 2),
        recall.EvalEntry("hola", "es", 3),
    ]
    filtered = recall.filter_by_languages(entries, ["fr"])
    assert len(filtered) == 1
    assert filtered[0].language == "fr"


def test_cosine_distribution_pure_python_fallback() -> None:
    # Force le path sans numpy en patching
    dist = recall.compute_cosine_distribution([0.5, 0.6, 0.7, 0.8, 0.9, 0.95, 0.99])
    assert 0.5 <= dist["mean"] <= 1.0
    assert 0.5 <= dist["p5"] <= 1.0
    assert 0.5 <= dist["p99"] <= 1.0


def test_cosine_distribution_empty_returns_zeros() -> None:
    dist = recall.compute_cosine_distribution([])
    assert dist == {"mean": 0.0, "p5": 0.0, "p50": 0.0, "p99": 0.0}


def test_load_eval_set_parses_jsonl(tmp_path: Path) -> None:
    p = tmp_path / "eval.jsonl"
    p.write_text(
        '{"text": "hello", "language": "en", "canonical_id": 1}\n'
        '{"text": "bonjour", "language": "fr", "canonical_id": 2}\n'
        '\n'                                            # ligne vide → skip
        '{"text": "hola", "language": "es", "canonical_id": 3}\n'
    )
    entries = recall.load_eval_set(p)
    assert len(entries) == 3
    assert entries[0].text == "hello"
    assert entries[2].language == "es"


# ──────────────────────────────────────────────────────────────────
# Latency bench
# ──────────────────────────────────────────────────────────────────


def test_compute_quantiles_basic() -> None:
    # Durations en secondes
    durations = [0.001 * i for i in range(1, 101)]    # 1 ms → 100 ms
    q = latency.compute_quantiles(durations)
    # p50 ≈ 50 ms, p99 ≈ 99 ms
    assert 45.0 <= q["p50_ms"] <= 55.0, q
    assert 95.0 <= q["p99_ms"] <= 101.0, q


def test_compute_quantiles_empty_returns_zeros() -> None:
    q = latency.compute_quantiles([])
    assert q == {"p50_ms": 0.0, "p95_ms": 0.0, "p99_ms": 0.0, "p999_ms": 0.0}


def test_compute_quantiles_single_value() -> None:
    q = latency.compute_quantiles([0.018])    # 18 ms
    assert q["p50_ms"] == pytest.approx(18.0, rel=0.05)


def test_evaluate_slo_compliance_gpu_pass() -> None:
    # GPU p99=18 ms pour seq=128 → SLO PASS (cible 26 ms)
    p99 = {16: 8.0, 64: 13.0, 128: 18.0, 256: 28.0, 512: 58.0}
    c = latency.evaluate_slo_compliance(p99, "cuda")
    assert c["SLO-BUNDLE-INFERENCE-GPU"]["passed"] is True
    assert c["SLO-BUNDLE-INFERENCE-GPU"]["measured_ns"] == 18_000_000
    assert c["SLO-BUNDLE-INFERENCE-CPU"]["passed"] is None
    assert "skipped" in c["SLO-BUNDLE-INFERENCE-CPU"]


def test_evaluate_slo_compliance_gpu_fail() -> None:
    # GPU p99=35 ms pour seq=128 → SLO FAIL (cible 26 ms)
    p99 = {128: 35.0}
    c = latency.evaluate_slo_compliance(p99, "cuda")
    assert c["SLO-BUNDLE-INFERENCE-GPU"]["passed"] is False


def test_evaluate_slo_compliance_cpu_pass() -> None:
    # CPU p99=150 ms pour seq=128 → SLO PASS (cible 180 ms)
    p99 = {128: 150.0}
    c = latency.evaluate_slo_compliance(p99, "cpu")
    assert c["SLO-BUNDLE-INFERENCE-CPU"]["passed"] is True
    assert c["SLO-BUNDLE-INFERENCE-CPU"]["measured_ns"] == 150_000_000


def test_evaluate_slo_compliance_unknown_provider() -> None:
    with pytest.raises(ValueError, match="Unknown provider"):
        latency.evaluate_slo_compliance({128: 18.0}, "tpu")


def test_generate_random_token_ids_deterministic() -> None:
    # Même seed = même séquence
    a = latency.generate_random_token_ids(seq_len=128, seed=42)
    b = latency.generate_random_token_ids(seq_len=128, seed=42)
    assert a == b


def test_generate_random_token_ids_different_seeds() -> None:
    a = latency.generate_random_token_ids(seq_len=128, seed=42)
    b = latency.generate_random_token_ids(seq_len=128, seed=43)
    assert a != b


def test_generate_random_token_ids_seq_len_respected() -> None:
    for sl in [16, 64, 128, 256, 512]:
        ids = latency.generate_random_token_ids(seq_len=sl, seed=0)
        assert len(ids) == sl


def test_generate_random_token_ids_vocab_bounded() -> None:
    ids = latency.generate_random_token_ids(seq_len=1000, vocab_size=100, seed=0)
    assert all(0 <= i < 100 for i in ids)


def test_canonical_seq_lens_match_doc() -> None:
    # Pin contractuel : les 5 seq_lens canoniques alignés avec doc § 7.2.
    assert latency.CANONICAL_SEQ_LENS == [16, 64, 128, 256, 512]


def test_slo_thresholds_match_baseline_yaml() -> None:
    # Pin contractuel : les seuils Python = ceux du YAML baseline.
    # GPU = 26 ms = 26_000_000 ns ; CPU = 180 ms = 180_000_000 ns.
    assert latency.SLO_THRESHOLDS_NS["SLO-BUNDLE-INFERENCE-GPU"] == 26_000_000
    assert latency.SLO_THRESHOLDS_NS["SLO-BUNDLE-INFERENCE-CPU"] == 180_000_000
