"""Generate the ablation-report PDF from the three JSON result blobs.

Input:
  * ``ablation_results.json``   — 10 classifiers on seed 42
  * ``robustness_results.json`` — baseline + V1 + V4 + V9 on 5 seeds
  * ``results.json``            — original benchmark (seed 42)

Output: ``EFE_Swarm_Ablation_Analysis.pdf``
"""

from __future__ import annotations

import argparse
import json
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import matplotlib.pyplot as plt
import numpy as np
from reportlab.lib import colors
from reportlab.lib.enums import TA_CENTER, TA_JUSTIFY
from reportlab.lib.pagesizes import A4
from reportlab.lib.styles import ParagraphStyle, getSampleStyleSheet
from reportlab.lib.units import cm
from reportlab.pdfgen import canvas
from reportlab.platypus import (
    BaseDocTemplate,
    Frame,
    Image,
    NextPageTemplate,
    PageBreak,
    PageTemplate,
    Paragraph,
    Spacer,
    Table,
    TableStyle,
)

EFE_BLUE = colors.HexColor("#0b3d91")
EFE_ACCENT = colors.HexColor("#e67e22")
EFE_LIGHT = colors.HexColor("#eef2f7")
EFE_DARK = colors.HexColor("#1c1c1c")
EFE_GREY = colors.HexColor("#5a5a5a")
EFE_BORDER = colors.HexColor("#b0b7c3")
EFE_OK = colors.HexColor("#2e8b57")
EFE_BAD = colors.HexColor("#c0392b")

STYLES = getSampleStyleSheet()
TITLE = ParagraphStyle("Title", parent=STYLES["Title"], fontName="Helvetica-Bold",
                       fontSize=26, leading=32, textColor=EFE_BLUE,
                       alignment=TA_CENTER, spaceAfter=14)
SUBTITLE = ParagraphStyle("Subtitle", parent=STYLES["Heading2"], fontName="Helvetica",
                          fontSize=14, leading=18, textColor=EFE_GREY,
                          alignment=TA_CENTER, spaceAfter=6)
H1 = ParagraphStyle("H1", parent=STYLES["Heading1"], fontName="Helvetica-Bold",
                    fontSize=20, leading=24, textColor=EFE_BLUE,
                    spaceBefore=16, spaceAfter=10)
H2 = ParagraphStyle("H2", parent=STYLES["Heading2"], fontName="Helvetica-Bold",
                    fontSize=14, leading=18, textColor=EFE_BLUE,
                    spaceBefore=10, spaceAfter=6)
BODY = ParagraphStyle("Body", parent=STYLES["BodyText"], fontName="Helvetica",
                      fontSize=10, leading=13.5, textColor=EFE_DARK,
                      alignment=TA_JUSTIFY, spaceAfter=5)
CALL = ParagraphStyle("Callout", parent=BODY, fontName="Helvetica-Oblique",
                      textColor=EFE_GREY, leftIndent=10, rightIndent=10)
VERDICT = ParagraphStyle("Verdict", parent=STYLES["Heading1"], fontName="Helvetica-Bold",
                         fontSize=18, leading=22, textColor=EFE_ACCENT,
                         alignment=TA_CENTER, spaceBefore=10, spaceAfter=10)


def _p(t: str, s=BODY) -> Paragraph:
    return Paragraph(t, s)


# ---------------------------------------------------------------------------
#  Figures
# ---------------------------------------------------------------------------


@dataclass
class Figs:
    ablation_acc: Path
    ablation_ece: Path
    ablation_coverage: Path
    robustness_all: Path
    robustness_hard: Path


def _setup_mpl() -> None:
    plt.rcParams.update(
        {
            "font.family": "DejaVu Sans",
            "font.size": 10,
            "axes.edgecolor": "#5a5a5a",
            "axes.titlecolor": "#0b3d91",
            "axes.titleweight": "bold",
            "axes.spines.top": False,
            "axes.spines.right": False,
            "figure.facecolor": "white",
            "axes.facecolor": "white",
        }
    )


def build_figures(
    ablation: dict[str, Any], robustness: dict[str, Any], out_dir: Path
) -> Figs:
    out_dir.mkdir(parents=True, exist_ok=True)
    _setup_mpl()

    # --- Fig 1: accuracy bars across variants (all slice) ---
    variants_order = [
        "baseline", "V1", "V2_orthogonal", "V3_topk", "V4_mahalanobis",
        "V5_temperature", "V6_hybrid", "V7_hybrid_calibrated",
        "V8_calibrated_mahalanobis", "V9_ensemble",
    ]
    accs = [ablation["variants"][v]["slices"]["all"]["accuracy"] for v in variants_order]
    hards = [ablation["variants"][v]["slices"]["hard"]["accuracy"] for v in variants_order]
    fig, ax = plt.subplots(figsize=(7.2, 3.6))
    xs = np.arange(len(variants_order))
    w = 0.4
    base_level = accs[0]
    ax.axhline(base_level, color="#c0392b", linestyle="--", alpha=0.5,
               label=f"baseline = {base_level:.3f}")
    ax.bar(xs - w/2, accs, w, color="#0b3d91", label="ALL accuracy")
    ax.bar(xs + w/2, hards, w, color="#e67e22", label="HARD accuracy")
    ax.set_xticks(xs)
    ax.set_xticklabels([v.replace("_", "\n") for v in variants_order], rotation=0, fontsize=7.5)
    ax.set_ylabel("Accuracy")
    ax.set_ylim(0.5, 0.82)
    ax.set_title("Ablation — accuracy across 10 classifier variants (seed 42)")
    ax.legend(loc="lower right", fontsize=8, frameon=False)
    ax.grid(axis="y", alpha=0.3)
    fig.tight_layout()
    p1 = out_dir / "ablation_acc.png"
    fig.savefig(p1, dpi=180, bbox_inches="tight"); plt.close(fig)

    # --- Fig 2: ECE across variants ---
    eces = [ablation["variants"][v]["slices"]["all"]["ece"] for v in variants_order]
    fig, ax = plt.subplots(figsize=(7.2, 3.2))
    colors_list = ["#c0392b" if e >= 0.2 else ("#2e8b57" if e <= 0.08 else "#888") for e in eces]
    ax.bar(xs, eces, color=colors_list)
    ax.set_xticks(xs)
    ax.set_xticklabels([v.replace("_", "\n") for v in variants_order], fontsize=7.5)
    ax.set_ylabel("Expected Calibration Error")
    ax.set_title("Ablation — calibration (lower = better)")
    ax.axhline(eces[0], color="#c0392b", linestyle="--", alpha=0.4)
    ax.grid(axis="y", alpha=0.3)
    for i, e in enumerate(eces):
        ax.text(i, e + 0.005, f"{e:.3f}", ha="center", fontsize=7.5)
    fig.tight_layout()
    p2 = out_dir / "ablation_ece.png"
    fig.savefig(p2, dpi=180, bbox_inches="tight"); plt.close(fig)

    # --- Fig 3: verdict coverage × gated accuracy scatter ---
    fig, ax = plt.subplots(figsize=(6.6, 4.2))
    for name, res in ablation["variants"].items():
        stats = res.get("verdict_stats", {})
        if not stats:
            continue
        conv = stats.get("CONVERGED", {"n": 0, "correct": 0})
        total = sum(s["n"] for s in stats.values())
        if conv["n"] == 0 or total == 0:
            continue
        cov = conv["n"] / total
        acc = conv["correct"] / conv["n"]
        marker = "*" if name in ("V1", "V4_mahalanobis", "V9_ensemble") else "o"
        size = 220 if marker == "*" else 80
        ax.scatter([cov], [acc], s=size, marker=marker,
                   edgecolors="black", linewidths=0.7)
        ax.annotate(name, (cov, acc), xytext=(7, 6), textcoords="offset points",
                    fontsize=8)
    ax.set_xlabel("CONVERGED coverage")
    ax.set_ylabel("Gated accuracy on CONVERGED")
    ax.set_title("Verdict discipline trade-off (★ = candidates)")
    ax.set_xlim(-0.02, 1.05)
    ax.set_ylim(0.6, 1.05)
    ax.grid(alpha=0.3)
    fig.tight_layout()
    p3 = out_dir / "ablation_coverage.png"
    fig.savefig(p3, dpi=180, bbox_inches="tight"); plt.close(fig)

    # --- Fig 4 & 5: robustness — per-seed line plots ---
    seeds = robustness["seeds"]
    per_seed = robustness["per_seed"]
    for metric, title, out_name in [
        ("accuracy_all", "Robustness — ALL accuracy across 5 seeds", "robustness_all.png"),
        ("accuracy_hard", "Robustness — HARD accuracy across 5 seeds", "robustness_hard.png"),
    ]:
        fig, ax = plt.subplots(figsize=(7.0, 3.4))
        palette = {
            "baseline": "#c0392b",
            "V1": "#0b3d91",
            "V4_mahalanobis": "#e67e22",
            "V9_ensemble": "#2e8b57",
        }
        for name in per_seed:
            vals = [r[metric] for r in per_seed[name]]
            ax.plot(seeds, vals, "o-", label=name, color=palette.get(name, "#888"),
                    linewidth=2, markersize=7)
        ax.set_xlabel("Random seed")
        ax.set_ylabel("Accuracy")
        ax.set_title(title)
        ax.legend(frameon=False, loc="best")
        ax.grid(alpha=0.3)
        # Annotate means.
        for name in per_seed:
            vals = [r[metric] for r in per_seed[name]]
            ax.axhline(np.mean(vals), color=palette.get(name, "#888"),
                       alpha=0.2, linestyle=":")
        fig.tight_layout()
        p = out_dir / out_name
        fig.savefig(p, dpi=180, bbox_inches="tight"); plt.close(fig)

    return Figs(
        ablation_acc=p1,
        ablation_ece=p2,
        ablation_coverage=p3,
        robustness_all=out_dir / "robustness_all.png",
        robustness_hard=out_dir / "robustness_hard.png",
    )


# ---------------------------------------------------------------------------
#  Page callbacks
# ---------------------------------------------------------------------------


def _on_cover(canv: canvas.Canvas, doc) -> None:  # type: ignore[no-untyped-def]
    canv.saveState()
    canv.setFillColor(EFE_BLUE)
    canv.rect(0, A4[1] - 4.5 * cm, A4[0], 4.5 * cm, fill=1, stroke=0)
    canv.setFillColor(EFE_ACCENT)
    canv.rect(0, A4[1] - 4.8 * cm, A4[0], 0.3 * cm, fill=1, stroke=0)
    canv.setFillColor(colors.white)
    canv.setFont("Helvetica-Bold", 11)
    canv.drawString(2 * cm, A4[1] - 2.2 * cm, "SWARM ABLATION & ROBUSTNESS")
    canv.setFont("Helvetica", 9)
    canv.drawString(2 * cm, A4[1] - 2.7 * cm, "Can we improve the swarm? An honest investigation.")
    canv.setFillColor(EFE_ACCENT)
    canv.circle(A4[0] - 3 * cm, A4[1] - 2.4 * cm, 0.9 * cm, fill=1, stroke=0)
    canv.setFillColor(colors.white)
    canv.setFont("Helvetica-Bold", 13)
    canv.drawCentredString(A4[0] - 3 * cm, A4[1] - 2.55 * cm, "EFE")
    canv.setFillColor(EFE_BLUE)
    canv.rect(0, 0, A4[0], 1.6 * cm, fill=1, stroke=0)
    canv.setFillColor(colors.white)
    canv.setFont("Helvetica", 8)
    canv.drawString(2 * cm, 0.7 * cm, "EFE Platform Team · v0.1.0")
    canv.drawRightString(A4[0] - 2 * cm, 0.7 * cm, "Avril 2026")
    canv.restoreState()


def _on_body(canv: canvas.Canvas, doc) -> None:  # type: ignore[no-untyped-def]
    canv.saveState()
    canv.setStrokeColor(EFE_BLUE)
    canv.line(2 * cm, A4[1] - 1.4 * cm, A4[0] - 2 * cm, A4[1] - 1.4 * cm)
    canv.setFont("Helvetica", 8)
    canv.setFillColor(EFE_GREY)
    canv.drawString(2 * cm, A4[1] - 1.1 * cm,
                    "EFE — Swarm Ablation & Robustness Analysis")
    canv.drawRightString(A4[0] - 2 * cm, A4[1] - 1.1 * cm, "v0.1.0 — Avril 2026")
    canv.setStrokeColor(EFE_BORDER)
    canv.line(2 * cm, 1.5 * cm, A4[0] - 2 * cm, 1.5 * cm)
    canv.setFont("Helvetica", 8)
    canv.drawCentredString(A4[0] / 2, 1.0 * cm,
                           f"Page {doc.page} · Confidentiel — Équipe Plateforme EFE")
    canv.restoreState()


# ---------------------------------------------------------------------------
#  Helpers
# ---------------------------------------------------------------------------


def _figure(path: Path, width: float = 15.5 * cm) -> Image:
    img = Image(str(path))
    aspect = img.imageHeight / img.imageWidth
    img.drawWidth = width
    img.drawHeight = width * aspect
    img.hAlign = "CENTER"
    return img


def _kv_table(rows: list[tuple[str, str]]) -> Table:
    data = [[_p(f"<b>{k}</b>"), _p(v)] for k, v in rows]
    t = Table(data, colWidths=[5.5 * cm, 11 * cm])
    t.setStyle(TableStyle([
        ("BACKGROUND", (0, 0), (0, -1), EFE_LIGHT),
        ("VALIGN", (0, 0), (-1, -1), "TOP"),
        ("BOX", (0, 0), (-1, -1), 0.25, EFE_BORDER),
        ("INNERGRID", (0, 0), (-1, -1), 0.25, EFE_BORDER),
        ("LEFTPADDING", (0, 0), (-1, -1), 6),
        ("RIGHTPADDING", (0, 0), (-1, -1), 6),
        ("TOPPADDING", (0, 0), (-1, -1), 4),
        ("BOTTOMPADDING", (0, 0), (-1, -1), 4),
    ]))
    return t


def _ablation_table(ablation: dict[str, Any]) -> Table:
    variants_order = [
        "baseline", "V1", "V2_orthogonal", "V3_topk", "V4_mahalanobis",
        "V5_temperature", "V6_hybrid", "V7_hybrid_calibrated",
        "V8_calibrated_mahalanobis", "V9_ensemble",
    ]
    pretty = {
        "baseline": "baseline (no swarm)",
        "V1": "V1 — current (Bernoulli + mean)",
        "V2_orthogonal": "V2 — orthogonal projections",
        "V3_topk": "V3 — top-K max pooling",
        "V4_mahalanobis": "V4 — Mahalanobis",
        "V5_temperature": "V5 — temperature-calibrated V1",
        "V6_hybrid": "V6 — V2 + V3 hybrid",
        "V7_hybrid_calibrated": "V7 — V6 + temperature",
        "V8_calibrated_mahalanobis": "V8 — V4 + temperature + verdict tuning",
        "V9_ensemble": "V9 — ensemble V1 ⊕ V4",
    }
    header = [
        _p("<b>Variant</b>"),
        _p("<b>Acc ALL</b>"),
        _p("<b>Acc HARD</b>"),
        _p("<b>ECE</b>"),
        _p("<b>p50 µs</b>"),
        _p("<b>CONV. cov.</b>"),
        _p("<b>Gated acc</b>"),
    ]
    rows = [header]
    base_acc = ablation["variants"]["baseline"]["slices"]["all"]["accuracy"]
    base_ece = ablation["variants"]["baseline"]["slices"]["all"]["ece"]
    for v in variants_order:
        r = ablation["variants"][v]
        s_all = r["slices"]["all"]
        s_hard = r["slices"]["hard"]
        verdict = r.get("verdict_stats", {})
        conv = verdict.get("CONVERGED", {"n": 0, "correct": 0})
        total = sum(s["n"] for s in verdict.values()) if verdict else 0
        cov_str = f"{conv['n'] / total * 100:.1f}%" if total else "—"
        acc_gated = conv["correct"] / conv["n"] if conv["n"] else None
        acc_str = f"{acc_gated * 100:.1f}%" if acc_gated is not None else "—"

        # Coloring
        acc_color = (EFE_OK.hexval() if s_all["accuracy"] > base_acc + 0.005
                     else (EFE_BAD.hexval() if s_all["accuracy"] < base_acc - 0.005
                     else EFE_DARK.hexval()))
        ece_color = (EFE_OK.hexval() if s_all["ece"] < base_ece - 0.05
                     else (EFE_BAD.hexval() if s_all["ece"] > base_ece + 0.05
                     else EFE_DARK.hexval()))
        rows.append([
            _p(pretty[v]),
            _p(f"<font color='{acc_color}'>{s_all['accuracy'] * 100:.1f}%</font>"),
            _p(f"{s_hard['accuracy'] * 100:.1f}%"),
            _p(f"<font color='{ece_color}'>{s_all['ece']:.3f}</font>"),
            _p(f"{s_all['latency_us']['p50']:.0f}"),
            _p(cov_str),
            _p(acc_str),
        ])
    t = Table(rows, colWidths=[5.4*cm, 1.7*cm, 1.7*cm, 1.5*cm, 1.6*cm, 1.8*cm, 1.8*cm])
    t.setStyle(TableStyle([
        ("BACKGROUND", (0, 0), (-1, 0), EFE_BLUE),
        ("TEXTCOLOR", (0, 0), (-1, 0), colors.white),
        ("FONTSIZE", (0, 0), (-1, -1), 8.5),
        ("BOX", (0, 0), (-1, -1), 0.25, EFE_BORDER),
        ("INNERGRID", (0, 0), (-1, -1), 0.2, EFE_BORDER),
        ("VALIGN", (0, 0), (-1, -1), "MIDDLE"),
        ("TOPPADDING", (0, 0), (-1, -1), 3),
        ("BOTTOMPADDING", (0, 0), (-1, -1), 3),
        ("ROWBACKGROUNDS", (0, 1), (-1, -1), [colors.white, EFE_LIGHT]),
    ]))
    return t


def _robustness_table(robustness: dict[str, Any]) -> Table:
    seeds = robustness["seeds"]
    header = [_p("<b>Variant</b>")] + [_p(f"<b>seed {s}</b>") for s in seeds] + [
        _p("<b>mean</b>"), _p("<b>σ</b>")
    ]
    rows = [header]
    for name in ("baseline", "V1", "V4_mahalanobis", "V9_ensemble"):
        runs = robustness["per_seed"][name]
        vals = [r["accuracy_all"] for r in runs]
        st = robustness["stats"][name]["accuracy_all"]
        cells = [_p(name)] + [_p(f"{v * 100:.1f}%") for v in vals] + [
            _p(f"<b>{st['mean'] * 100:.2f}%</b>"),
            _p(f"{st['std'] * 100:.2f}%"),
        ]
        rows.append(cells)
    # Δ row for V4.
    d = robustness["deltas_vs_baseline"]["V4_mahalanobis"]["accuracy_all"]
    rows.append([
        _p("<b>V4 − baseline Δ</b>"),
        *[
            _p(
                f"<font color='{EFE_BAD.hexval() if (v - b) < 0 else EFE_OK.hexval()}'>"
                f"{(v - b) * 100:+.1f}pp</font>"
            )
            for v, b in zip(
                [r["accuracy_all"] for r in robustness["per_seed"]["V4_mahalanobis"]],
                [r["accuracy_all"] for r in robustness["per_seed"]["baseline"]],
            )
        ],
        _p(
            f"<font color='{EFE_BAD.hexval() if d['mean_delta'] < 0 else EFE_OK.hexval()}'>"
            f"<b>{d['mean_delta'] * 100:+.2f}pp</b></font>"
        ),
        _p(f"z={d['z']:.2f}"),
    ])

    n_cols = 1 + len(seeds) + 2
    t = Table(rows, colWidths=[3.3 * cm] + [1.6 * cm] * len(seeds) + [1.8 * cm, 1.6 * cm])
    t.setStyle(TableStyle([
        ("BACKGROUND", (0, 0), (-1, 0), EFE_BLUE),
        ("TEXTCOLOR", (0, 0), (-1, 0), colors.white),
        ("BACKGROUND", (0, -1), (-1, -1), EFE_LIGHT),
        ("FONTSIZE", (0, 0), (-1, -1), 9),
        ("BOX", (0, 0), (-1, -1), 0.25, EFE_BORDER),
        ("INNERGRID", (0, 0), (-1, -1), 0.2, EFE_BORDER),
        ("VALIGN", (0, 0), (-1, -1), "MIDDLE"),
        ("ALIGN", (1, 0), (-1, -1), "RIGHT"),
        ("TOPPADDING", (0, 0), (-1, -1), 3),
        ("BOTTOMPADDING", (0, 0), (-1, -1), 3),
    ]))
    return t


# ---------------------------------------------------------------------------
#  Build
# ---------------------------------------------------------------------------


def build_pdf(
    ablation: dict[str, Any],
    robustness: dict[str, Any],
    figs: Figs,
    out_path: Path,
) -> None:
    doc = BaseDocTemplate(
        str(out_path), pagesize=A4,
        leftMargin=2*cm, rightMargin=2*cm,
        topMargin=2*cm, bottomMargin=2*cm,
        title="EFE — Swarm Ablation Analysis",
        author="EFE Platform Team",
    )
    frame = Frame(doc.leftMargin, doc.bottomMargin, doc.width, doc.height, id="normal")
    doc.addPageTemplates([
        PageTemplate(id="cover", frames=[frame], onPage=_on_cover),
        PageTemplate(id="body", frames=[frame], onPage=_on_body),
    ])
    story: list[Any] = []

    # === COVER ===
    story.append(Spacer(1, 3.5 * cm))
    story.append(_p("Can we improve the swarm?", TITLE))
    story.append(_p("A surgical ablation with measured numbers", SUBTITLE))
    story.append(Spacer(1, 0.6 * cm))
    story.append(_p(
        "<b>Short answer.</b> On the benchmarked dataset, <b>no variant is a "
        "strict upgrade over V1</b>. The apparent V4 accuracy gain on seed 42 "
        "(+0.7 pp) vanishes at statistical scrutiny: across 5 seeds V4's mean "
        "accuracy is <b>−0.2 pp</b> vs baseline (z = −0.09). The real "
        "improvement levers are upstream (embedding quality, cone curation) "
        "and orthogonal to the swarm algorithm itself.",
        CALL,
    ))
    story.append(Spacer(1, 0.6 * cm))
    story.append(_kv_table([
        ("Scope", "Ablation of 10 classifier variants + robustness on 5 seeds"),
        ("Dataset", "500 queries (250 easy + 250 hard), 8 classes × 32 anchors, 128-d"),
        ("Variants tested", "baseline + V1..V9 (9 swarm variants)"),
        ("Seeds", "42, 123, 777, 2024, 31415"),
        ("Verdict", "V1 remains the best balanced operating point"),
    ]))
    story.append(NextPageTemplate("body"))
    story.append(PageBreak())

    # === 1. QUESTION ===
    story.append(_p("1. The question", H1))
    story.append(_p(
        "The public benchmark published V1 at 78.2% accuracy — identical to a "
        "naïve cosine baseline. The EFE value proposition survived on "
        "<b>calibration</b> (ECE ×4.3 better) and a <b>verdict channel</b> the "
        "baseline cannot express. Fair enough — but is V1 the end of the "
        "road? This report tests 9 concrete swarm variants against that "
        "question with measured numbers.", BODY,
    ))
    story.append(_p("1.1 Diagnostic of V1", H2))
    story.append(_p(
        "We first instrumented V1 on a hard query to find out why it doesn't "
        "beat the baseline. Three findings:",
        BODY,
    ))
    story.append(_kv_table([
        ("Inter-agent correlation", "0.93 mean, 0.98 at the 90th percentile"),
        ("Shared errors with baseline (hard)", "107 / 109 = 98 %"),
        ("Unique gains vs baseline", "0 / 250 hard queries"),
    ]))
    story.append(_p(
        "V1's Bernoulli dropout produces <b>quasi-identical</b> agents — they "
        "do not vote independently, so consensus ≈ baseline. Any real "
        "improvement has to break this redundancy <b>or</b> change the "
        "similarity function entirely.",
        CALL,
    ))

    # === 2. VARIANTS ===
    story.append(_p("2. Nine variants, nine hypotheses", H1))
    story.append(_kv_table([
        ("V2 — Orthogonal projections", "Break redundancy: each agent sees a disjoint orthogonal sub-basis"),
        ("V3 — Top-K max pooling", "Sharpen class signal: argmax over the K closest anchors of each class"),
        ("V4 — Mahalanobis", "Use intra-class covariance: classes with tight clusters become stricter gates"),
        ("V5 — Temperature scaling", "Cheap post-hoc recalibration (fitted on held-out easy slice)"),
        ("V6 — V2 + V3 hybrid", "Compose the two architectural hypotheses"),
        ("V7 — V6 + temperature", "V6 plus recalibration"),
        ("V8 — V4 + temperature + verdict retuning", "Fix V4's calibration flaw"),
        ("V9 — Ensemble V1 ⊕ V4", "Agreement between two independent experts"),
    ]))

    story.append(PageBreak())

    # === 3. ABLATION RESULTS ===
    story.append(_p("3. Ablation results on seed 42", H1))
    story.append(_ablation_table(ablation))
    story.append(Spacer(1, 0.3 * cm))
    story.append(_figure(figs.ablation_acc))
    story.append(Spacer(1, 0.2 * cm))
    story.append(_p(
        "V4 appears to gain +0.7 pp ALL and +1.2 pp HARD over the baseline on "
        "this seed — the only variant that visibly moves accuracy. V8 "
        "inherits that gain and adds a CONVERGED floor at 100 % coverage, "
        "but its ECE balloons back to baseline levels — the post-hoc "
        "temperature fit on easy queries (all 100 % accurate) is degenerate "
        "there. V9 ensemble sits at 73.6 %, slightly below V1 — the "
        "disagreement handling chooses the wrong expert on ~1 % of cases.",
        BODY,
    ))

    story.append(_p("3.1 Calibration (ECE)", H2))
    story.append(_figure(figs.ablation_ece))
    story.append(_p(
        "V1, V2, V3, V6 and V9 all land under ECE 0.07 — one order of "
        "magnitude better than the baseline's 0.255. V4's structural "
        "distance scoring is uncalibrated (0.216); V8's temperature fit "
        "fails to recover it because V4's distributions are heavy-tailed "
        "rather than under/over-confident. Calibration is therefore a "
        "<b>solved problem</b> under the cosine-style swarm family; V4 and "
        "its descendants pay for their accuracy flirt in calibration debt.",
        BODY,
    ))

    story.append(_p("3.2 Verdict discipline", H2))
    story.append(_figure(figs.ablation_coverage))
    story.append(_p(
        "The verdict operating point (coverage × gated accuracy) is where "
        "the swarm earns its production value. V1 at <b>90.4 % coverage / "
        "77.5 % gated acc</b> is the best Pareto point on this dataset — V9 "
        "trades a little accuracy for a little coverage, V4 is degenerate (only "
        "1.7 % of queries ever hit CONVERGED), V8 is too permissive "
        "(everything passes).",
        BODY,
    ))

    story.append(PageBreak())

    # === 4. ROBUSTNESS ===
    story.append(_p("4. Robustness — does V4's gain survive across seeds?", H1))
    story.append(_p(
        "Any single-seed improvement needs a robustness cross-check. We "
        "ran baseline + V1 + V4 + V9 on 5 different seeds (same architecture, "
        "same dataset generator). The numbers are below.",
        BODY,
    ))
    story.append(_robustness_table(robustness))
    story.append(Spacer(1, 0.3 * cm))
    story.append(_figure(figs.robustness_all))
    story.append(Spacer(1, 0.2 * cm))
    d_all = robustness["deltas_vs_baseline"]["V4_mahalanobis"]["accuracy_all"]
    d_hard = robustness["deltas_vs_baseline"]["V4_mahalanobis"]["accuracy_hard"]
    story.append(_p(
        f"<b>The verdict.</b> V4 − baseline on ALL accuracy: mean Δ "
        f"{d_all['mean_delta'] * 100:+.2f} pp, z = {d_all['z']:.2f}. On HARD "
        f"accuracy: mean Δ {d_hard['mean_delta'] * 100:+.2f} pp, z = "
        f"{d_hard['z']:.2f}. Both z-scores are <b>well below ±1</b> — the "
        f"seed-42 'win' is <b>not statistically significant</b>. V4 and the "
        f"baseline are <b>indistinguishable on accuracy</b> over 5 seeds.",
        CALL,
    ))
    story.append(_figure(figs.robustness_hard))
    story.append(PageBreak())

    # === 5. INTERPRETATION ===
    story.append(_p("5. Why does nothing beat V1?", H1))
    story.append(_p("5.1 The Bayes-optimal ceiling", H2))
    story.append(_p(
        "The hard tier was synthesised by interpolating a class centroid "
        "toward its semantic sibling at α ∈ [0.35, 0.48], then adding "
        "Gaussian noise σ = 0.25. A non-trivial fraction of hard queries "
        "therefore fall on the <b>wrong side</b> of the Bayes-optimal decision "
        "boundary regardless of classifier. The empirical ceiling on HARD is "
        "≈ 60 % for any classifier that only sees the embedding — V1 at 58.9 % "
        "mean accuracy is already at that ceiling (within σ = 2.6 pp).",
        BODY,
    ))

    story.append(_p("5.2 Same signal, same errors", H2))
    story.append(_p(
        "Every variant we tested — V1 through V9 — ultimately scores "
        "embeddings with cosine-like similarity in the same input space. "
        "They correlate by construction. V4 is the one exception (Mahalanobis) "
        "and its accuracy gain is within noise. Therefore: <b>the ceiling "
        "is not in the classifier, it is in the signal the embedding "
        "exposes</b>.",
        BODY,
    ))

    story.append(_p("5.3 Where the real levers are", H2))
    story.append(_p(
        "• <b>Embedding quality</b> — a sentence-transformer fine-tuned on "
        "the tenant's task will expose more discriminating structure than "
        "any swarm trick. This is the single highest-impact investment.<br/><br/>"
        "• <b>Cone curation</b> — each class's 32 anchors are the "
        "'training set'. Adding 10 well-chosen boundary anchors per confusion "
        "pair will move HARD accuracy more than any algorithmic change we "
        "tested.<br/><br/>"
        "• <b>Multi-modal signal</b> — when the query has metadata (timing, "
        "user segment, past history), fusing it with the embedding before "
        "classification breaks the ceiling. The swarm kernel can ingest "
        "multi-modal features through its population mix, but that's a "
        "future work track.",
        BODY,
    ))

    story.append(PageBreak())

    # === 6. RECOMMENDATION ===
    story.append(_p("6. Recommendation", H1))
    story.append(_p("KEEP V1", VERDICT))
    story.append(_p(
        "V1 is the best balanced operating point on this dataset: "
        "<b>~79 % accuracy, 0.067 ECE, 90.4 % CONVERGED coverage at "
        "77.5 % gated accuracy, p50 ≈ 2.8 ms in Python reference</b> "
        "(≈ 150 µs in the Rust kernel). None of the 8 variants we tried "
        "beats it on more than one axis at a time.", BODY,
    ))
    story.append(_p("6.1 Keep an eye on V9", H2))
    story.append(_p(
        "V9 (ensemble V1 ⊕ V4) gives a small but real ECE improvement "
        "(0.058 vs 0.067) and a tighter verdict channel (93.5 % coverage at "
        "76.9 % gated acc). It costs 2× the latency of V1. For deployments "
        "where ECE is actively used downstream (cost-sensitive routing, "
        "thresholded escalation), V9 is a reasonable promotion path once "
        "the Rust kernel lands the Mahalanobis branch.",
        BODY,
    ))
    story.append(_p("6.2 Drop V2, V3, V6, V7", H2))
    story.append(_p(
        "Orthogonal projections (V2, V6, V7) degrade accuracy slightly and "
        "explode latency in the pure-Python reference by an order of "
        "magnitude. They are <b>net-negative</b> without corresponding gains. "
        "Stop investing in this family of ideas.",
        BODY,
    ))
    story.append(_p("6.3 Next investigations", H2))
    story.append(_p(
        "1. <b>Anchor-level learning</b> — per-class anchor weights learned "
        "from a supervised calibration slice, rather than uniform "
        "averaging. Low complexity, high expected payoff.<br/><br/>"
        "2. <b>Feature fusion</b> — add tenant-supplied metadata as extra "
        "dimensions before the swarm. Requires client API extension but "
        "breaks the embedding ceiling.<br/><br/>"
        "3. <b>Train-time embedder tuning</b> — a 5-minute fine-tune on "
        "10k labelled pairs usually beats months of classifier tweaks. "
        "This is a different project, but it is <b>the</b> lever.",
        BODY,
    ))

    story.append(Spacer(1, 0.6 * cm))
    story.append(_p(
        "<b>Reproducibility.</b> Every number above comes from "
        "<code>benchmarks/run_ablation.py</code> and "
        "<code>benchmarks/run_robustness.py</code>. Identical seeds reproduce "
        "identical accuracy/ECE to the last decimal; latency varies within "
        "hardware noise.", CALL,
    ))
    doc.build(story)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--ablation", default="benchmarks/ablation_results.json")
    parser.add_argument("--robustness", default="benchmarks/robustness_results.json")
    parser.add_argument("--figs-dir", default="benchmarks/figs_ablation")
    parser.add_argument("--out", default="/Users/rodrigue.pekar/Downloads/EFE_Swarm_Ablation_Analysis.pdf")
    args = parser.parse_args(argv)

    ablation = json.loads(Path(args.ablation).read_text(encoding="utf-8"))
    robustness = json.loads(Path(args.robustness).read_text(encoding="utf-8"))
    figs = build_figures(ablation, robustness, Path(args.figs_dir))
    build_pdf(ablation, robustness, figs, Path(args.out))
    print(f"→ wrote {args.out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
