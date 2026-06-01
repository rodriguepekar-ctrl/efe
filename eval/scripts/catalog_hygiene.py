"""Sprint D8 — Catalog hygiene scoring tool.

Diagnostic that runs **without RPC** (offline, deterministic, < 30 s
on 100 cases) on a `(catalog.yaml, golden.jsonl)` pair to detect
common pathologies BEFORE deployment :

    1. Class balance     — n canonical sentences per class
    2. Inter-class collision — pairwise cosine of class centroids
    3. Within-class consistency — variance of intra-class anchor cosines
    4. Golden coverage   — class distribution in golden
    5. Hardest cases     — golden cases where raw cosine confuses

Each check produces a status (`OK` / `WARN` / `ERROR`) and an
explanation. The script aggregates them into a 0-10 hygiene score
and emits a Markdown report.

Usage :
    python eval/scripts/catalog_hygiene.py \
        --catalog catalogs/support_triage.yaml \
        --golden eval/golden/support_triage.jsonl \
        --output target/hygiene_support_triage.md \
        [--strict]   # exit 1 on any WARN/ERROR

The `--strict` flag is for CI gating ; without it the report is
informative.
"""
from __future__ import annotations

import argparse
import json
import statistics
import sys
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

import numpy as np

_THIS = Path(__file__).resolve()
_ROOT = _THIS.parents[2]
_CLIENT = _ROOT / "clients" / "python"
if (_CLIENT / "efe_tool").is_dir():
    sys.path.insert(0, str(_CLIENT))

from efe_tool import AnchorCatalog, LocalSTEmbedder  # noqa: E402


# ---------------------------------------------------------------------------
# Thresholds (calibrated on the 3 D7 catalogs ; documented in §5.5)
# ---------------------------------------------------------------------------

#: Max ratio of (max class size) / (min class size) before WARN.
CLASS_BALANCE_RATIO_WARN = 3.0
#: Pairwise inter-class cosine above which classes risk collision (WARN).
COLLISION_COS_WARN = 0.70
#: Pairwise inter-class cosine above which collision is severe (ERROR).
COLLISION_COS_ERROR = 0.85
#: Max within-class variance of anchor cosines before WARN (anchor incoherent).
WITHIN_CLASS_VAR_WARN = 0.30
#: Min support per class in golden before WARN (statistical significance).
GOLDEN_MIN_SUPPORT_WARN = 5
#: Top-K hardest cases to surface (where raw cosine misclassifies vs expected).
HARDEST_CASES_K = 10


@dataclass
class CheckResult:
    name: str
    status: str  # "OK" | "WARN" | "ERROR"
    detail: str
    score_contribution: float  # 0.0 to 1.0
    extra: dict[str, Any] = field(default_factory=dict)


def _cosine(a: np.ndarray, b: np.ndarray) -> float:
    na = np.linalg.norm(a)
    nb = np.linalg.norm(b)
    if na <= 0 or nb <= 0 or not (np.isfinite(na) and np.isfinite(nb)):
        return 0.0
    return float(np.dot(a, b) / (na * nb))


def _class_centroid(catalog: AnchorCatalog, class_name: str) -> np.ndarray:
    """Mean of the canonical-sentence embeddings for a class.

    Falls back to the catalog's pre-compiled `anchors[i]` row when
    canonical sentences are not retained at the row level — which is
    the case for production catalogs after compile.
    """
    # In the compiled MVP catalog, `catalog.anchors` is shape (N_classes, D)
    # already aggregated per class. We just look up the row index.
    for i, c in enumerate(catalog.classes):
        if c.name == class_name:
            return catalog.anchors[i]
    raise ValueError(f"unknown class : {class_name}")


# ---------------------------------------------------------------------------
# Individual checks
# ---------------------------------------------------------------------------


def check_class_balance(catalog: AnchorCatalog) -> CheckResult:
    counts = {c.name: len(c.canonical_sentences) for c in catalog.classes}
    if not counts:
        return CheckResult("Class balance", "ERROR",
                           "No classes in catalog.", 0.0, {"counts": {}})
    cmax = max(counts.values())
    cmin = max(1, min(counts.values()))
    ratio = cmax / cmin
    if ratio <= 1.5:
        status, score = "OK", 1.0
        detail = f"All classes balanced (max/min ratio = {ratio:.1f})."
    elif ratio <= CLASS_BALANCE_RATIO_WARN:
        status, score = "OK", 0.85
        detail = f"Classes mildly imbalanced (max/min = {ratio:.1f})."
    else:
        status, score = "WARN", 0.5
        detail = (f"Strong imbalance : max/min = {ratio:.1f} > "
                  f"{CLASS_BALANCE_RATIO_WARN}. Consider adding "
                  f"canonical sentences to under-represented classes.")
    return CheckResult(
        "Class balance", status, detail, score,
        {"counts": counts, "ratio": ratio}
    )


def check_inter_class_collision(catalog: AnchorCatalog) -> CheckResult:
    n = len(catalog.classes)
    if n < 2:
        return CheckResult("Inter-class collision", "ERROR",
                           "Need ≥ 2 classes.", 0.0)
    pairs: list[tuple[str, str, float]] = []
    for i in range(n):
        for j in range(i + 1, n):
            a = catalog.anchors[i]
            b = catalog.anchors[j]
            c = _cosine(a, b)
            pairs.append((catalog.classes[i].name, catalog.classes[j].name, c))
    pairs.sort(key=lambda x: -x[2])
    worst = pairs[0]
    if worst[2] >= COLLISION_COS_ERROR:
        status, score = "ERROR", 0.2
        detail = (f"Severe collision : `{worst[0]}` × `{worst[1]}` cos = "
                  f"{worst[2]:.3f} ≥ {COLLISION_COS_ERROR}. These classes "
                  f"will be inherently confused.")
    elif worst[2] >= COLLISION_COS_WARN:
        status, score = "WARN", 0.6
        detail = (f"High collision risk : `{worst[0]}` × `{worst[1]}` cos = "
                  f"{worst[2]:.3f} ≥ {COLLISION_COS_WARN}. Re-curate the "
                  f"canonical sentences for one of these classes.")
    else:
        status, score = "OK", 1.0
        detail = f"All inter-class cosines < {COLLISION_COS_WARN} (max = {worst[2]:.3f})."
    return CheckResult("Inter-class collision", status, detail, score,
                       {"pairs": [{"a": a, "b": b, "cos": c} for a, b, c in pairs]})


def check_within_class_consistency(
    catalog: AnchorCatalog, embedder: LocalSTEmbedder
) -> CheckResult:
    """Compute variance of cosine(canonical_i, class_centroid) per class.

    A high variance means the canonical sentences are not coherent —
    the class is internally fuzzy. This is independent of the catalog's
    compilation step.
    """
    if not catalog.classes:
        return CheckResult("Within-class consistency", "ERROR",
                           "No classes.", 0.0)
    per_class_var: dict[str, float] = {}
    worst_class: str = ""
    worst_var: float = 0.0
    for cls in catalog.classes:
        if len(cls.canonical_sentences) < 2:
            per_class_var[cls.name] = 0.0
            continue
        sentence_embs = np.array([
            embedder.encode(s) for s in cls.canonical_sentences
        ])
        centroid = sentence_embs.mean(axis=0)
        cosines = [_cosine(e, centroid) for e in sentence_embs]
        var = statistics.variance(cosines) if len(cosines) >= 2 else 0.0
        per_class_var[cls.name] = round(var, 6)
        if var > worst_var:
            worst_var = var
            worst_class = cls.name
    if worst_var > WITHIN_CLASS_VAR_WARN:
        status, score = "WARN", 0.6
        detail = (f"Class `{worst_class}` has high intra-class variance "
                  f"({worst_var:.4f} > {WITHIN_CLASS_VAR_WARN}) — the "
                  f"canonical sentences may be incoherent.")
    else:
        status, score = "OK", 1.0
        detail = (f"All classes coherent (max variance = "
                  f"{worst_var:.4f} ≤ {WITHIN_CLASS_VAR_WARN}).")
    return CheckResult("Within-class consistency", status, detail, score,
                       {"per_class_variance": per_class_var})


def check_golden_coverage(
    catalog: AnchorCatalog, golden_items: list[dict[str, Any]]
) -> CheckResult:
    if not golden_items:
        return CheckResult("Golden coverage", "ERROR",
                           "Empty golden.", 0.0)
    counts = {c.name: 0 for c in catalog.classes}
    unknown = 0
    for it in golden_items:
        exp = it.get("expected", "")
        if exp in counts:
            counts[exp] += 1
        else:
            unknown += 1
    min_support = min(counts.values())
    weakest_classes = [c for c, n in counts.items() if n < GOLDEN_MIN_SUPPORT_WARN]
    if min_support < GOLDEN_MIN_SUPPORT_WARN or unknown > 0:
        status, score = "WARN", 0.5
        detail = (f"Min class support = {min_support} (< "
                  f"{GOLDEN_MIN_SUPPORT_WARN}). Weak classes : "
                  f"{', '.join(weakest_classes)}. Unknown labels = "
                  f"{unknown}.")
    else:
        status, score = "OK", 1.0
        detail = f"All classes ≥ {GOLDEN_MIN_SUPPORT_WARN} support."
    return CheckResult("Golden coverage", status, detail, score,
                       {"counts": counts, "unknown": unknown,
                        "min_support": min_support})


def check_hardest_cases(
    catalog: AnchorCatalog,
    golden_items: list[dict[str, Any]],
    embedder: LocalSTEmbedder,
) -> CheckResult:
    """Find golden cases where the raw-cosine argmax disagrees with
    the expected label, and rank them by margin (smaller margin = worse).
    """
    cases: list[dict[str, Any]] = []
    for it in golden_items:
        vec = embedder.encode(it["text"])
        # Cosine to each class anchor.
        sims = catalog.anchors @ (vec / (np.linalg.norm(vec) + 1e-9))
        order = np.argsort(sims)[::-1]
        argmax_idx = int(order[0])
        runner_idx = int(order[1]) if order.size > 1 else argmax_idx
        argmax_label = catalog.classes[argmax_idx].name
        runner_label = catalog.classes[runner_idx].name
        expected = it["expected"]
        if argmax_label != expected:
            margin = float(sims[argmax_idx]) - float(sims[runner_idx])
            cases.append({
                "id": it.get("id", "?"),
                "text": it["text"][:80],
                "expected": expected,
                "argmax": argmax_label,
                "runner": runner_label,
                "argmax_score": float(sims[argmax_idx]),
                "runner_score": float(sims[runner_idx]),
                "margin": margin,
            })
    # Sort by smaller margin (closer call = harder)
    cases.sort(key=lambda c: c["margin"])
    hardest = cases[:HARDEST_CASES_K]
    if not cases:
        return CheckResult("Hardest cases", "OK",
                           "All golden cases pass raw-cosine argmax.",
                           1.0, {"hardest": []})
    err_rate = len(cases) / max(1, len(golden_items))
    if err_rate > 0.30:
        status, score = "WARN", 0.5
    elif err_rate > 0.10:
        status, score = "WARN", 0.7
    else:
        status, score = "OK", 0.9
    detail = (f"{len(cases)} / {len(golden_items)} golden cases "
              f"({err_rate:.1%}) misclassified by raw-cosine. "
              f"Top {min(HARDEST_CASES_K, len(cases))} hardest listed below.")
    return CheckResult("Hardest cases", status, detail, score,
                       {"hardest": hardest, "n_misclassified": len(cases),
                        "error_rate": round(err_rate, 4)})


# ---------------------------------------------------------------------------
# Aggregate score + Markdown report
# ---------------------------------------------------------------------------


def compute_score(checks: list[CheckResult]) -> float:
    """Mean of per-check score_contribution × 10. Maps to 0..10."""
    if not checks:
        return 0.0
    return round(10.0 * sum(c.score_contribution for c in checks) / len(checks), 1)


def grade(score: float) -> str:
    if score >= 9.0:
        return "Excellent"
    if score >= 7.5:
        return "Good"
    if score >= 6.0:
        return "Fair"
    if score >= 4.0:
        return "Poor"
    return "Critical"


def render_markdown(
    catalog: AnchorCatalog,
    golden_path: str,
    n_golden: int,
    checks: list[CheckResult],
    score: float,
) -> str:
    out: list[str] = []
    out.append(f"# Catalog hygiene report — `{catalog.name}@{catalog.version}`\n\n")
    out.append(f"**Overall score : {score:.1f} / 10.0** (grade : **{grade(score)}**)\n\n")
    out.append(f"- Catalog file : `{getattr(catalog, 'source_path', 'unknown')}`\n")
    out.append(f"- Golden file  : `{golden_path}` ({n_golden} cases)\n")
    out.append(f"- Embedder     : `{catalog.embedder_fingerprint}`\n")
    out.append(f"- Classes      : {', '.join(c.name for c in catalog.classes)}\n\n")

    for c in checks:
        emoji = {"OK": "✅", "WARN": "⚠️", "ERROR": "❌"}.get(c.status, "❓")
        out.append(f"## {emoji} {c.name} — {c.status}\n\n")
        out.append(f"{c.detail}\n\n")
        # Render extras as needed.
        if c.name == "Class balance" and "counts" in c.extra:
            out.append("| Class | n_canonicals |\n|-------|------:|\n")
            for cls, n in c.extra["counts"].items():
                out.append(f"| `{cls}` | {n} |\n")
            out.append("\n")
        if c.name == "Inter-class collision" and "pairs" in c.extra:
            out.append("Top 5 inter-class cosines :\n\n")
            out.append("| Class A | Class B | Cosine |\n|---------|---------|-------:|\n")
            for p in c.extra["pairs"][:5]:
                out.append(f"| `{p['a']}` | `{p['b']}` | {p['cos']:.4f} |\n")
            out.append("\n")
        if c.name == "Golden coverage" and "counts" in c.extra:
            out.append("| Class | Support |\n|-------|------:|\n")
            for cls, n in c.extra["counts"].items():
                out.append(f"| `{cls}` | {n} |\n")
            if c.extra.get("unknown", 0):
                out.append(f"| _unknown labels_ | {c.extra['unknown']} |\n")
            out.append("\n")
        if c.name == "Within-class consistency":
            out.append("| Class | Intra-class variance |\n|-------|------:|\n")
            for cls, v in c.extra["per_class_variance"].items():
                out.append(f"| `{cls}` | {v:.4f} |\n")
            out.append("\n")
        if c.name == "Hardest cases" and c.extra.get("hardest"):
            out.append("| ID | Text (truncated) | Expected | Argmax | Margin |\n")
            out.append("|----|------|----------|--------|------:|\n")
            for h in c.extra["hardest"]:
                txt = h["text"].replace("|", "\\|")
                out.append(f"| {h['id']} | {txt} | `{h['expected']}` | "
                           f"`{h['argmax']}` | {h['margin']:+.3f} |\n")
            out.append("\n")

    out.append("## Summary of mitigations suggested\n\n")
    mitigations: list[str] = []
    for c in checks:
        if c.status == "WARN" or c.status == "ERROR":
            if c.name == "Class balance":
                mitigations.append("Add canonical sentences to under-represented classes.")
            elif c.name == "Inter-class collision":
                mitigations.append("Re-curate canonical sentences of colliding classes ; "
                                  "or split the dominant class.")
            elif c.name == "Within-class consistency":
                mitigations.append("Review intra-class canonical sentences for coherence ; "
                                  "drop outliers.")
            elif c.name == "Golden coverage":
                mitigations.append("Add ≥ 5 golden examples per under-represented class.")
            elif c.name == "Hardest cases":
                mitigations.append("Review hardest cases : either re-label, or add "
                                  "canonical sentences capturing the missing aspect.")
    if not mitigations:
        out.append("_No mitigations needed — catalog hygiene is excellent._\n")
    else:
        for m in mitigations:
            out.append(f"- {m}\n")
    out.append("\n## Notes\n\n")
    out.append("- Re-run `python eval/scripts/catalog_hygiene.py --catalog <path> "
               "--golden <path> --output <path>` after changes.\n")
    out.append("- Use `--strict` flag for CI gating (exits 1 on any WARN/ERROR).\n")
    out.append("- See `docs/bench/d8_…architecture.md` § 8 for the threshold rationale.\n")
    return "".join(out)


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


def _main() -> int:
    p = argparse.ArgumentParser(
        description="Sprint D8 — Catalog hygiene scoring.")
    p.add_argument("--catalog", required=True)
    p.add_argument("--golden", required=True)
    p.add_argument("--output", required=True, help="Markdown output path.")
    p.add_argument("--json-output", default=None,
                   help="Optional JSON output (for CI consumption).")
    p.add_argument("--strict", action="store_true",
                   help="Exit code 1 on any WARN or ERROR.")
    args = p.parse_args()

    embedder = LocalSTEmbedder()
    catalog = AnchorCatalog.load(args.catalog, embedder=embedder)
    golden_items: list[dict[str, Any]] = []
    with Path(args.golden).open("r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line:
                golden_items.append(json.loads(line))

    checks = [
        check_class_balance(catalog),
        check_inter_class_collision(catalog),
        check_within_class_consistency(catalog, embedder),
        check_golden_coverage(catalog, golden_items),
        check_hardest_cases(catalog, golden_items, embedder),
    ]
    score = compute_score(checks)

    md = render_markdown(catalog, args.golden, len(golden_items), checks, score)
    Path(args.output).write_text(md)
    print(f"wrote {args.output}")
    print(f"\nScore : {score:.1f} / 10  ({grade(score)})")
    for c in checks:
        emoji = {"OK": "✅", "WARN": "⚠️", "ERROR": "❌"}.get(c.status, "❓")
        print(f"  {emoji}  {c.name:<28} {c.status:<6} {c.detail[:90]}")

    if args.json_output:
        Path(args.json_output).write_text(json.dumps({
            "catalog": catalog.name,
            "catalog_version": catalog.version,
            "golden": str(args.golden),
            "n_golden": len(golden_items),
            "score": score,
            "grade": grade(score),
            "checks": [{
                "name": c.name,
                "status": c.status,
                "detail": c.detail,
                "score_contribution": c.score_contribution,
                "extra": c.extra,
            } for c in checks],
        }, indent=2, ensure_ascii=False))
        print(f"wrote {args.json_output}")

    if args.strict:
        for c in checks:
            if c.status in ("WARN", "ERROR"):
                print(f"\nFAIL (--strict) : check `{c.name}` is {c.status}.")
                return 1
    return 0


if __name__ == "__main__":
    sys.exit(_main())
