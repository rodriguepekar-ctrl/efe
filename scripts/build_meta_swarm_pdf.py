#!/usr/bin/env python3
"""
PR #5..#15 — Meta-Swarm Architecture Compendium PDF builder.

Reads every `docs/EFE_A1_PHASE2_PR??_ARCHITECTURE.md` + the master
Phase-2 doc, parses markdown line-by-line, and emits a single PDF
with a cover page, master table-of-contents, and one chapter per PR.

Output : `EFE_A1_PHASE2_META_SWARM_COMPENDIUM.pdf` at repo root.

Dependencies : reportlab >= 4.0 (mandatory).  Pygments / markdown are
NOT required — we do parsing in pure stdlib for predictability.
"""

from __future__ import annotations

import os
import re
import sys
from dataclasses import dataclass

from reportlab.lib import colors
from reportlab.lib.pagesizes import A4
from reportlab.lib.styles import ParagraphStyle, getSampleStyleSheet
from reportlab.lib.units import cm
from reportlab.lib.enums import TA_CENTER, TA_LEFT
from reportlab.platypus import (
    BaseDocTemplate, Frame, KeepTogether, PageBreak, PageTemplate,
    Paragraph, Preformatted, Spacer, Table, TableStyle,
)

# ─── Paths ──────────────────────────────────────────────────────────
REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
DOCS_DIR = os.path.join(REPO_ROOT, "docs")
OUTPUT = os.path.join(REPO_ROOT, "EFE_A1_PHASE2_META_SWARM_COMPENDIUM.pdf")

# ─── Chapter ordering ───────────────────────────────────────────────
# Audit-pass expansion : the original 12 chapters (Master + PR05-PR15)
# missed FIVE artefacts found during deep cross-check :
#   - Phase 2 roadmap (PR catalog of 25 PRs)
#   - Phase 1 PR #1 / #3 / #4 (founding work on the crate before P2)
#   - Phase 2 threat model (frozen security artefact — distinct from
#     PR #7's execution-ready plan doc that describes how the threat
#     model is AUTHORED)
#
# Each entry is (display label, path relative to repo root, human title).
CHAPTERS: list[tuple[str, str, str]] = [
    # ─── Part I : Master + Roadmap ───
    ("Master",
     "docs/EFE_A1_PHASE2_ARCHITECTURE.md",
     "Phase-2 master architecture (overarching scope)"),
    ("Roadmap",
     "docs/EFE_A1_PHASE2_ROADMAP.md",
     "Phase-2 roadmap : PR catalog (25 PRs), sprints, cadence, L2 gate"),

    # ─── Part II : Phase-1 foundations (the crate before Phase 2) ───
    ("Phase-1 PR #1",
     "crates/efe-meta-swarm/PR_DESCRIPTION.md",
     "Scaffold hierarchical multi-swarm crate (foundational)"),
    ("Phase-1 PR #3",
     "crates/efe-meta-swarm/PR3_DESCRIPTION.md",
     "200-proptest target (130+ new cases)"),
    ("Phase-1 PR #4",
     "crates/efe-meta-swarm/PR4_DESCRIPTION.md",
     "Bench baseline tracking + master-gate hook"),

    # ─── Part III : Phase-2 security artefact ───
    ("Threat Model",
     "docs/security/PHASE2_THREAT_MODEL.md",
     "Phase-2 threat model (frozen security artefact)"),

    # ─── Part IV : Phase-2 PRs (chronological — 11 of 25 PRs documented) ───
    ("PR #5",
     "docs/EFE_A1_PHASE2_PR05_ARCHITECTURE.md",
     "efe-meta-swarm-proto scaffold"),
    ("PR #6",
     "docs/EFE_A1_PHASE2_PR06_ARCHITECTURE.md",
     "efe-meta-swarm-server scaffold"),
    ("PR #7",
     "docs/EFE_A1_PHASE2_PR07_ARCHITECTURE.md",
     "Phase-2 threat model (execution plan)"),
    ("PR #8",
     "docs/EFE_A1_PHASE2_PR08_ARCHITECTURE.md",
     "MetaSwarmService::Iterate wiring"),
    ("PR #9",
     "docs/EFE_A1_PHASE2_PR09_ARCHITECTURE.md",
     "MetaParticipantService::Step wiring"),
    ("PR #10",
     "docs/EFE_A1_PHASE2_PR10_ARCHITECTURE.md",
     "integration_grpc_multiprocess"),
    ("PR #11",
     "docs/EFE_A1_PHASE2_PR11_ARCHITECTURE.md",
     "EFE_META_SWARM_ENABLED plumbing"),
    ("PR #12",
     "docs/EFE_A1_PHASE2_PR12_ARCHITECTURE.md",
     "SIGHUP + ReloadConfig RPC"),
    ("PR #13",
     "docs/EFE_A1_PHASE2_PR13_ARCHITECTURE.md",
     "Helm sub-chart skeleton"),
    ("PR #14",
     "docs/EFE_A1_PHASE2_PR14_ARCHITECTURE.md",
     "Helm values + templates (production-grade)"),
    ("PR #15",
     "docs/EFE_A1_PHASE2_PR15_ARCHITECTURE.md",
     "helm-meta-swarm-lint workflow"),
]

# Phase-2 PRs in the roadmap catalog WITHOUT an architecture doc YET.
# Surfaced on the cover + ToC so stakeholders see the gap.
FUTURE_PRS: list[tuple[str, str]] = [
    ("PR #16", "mTLS + cert-manager Issuer"),
    ("PR #17", "14 Prometheus metrics"),
    ("PR #18", "8 OpenTelemetry span types"),
    ("PR #19", "Grafana dashboards + alert rules"),
    ("PR #20", "loadgen traffic generator"),
    ("PR #21", "public-bench harness + 5 workloads"),
    ("PR #22", "4 SLOs + rollup pin"),
    ("PR #23", "smoke rollup pin"),
    ("PR #24", "publish bench results"),
    ("PR #25", "Phase-2 lessons + soak report + roadmap refresh"),
]

# ─── Styles ─────────────────────────────────────────────────────────
ss = getSampleStyleSheet()

# Body — Helvetica 9/12.
BODY = ParagraphStyle(
    "Body",
    parent=ss["BodyText"],
    fontName="Helvetica",
    fontSize=9,
    leading=12,
    spaceAfter=4,
    alignment=TA_LEFT,
)

# Heading hierarchy.
H1 = ParagraphStyle(
    "H1", parent=ss["Heading1"],
    fontName="Helvetica-Bold", fontSize=18, leading=22,
    spaceBefore=18, spaceAfter=10,
    textColor=colors.HexColor("#1a1a1a"),
    keepWithNext=True,
)
H2 = ParagraphStyle(
    "H2", parent=ss["Heading2"],
    fontName="Helvetica-Bold", fontSize=14, leading=18,
    spaceBefore=14, spaceAfter=8,
    textColor=colors.HexColor("#2c2c2c"),
    keepWithNext=True,
)
H3 = ParagraphStyle(
    "H3", parent=ss["Heading3"],
    fontName="Helvetica-Bold", fontSize=11.5, leading=14,
    spaceBefore=10, spaceAfter=6,
    textColor=colors.HexColor("#3c3c3c"),
    keepWithNext=True,
)
H4 = ParagraphStyle(
    "H4", parent=ss["Heading4"],
    fontName="Helvetica-Bold", fontSize=10, leading=12,
    spaceBefore=8, spaceAfter=4,
    textColor=colors.HexColor("#444"),
    keepWithNext=True,
)
H5 = ParagraphStyle(
    "H5", parent=ss["Heading5"],
    fontName="Helvetica-Oblique", fontSize=9.5, leading=11,
    spaceBefore=6, spaceAfter=3,
    keepWithNext=True,
)

# Code blocks — Courier 7/9.
CODE = ParagraphStyle(
    "Code", parent=ss["Code"],
    fontName="Courier", fontSize=7, leading=8.6,
    leftIndent=0.4 * cm,
    backColor=colors.HexColor("#f4f4f4"),
    borderColor=colors.HexColor("#dddddd"),
    borderWidth=0.4,
    borderPadding=4,
    spaceBefore=4,
    spaceAfter=4,
    wordWrap="CJK",
)

# Bullet item — small left indent + bullet glyph in text.
BULLET = ParagraphStyle(
    "Bullet", parent=BODY,
    leftIndent=0.5 * cm, bulletIndent=0.1 * cm,
    spaceAfter=2,
)

# Cover styles.
COVER_TITLE = ParagraphStyle(
    "CoverTitle", fontName="Helvetica-Bold", fontSize=32,
    leading=38, alignment=TA_CENTER,
    textColor=colors.HexColor("#1a1a1a"),
)
COVER_SUBTITLE = ParagraphStyle(
    "CoverSub", fontName="Helvetica", fontSize=15,
    leading=20, alignment=TA_CENTER,
    textColor=colors.HexColor("#555"),
)
COVER_META = ParagraphStyle(
    "CoverMeta", fontName="Helvetica", fontSize=10,
    leading=14, alignment=TA_CENTER,
    textColor=colors.HexColor("#888"),
)

# Chapter divider title (the "PR #N" page).
CHAPTER_TITLE = ParagraphStyle(
    "ChapterTitle", fontName="Helvetica-Bold", fontSize=28,
    leading=34, alignment=TA_CENTER,
    textColor=colors.HexColor("#1a1a1a"),
)
CHAPTER_SUBTITLE = ParagraphStyle(
    "ChapterSubtitle", fontName="Helvetica", fontSize=13,
    leading=18, alignment=TA_CENTER,
    textColor=colors.HexColor("#666"),
)

# Table cell style.
TBL_CELL = ParagraphStyle(
    "TblCell", parent=BODY, fontSize=8, leading=10, spaceAfter=0,
)
TBL_HEAD = ParagraphStyle(
    "TblHead", parent=TBL_CELL, fontName="Helvetica-Bold",
)

# ─── Emoji → text replacement ──────────────────────────────────────
# Helvetica doesn't render unicode emoji ; replace with bracketed
# tokens that survive the PDF rendering cleanly.
EMOJI_MAP = {
    "✅": "[OK]",
    "❌": "[FAIL]",
    "⚠️": "[WARN]",
    "⚠": "[WARN]",
    "🔴": "[CRIT]",
    "🟠": "[HIGH]",
    "🟡": "[MED]",
    "🟢": "[LOW]",
    "🚫": "[BLK]",
    "⏭️": "[SKIP]",
    "⏭": "[SKIP]",
    "❓": "[?]",
    "✓": "[v]",
    "✗": "[x]",
    "→": "->",
    "←": "<-",
    "⇒": "=>",
    "⇐": "<=",
    "—": "--",
    "–": "-",
    "…": "...",
    "•": "*",
    "ℹ️": "[INFO]",
    "ℹ": "[INFO]",
    "═": "=",
    "─": "-",
    "│": "|",
    "┌": "+",
    "┐": "+",
    "└": "+",
    "┘": "+",
    "├": "+",
    "┤": "+",
    "┬": "+",
    "┴": "+",
    "┼": "+",
    "↔": "<->",
    "↑": "^",
    "↓": "v",
    "≤": "<=",
    "≥": ">=",
    "≠": "!=",
    "×": "x",
    "“": "\"",
    "”": "\"",
    "‘": "'",
    "’": "'",
    "α": "alpha",
    "β": "beta",
    "γ": "gamma",
    "μ": "u",
    "π": "pi",
    "Σ": "sigma",
    "✋": "[stop]",
    "🔒": "[lock]",
    "📦": "[pkg]",
    "🎯": "[target]",
    "🛡": "[shield]",
    "🛡️": "[shield]",
    "🏷": "[tag]",
    "🏷️": "[tag]",
    "📊": "[chart]",
}


def strip_emoji(s: str) -> str:
    for k, v in EMOJI_MAP.items():
        s = s.replace(k, v)
    return s


# ─── Inline markdown formatting ────────────────────────────────────
def inline_md(text: str) -> str:
    """Convert inline markdown to reportlab Paragraph-friendly markup.

    Order is critical : escape XML first, then re-apply markup tags
    that reportlab understands (<b>, <i>, <font>).
    """
    text = strip_emoji(text)
    # XML-escape & < > first.
    text = text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")

    # Backtick `inline code` -> mono-spaced font.
    text = re.sub(
        r"`([^`]+)`",
        lambda m: f'<font face="Courier" size="8" color="#444444">{m.group(1)}</font>',
        text,
    )

    # **bold**
    text = re.sub(r"\*\*([^*]+)\*\*", r"<b>\1</b>", text)
    # *italic* (avoid double-asterisks)
    text = re.sub(r"(?<![*\\])\*([^*\n]+)\*(?!\*)", r"<i>\1</i>", text)

    # [text](url) -> just text (link rendering is verbose for compendium).
    text = re.sub(r"\[([^\]]+)\]\([^)]+\)", r"\1", text)

    return text


def safe_paragraph(text: str, style: ParagraphStyle):
    """Build a Paragraph that won't crash on malformed inline markup."""
    try:
        return Paragraph(text, style)
    except Exception:
        # Fallback : strip ALL markup.
        plain = re.sub(r"<[^>]+>", "", text)
        return Paragraph(plain.replace("&", "&amp;"), style)


# ─── Block-level markdown parser ───────────────────────────────────
HEADER_STYLES = [H1, H2, H3, H4, H5, H5]


def _parse_table_separator(line: str) -> bool:
    """Detect a markdown table separator line `|---|:---:|---:|`."""
    s = line.strip().strip("|")
    if not s:
        return False
    parts = [p.strip() for p in s.split("|")]
    return all(re.match(r"^:?-+:?$", p) for p in parts)


def _split_table_row(line: str) -> list[str]:
    """Split a table row, trimming outer pipes."""
    s = line.strip()
    if s.startswith("|"):
        s = s[1:]
    if s.endswith("|"):
        s = s[:-1]
    return [c.strip() for c in s.split("|")]


def parse_markdown(content: str, story: list) -> None:
    """Append flowables to `story` from markdown `content`."""
    lines = content.split("\n")
    i = 0
    n = len(lines)
    while i < n:
        line = lines[i]
        stripped = line.strip()

        # Blank.
        if not stripped:
            i += 1
            continue

        # Headers # .. ######
        m = re.match(r"^(#{1,6})\s+(.+?)\s*#*\s*$", line)
        if m:
            level = len(m.group(1))
            text = inline_md(m.group(2))
            style = HEADER_STYLES[level - 1]
            story.append(safe_paragraph(text, style))
            i += 1
            continue

        # Fenced code block ``` ... ```
        if stripped.startswith("```"):
            i += 1
            code_lines = []
            while i < n and not lines[i].strip().startswith("```"):
                code_lines.append(lines[i].rstrip("\r"))
                i += 1
            i += 1  # skip closing fence
            # Truncate excessively-long lines that would overflow the page.
            normalized = []
            for cl in code_lines:
                cl = strip_emoji(cl).replace("\t", "    ")
                # Replace XML-unsafe chars for Preformatted.
                cl = cl.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
                # Hard wrap at 105 chars (with continuation marker).
                while len(cl) > 105:
                    normalized.append(cl[:105] + " ↩")
                    cl = "    " + cl[105:]
                normalized.append(cl)
            try:
                story.append(Preformatted("\n".join(normalized), CODE))
            except Exception:
                story.append(safe_paragraph("&lt;code block render error&gt;", BODY))
            continue

        # Horizontal rule ---
        if re.match(r"^\s*-{3,}\s*$", line):
            story.append(Spacer(1, 6))
            i += 1
            continue

        # Block-quote >  (treat as italic indented).
        if stripped.startswith(">"):
            qlines = []
            while i < n and lines[i].strip().startswith(">"):
                qlines.append(lines[i].strip().lstrip(">").strip())
                i += 1
            text = " ".join(qlines)
            quote_style = ParagraphStyle(
                "Quote", parent=BODY, leftIndent=0.5 * cm,
                fontName="Helvetica-Oblique", textColor=colors.HexColor("#555"),
            )
            story.append(safe_paragraph(inline_md(text), quote_style))
            continue

        # Table.
        if stripped.startswith("|") and i + 1 < n and _parse_table_separator(lines[i + 1]):
            header = _split_table_row(line)
            i += 2  # past header + separator
            rows = []
            while i < n and lines[i].strip().startswith("|"):
                rows.append(_split_table_row(lines[i]))
                i += 1
            # Normalize row widths.
            ncols = len(header)
            for r in rows:
                while len(r) < ncols:
                    r.append("")
                if len(r) > ncols:
                    r[:] = r[:ncols]
            # Build Paragraph cells.
            data = [[safe_paragraph(inline_md(c), TBL_HEAD) for c in header]]
            for r in rows:
                data.append([safe_paragraph(inline_md(c), TBL_CELL) for c in r])
            # Column widths : equal split of (A4 width − margins ~ 17 cm).
            available = 17 * cm
            col_w = available / max(ncols, 1)
            try:
                tbl = Table(data, colWidths=[col_w] * ncols, repeatRows=1)
                tbl.setStyle(TableStyle([
                    ("BACKGROUND", (0, 0), (-1, 0), colors.HexColor("#e8e8e8")),
                    ("GRID", (0, 0), (-1, -1), 0.3, colors.HexColor("#bbbbbb")),
                    ("FONTSIZE", (0, 0), (-1, -1), 8),
                    ("VALIGN", (0, 0), (-1, -1), "TOP"),
                    ("LEFTPADDING", (0, 0), (-1, -1), 3),
                    ("RIGHTPADDING", (0, 0), (-1, -1), 3),
                    ("TOPPADDING", (0, 0), (-1, -1), 2),
                    ("BOTTOMPADDING", (0, 0), (-1, -1), 2),
                ]))
                story.append(tbl)
            except Exception:
                # Fallback : render as plain rows.
                for r in [header, *rows]:
                    story.append(safe_paragraph(inline_md(" | ".join(r)), BODY))
            story.append(Spacer(1, 4))
            continue

        # Bullet list.
        if re.match(r"^\s*[-*]\s+", line):
            while i < n and re.match(r"^\s*[-*]\s+", lines[i]):
                text = re.sub(r"^\s*[-*]\s+", "", lines[i])
                story.append(safe_paragraph("&bull; " + inline_md(text), BULLET))
                i += 1
            continue

        # Numbered list.
        if re.match(r"^\s*\d+\.\s+", line):
            counter = 0
            while i < n and re.match(r"^\s*\d+\.\s+", lines[i]):
                counter += 1
                text = re.sub(r"^\s*\d+\.\s+", "", lines[i])
                story.append(safe_paragraph(f"{counter}. " + inline_md(text), BULLET))
                i += 1
            continue

        # Normal paragraph : gather until blank / block-element.
        para_lines = [line]
        i += 1
        block_starts = (
            "#", "|", "```", ">",
        )
        while i < n:
            nxt = lines[i]
            nxt_stripped = nxt.strip()
            if not nxt_stripped:
                break
            if nxt_stripped[0] in block_starts:
                break
            if re.match(r"^\s*-{3,}\s*$", nxt):
                break
            if re.match(r"^\s*[-*]\s+", nxt):
                break
            if re.match(r"^\s*\d+\.\s+", nxt):
                break
            if re.match(r"^(#{1,6})\s+", nxt):
                break
            para_lines.append(nxt)
            i += 1
        text = " ".join(p.strip() for p in para_lines).strip()
        if text:
            story.append(safe_paragraph(inline_md(text), BODY))


# ─── Page header / footer ──────────────────────────────────────────
@dataclass
class _State:
    current_chapter: str = ""


_state = _State()


def header_footer(canvas, doc):
    canvas.saveState()
    canvas.setFont("Helvetica", 7.5)
    canvas.setFillColor(colors.HexColor("#777"))
    # Header line.
    canvas.drawString(
        2 * cm, A4[1] - 1.3 * cm,
        "EFE-A1 Phase 2 / Meta-Swarm Architecture Compendium")
    canvas.drawRightString(
        A4[0] - 2 * cm, A4[1] - 1.3 * cm,
        _state.current_chapter or "")
    # Separator under header.
    canvas.setStrokeColor(colors.HexColor("#ddd"))
    canvas.setLineWidth(0.4)
    canvas.line(2 * cm, A4[1] - 1.5 * cm, A4[0] - 2 * cm, A4[1] - 1.5 * cm)
    # Footer page number.
    canvas.drawCentredString(
        A4[0] / 2, 1.2 * cm,
        f"— {doc.page} —")
    canvas.restoreState()


class ChapterMark:
    """Flowable-shaped marker that updates the running header on draw."""
    def __init__(self, label: str):
        self.label = label

    def wrap(self, *_):
        return (0, 0)

    def draw(self):
        pass

    def split(self, *_):
        return [self]


# A real reportlab Flowable subclass that sets state on draw.
from reportlab.platypus.flowables import Flowable


class _SetHeader(Flowable):
    def __init__(self, label: str):
        super().__init__()
        self.label = label

    def wrap(self, _w, _h):
        return (0, 0)

    def draw(self):
        _state.current_chapter = self.label


# ─── Story assembly ────────────────────────────────────────────────
def build_story() -> list:
    story: list = []

    # ── Cover page ──
    story.append(Spacer(1, 4.5 * cm))
    story.append(Paragraph("EFE-A1", COVER_TITLE))
    story.append(Spacer(1, 0.4 * cm))
    story.append(Paragraph("Meta-Swarm Architecture Compendium", COVER_SUBTITLE))
    story.append(Spacer(1, 1 * cm))
    story.append(Paragraph(
        "Phase-1 foundations &nbsp;+&nbsp; Phase-2 master &nbsp;+&nbsp; roadmap &nbsp;+&nbsp; "
        "threat model &nbsp;+&nbsp; 11 PRs (#5 → #15)",
        COVER_META))
    story.append(Spacer(1, 0.4 * cm))
    story.append(Paragraph(
        f"{len(CHAPTERS)} chapters &nbsp;|&nbsp; {len(FUTURE_PRS)} PRs in catalog without doc yet",
        COVER_META))
    story.append(Spacer(1, 1.2 * cm))
    story.append(Paragraph(
        "Joint contractive iteration controller + N participants. "
        "Crate scaffold (Phase 1) -> proto + server + threat model -> "
        "gRPC wiring -> SIGHUP hot-reload -> Helm sub-chart -> production-grade "
        "values + templates -> CI lint workflow (Phase 2 to date).",
        COVER_META))
    story.append(Spacer(1, 3 * cm))
    story.append(Paragraph(
        "Generated for stakeholder review.",
        COVER_META))
    story.append(PageBreak())

    # ── Master ToC overview ──
    story.append(_SetHeader("Table of contents"))
    story.append(Paragraph("Table of contents", H1))
    toc_data = [["Chapter", "Title", "LOC"]]
    total_loc = 0
    for label, fn, title in CHAPTERS:
        path = os.path.join(REPO_ROOT, fn)
        loc = sum(1 for _ in open(path, "r", encoding="utf-8")) if os.path.exists(path) else 0
        total_loc += loc
        toc_data.append([label, title, f"{loc:,}"])
    toc_data.append(["", "Total source LOC", f"{total_loc:,}"])
    toc_tbl = Table(
        [
            [safe_paragraph(c, TBL_HEAD) for c in toc_data[0]],
        ] + [
            [safe_paragraph(c, TBL_CELL) for c in row] for row in toc_data[1:]
        ],
        colWidths=[3 * cm, 11 * cm, 2 * cm],
        repeatRows=1,
    )
    toc_tbl.setStyle(TableStyle([
        ("BACKGROUND", (0, 0), (-1, 0), colors.HexColor("#e8e8e8")),
        ("BACKGROUND", (0, -1), (-1, -1), colors.HexColor("#f5f5f5")),
        ("GRID", (0, 0), (-1, -1), 0.3, colors.HexColor("#bbb")),
        ("FONTSIZE", (0, 0), (-1, -1), 9),
        ("VALIGN", (0, 0), (-1, -1), "TOP"),
        ("ALIGN", (-1, 0), (-1, -1), "RIGHT"),
        ("FONTNAME", (0, -1), (-1, -1), "Helvetica-Bold"),
        ("LEFTPADDING", (0, 0), (-1, -1), 4),
        ("RIGHTPADDING", (0, 0), (-1, -1), 4),
        ("TOPPADDING", (0, 0), (-1, -1), 3),
        ("BOTTOMPADDING", (0, 0), (-1, -1), 3),
    ]))
    story.append(toc_tbl)
    story.append(Spacer(1, 0.6 * cm))

    # ── Future-work table : PRs in the roadmap catalog without doc yet. ──
    story.append(Paragraph(
        "Phase-2 PRs planned in the roadmap catalog but NOT YET documented "
        "(architecture docs pending)", H3))
    fut_data = [["#", "Title"]]
    for label, title in FUTURE_PRS:
        fut_data.append([label, title])
    fut_tbl = Table(
        [
            [safe_paragraph(c, TBL_HEAD) for c in fut_data[0]],
        ] + [
            [safe_paragraph(c, TBL_CELL) for c in row] for row in fut_data[1:]
        ],
        colWidths=[3 * cm, 13 * cm],
        repeatRows=1,
    )
    fut_tbl.setStyle(TableStyle([
        ("BACKGROUND", (0, 0), (-1, 0), colors.HexColor("#fff3cd")),
        ("GRID", (0, 0), (-1, -1), 0.3, colors.HexColor("#ccc")),
        ("FONTSIZE", (0, 0), (-1, -1), 8.5),
        ("VALIGN", (0, 0), (-1, -1), "TOP"),
        ("LEFTPADDING", (0, 0), (-1, -1), 4),
        ("RIGHTPADDING", (0, 0), (-1, -1), 4),
        ("TOPPADDING", (0, 0), (-1, -1), 3),
        ("BOTTOMPADDING", (0, 0), (-1, -1), 3),
    ]))
    story.append(fut_tbl)
    story.append(Spacer(1, 0.5 * cm))

    story.append(Paragraph(
        "Rendering notes : each chapter is the original markdown rendered "
        "verbatim. Code blocks wrap at 105 chars with continuation marker. "
        "Emoji are replaced by bracketed tokens (e.g. [OK], [CRIT]) so "
        "the PDF stays printable on monochrome printers. Tables retain "
        "header repeat across page breaks.", BODY))
    story.append(PageBreak())

    # ── Per-chapter rendering ──
    for idx, (label, fn, title) in enumerate(CHAPTERS, start=1):
        path = os.path.join(REPO_ROOT, fn)
        if not os.path.exists(path):
            print(f"  [skip] missing : {path}", file=sys.stderr)
            continue

        # Chapter divider page.
        story.append(_SetHeader(f"{label} — {title}"))
        story.append(Spacer(1, 4.5 * cm))
        story.append(Paragraph(label, CHAPTER_TITLE))
        story.append(Spacer(1, 0.5 * cm))
        story.append(Paragraph(title, CHAPTER_SUBTITLE))
        story.append(Spacer(1, 0.6 * cm))
        try:
            n_lines = sum(1 for _ in open(path, "r", encoding="utf-8"))
        except Exception:
            n_lines = 0
        story.append(Paragraph(
            f"Chapter {idx} of {len(CHAPTERS)} — source : "
            f"<font face='Courier' size='8'>{fn}</font> ({n_lines:,} LOC)",
            CHAPTER_SUBTITLE,
        ))
        story.append(PageBreak())

        # Render markdown.
        with open(path, "r", encoding="utf-8") as f:
            content = f.read()
        parse_markdown(content, story)
        story.append(PageBreak())

    return story


# ─── Build the PDF ─────────────────────────────────────────────────
def main() -> int:
    print(f"── building : {OUTPUT}")
    print(f"   chapters : {len(CHAPTERS)}")
    print(f"   source   : {DOCS_DIR}")

    doc = BaseDocTemplate(
        OUTPUT,
        pagesize=A4,
        leftMargin=2 * cm,
        rightMargin=2 * cm,
        topMargin=2 * cm,
        bottomMargin=2 * cm,
        title="EFE-A1 Phase 2 — Meta-Swarm Architecture Compendium",
        author="EFE platform team",
    )
    frame = Frame(
        doc.leftMargin,
        doc.bottomMargin,
        doc.width,
        doc.height,
        id="main",
        showBoundary=0,
    )
    template = PageTemplate(id="main", frames=[frame], onPage=header_footer)
    doc.addPageTemplates([template])

    story = build_story()

    print(f"   flowables: {len(story):,}")
    doc.build(story)

    sz = os.path.getsize(OUTPUT)
    print(f"[OK] {OUTPUT}  ({sz / 1024 / 1024:.1f} MiB)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
