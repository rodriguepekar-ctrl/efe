"""Sprint D8 anti-regression tests for `eval/scripts/catalog_hygiene.py`.

The hygiene tool runs without RPC, but it requires `sentence-
transformers` (LocalSTEmbedder). For unit tests we mock the embedder
and the catalog to avoid the model load cost.

Run :
    python3 scripts/tests/test_d8_catalog_hygiene.py
"""
from __future__ import annotations

import sys
from pathlib import Path

import numpy as np

_THIS = Path(__file__).resolve()
_ROOT = _THIS.parents[2]
sys.path.insert(0, str(_ROOT / "eval" / "scripts"))

from catalog_hygiene import (  # noqa: E402
    check_class_balance,
    check_inter_class_collision,
    check_golden_coverage,
    compute_score,
    grade,
    CheckResult,
)


# ---------------------------------------------------------------------------
# Mock catalog / class definitions
# ---------------------------------------------------------------------------


class _ClassDef:
    def __init__(self, name: str, n_canonicals: int):
        self.name = name
        self.canonical_sentences = tuple(f"sent {i}" for i in range(n_canonicals))


class _Catalog:
    def __init__(self, name: str, classes_with_counts: list[tuple[str, int]],
                 anchors_matrix: np.ndarray):
        self.name = name
        self.version = "1.0.0"
        self.classes = [_ClassDef(n, c) for n, c in classes_with_counts]
        self.anchors = anchors_matrix
        self.embedder_fingerprint = "test/embedder@1"
        self.dim = int(anchors_matrix.shape[1])


def test_class_balance_ok():
    cat = _Catalog("test", [("a", 7), ("b", 7)], np.eye(2, dtype=np.float32))
    r = check_class_balance(cat)
    assert r.status == "OK"
    assert r.extra["counts"] == {"a": 7, "b": 7}
    assert r.score_contribution == 1.0
    print("OK : balanced classes → OK status, full score")


def test_class_balance_imbalance_warns():
    cat = _Catalog("test", [("a", 12), ("b", 3)], np.eye(2, dtype=np.float32))
    r = check_class_balance(cat)
    assert r.status == "WARN"
    assert r.score_contribution < 1.0
    assert r.extra["ratio"] == 4.0
    print("OK : imbalance > 3.0 → WARN")


def test_inter_class_collision_orthogonal_ok():
    # 3 mutually orthogonal anchors → max cos = 0.
    anchors = np.eye(3, dtype=np.float32)
    cat = _Catalog("test", [("a", 5), ("b", 5), ("c", 5)], anchors)
    r = check_inter_class_collision(cat)
    assert r.status == "OK"
    print("OK : orthogonal anchors → OK collision")


def test_inter_class_collision_near_duplicate_errors():
    # Two anchors very close → high collision.
    a = np.array([1.0, 0.0, 0.0], dtype=np.float32)
    b = np.array([0.999, 0.045, 0.0], dtype=np.float32)  # cos ~ 0.999
    c = np.array([0.0, 0.0, 1.0], dtype=np.float32)
    anchors = np.array([a, b, c])
    cat = _Catalog("test", [("a", 5), ("b", 5), ("c", 5)], anchors)
    r = check_inter_class_collision(cat)
    assert r.status == "ERROR"
    assert r.score_contribution < 0.5
    print("OK : near-duplicate anchors → ERROR collision")


def test_golden_coverage_under_supported_warns():
    cat = _Catalog("test", [("a", 5), ("b", 5), ("c", 5), ("d", 5)],
                   np.eye(4, dtype=np.float32))
    items = (
        [{"id": f"a{i}", "text": "x", "expected": "a"} for i in range(20)]
        + [{"id": f"b{i}", "text": "x", "expected": "b"} for i in range(20)]
        + [{"id": f"c{i}", "text": "x", "expected": "c"} for i in range(2)]   # < 5
        + [{"id": f"d{i}", "text": "x", "expected": "d"} for i in range(20)]
    )
    r = check_golden_coverage(cat, items)
    assert r.status == "WARN"
    assert "c" in r.detail
    print("OK : class with < 5 support → WARN")


def test_compute_score_averages_contributions():
    checks = [
        CheckResult("a", "OK", "", 1.0),
        CheckResult("b", "WARN", "", 0.5),
        CheckResult("c", "OK", "", 0.85),
    ]
    score = compute_score(checks)
    expected = round(10.0 * (1.0 + 0.5 + 0.85) / 3, 1)
    assert score == expected
    print(f"OK : compute_score returns {score} (expected {expected})")


def test_grade_buckets():
    assert grade(9.5) == "Excellent"
    assert grade(7.5) == "Good"
    assert grade(6.0) == "Fair"
    assert grade(4.0) == "Poor"
    assert grade(2.0) == "Critical"
    print("OK : grade() bucket boundaries hold")


def main() -> int:
    tests = [
        test_class_balance_ok,
        test_class_balance_imbalance_warns,
        test_inter_class_collision_orthogonal_ok,
        test_inter_class_collision_near_duplicate_errors,
        test_golden_coverage_under_supported_warns,
        test_compute_score_averages_contributions,
        test_grade_buckets,
    ]
    failures = 0
    for t in tests:
        try:
            t()
        except AssertionError as e:
            print(f"FAIL : {t.__name__} — {e}")
            failures += 1
        except Exception as e:
            print(f"ERROR : {t.__name__} — {type(e).__name__}: {e}")
            failures += 1
    if failures:
        print(f"\n{failures}/{len(tests)} tests failed")
        return 1
    print(f"\nAll {len(tests)} tests passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
