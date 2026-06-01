#!/usr/bin/env python3
"""
Magazine-grade investor PDF builder (EFE + Meta-Swarm).

This is a full re-design of the earlier corporate-report pipeline.
Output style is closer to a Stripe / Linear / a16z investor brief :
* magazine-style cover with massive display type and a single hero stat,
* full-bleed colour divider pages between major sections,
* real data-viz (bar charts, timeline strips, stack diagrams) instead of
  data tables wherever a chart conveys the same information faster,
* generous white space, single hero per spread,
* KPI tiles with 60–80 pt numerals,
* hand-drawn (programmatic) architecture stack + evidence-chain diagrams.

Run :
    python3 scripts/investor-pdf/build_investor_pdf.py
"""

from __future__ import annotations

import datetime as _dt
import pathlib
import sys
from dataclasses import dataclass

from reportlab.graphics.charts.barcharts import HorizontalBarChart, VerticalBarChart
from reportlab.graphics.shapes import (
    Circle, Drawing, Group, Line, Polygon, Rect, String,
)
from reportlab.lib import colors
from reportlab.lib.enums import TA_CENTER, TA_JUSTIFY, TA_LEFT, TA_RIGHT
from reportlab.lib.pagesizes import A4
from reportlab.lib.styles import ParagraphStyle, getSampleStyleSheet
from reportlab.pdfgen import canvas
from reportlab.platypus import (
    BaseDocTemplate, Flowable, Frame, NextPageTemplate, PageBreak,
    PageTemplate, Paragraph, Spacer, Table, TableStyle,
)
from reportlab.platypus.flowables import HRFlowable

# ── Design tokens ──────────────────────────────────────────────────────────
PAGE_W, PAGE_H = A4
ML, MR, MT, MB = 56, 56, 56, 56

# Brand palette — modern, restrained, two-tone with bright accent.
INK         = colors.HexColor("#0B0F14")  # near-black for body
INK_2       = colors.HexColor("#2C3744")  # secondary text
MUTED       = colors.HexColor("#6B7785")  # captions
RULE        = colors.HexColor("#D9DDE3")
TINT        = colors.HexColor("#F4F6F8")
TINT_DEEP   = colors.HexColor("#EAEEF2")
PAPER       = colors.HexColor("#FFFFFF")

# Hero brand colour (deep teal / petrol — premium, not over-used).
BRAND       = colors.HexColor("#0F3D40")
BRAND_DK    = colors.HexColor("#06292B")
BRAND_LT    = colors.HexColor("#1E5E62")

# Single bright accent (electric coral — only for callouts / chart bars
# that carry the headline number).
ACCENT      = colors.HexColor("#FF5B49")
ACCENT_DK   = colors.HexColor("#E04835")
SUCCESS     = colors.HexColor("#1F8A56")
WARN        = colors.HexColor("#C29A2A")

# Typography (core 14, no embedded font drama).
F_BODY = "Helvetica"
F_BOLD = "Helvetica-Bold"
F_OBL  = "Helvetica-Oblique"
F_MONO = "Courier"


# ── Paragraph styles ───────────────────────────────────────────────────────
def make_styles() -> dict[str, ParagraphStyle]:
    s = getSampleStyleSheet()["BodyText"]
    S: dict[str, ParagraphStyle] = {}
    S["Body"] = ParagraphStyle("Body", parent=s, fontName=F_BODY,
        fontSize=10, leading=14.5, textColor=INK,
        alignment=TA_LEFT, spaceBefore=0, spaceAfter=6)
    S["BodyJ"] = ParagraphStyle("BodyJ", parent=S["Body"], alignment=TA_JUSTIFY)
    S["Lead"] = ParagraphStyle("Lead", parent=s, fontName=F_BODY,
        fontSize=13, leading=19, textColor=INK_2,
        alignment=TA_LEFT, spaceBefore=4, spaceAfter=14)
    S["H1"] = ParagraphStyle("H1", parent=s, fontName=F_BOLD,
        fontSize=26, leading=30, textColor=INK,
        alignment=TA_LEFT, spaceBefore=0, spaceAfter=10)
    S["H2"] = ParagraphStyle("H2", parent=s, fontName=F_BOLD,
        fontSize=14, leading=18, textColor=INK,
        alignment=TA_LEFT, spaceBefore=18, spaceAfter=6)
    S["H3"] = ParagraphStyle("H3", parent=s, fontName=F_BOLD,
        fontSize=10.5, leading=14, textColor=BRAND,
        alignment=TA_LEFT, spaceBefore=10, spaceAfter=2)
    S["Eyebrow"] = ParagraphStyle("Eyebrow", parent=s, fontName=F_BOLD,
        fontSize=8.4, leading=10, textColor=ACCENT,
        alignment=TA_LEFT, spaceBefore=0, spaceAfter=4)
    S["Caption"] = ParagraphStyle("Caption", parent=s, fontName=F_OBL,
        fontSize=8.5, leading=12, textColor=MUTED,
        alignment=TA_LEFT, spaceBefore=0, spaceAfter=6)
    S["Pull"] = ParagraphStyle("Pull", parent=s, fontName=F_BOLD,
        fontSize=18, leading=26, textColor=INK,
        alignment=TA_LEFT, leftIndent=0, rightIndent=0,
        spaceBefore=14, spaceAfter=14)
    S["Bullet"] = ParagraphStyle("Bullet", parent=S["Body"], leftIndent=14,
        bulletIndent=0, spaceBefore=2, spaceAfter=2)
    S["TCellH"] = ParagraphStyle("TCellH", parent=s, fontName=F_BOLD,
        fontSize=8.6, leading=11, textColor=PAPER, alignment=TA_LEFT)
    S["TCell"] = ParagraphStyle("TCell", parent=s, fontName=F_BODY,
        fontSize=9, leading=12, textColor=INK, alignment=TA_LEFT)
    S["TCellNum"] = ParagraphStyle("TCellNum", parent=S["TCell"],
        fontName=F_MONO, alignment=TA_RIGHT)
    S["TCellCode"] = ParagraphStyle("TCellCode", parent=S["TCell"],
        fontName=F_MONO, fontSize=8.5)
    S["Code"] = ParagraphStyle("Code", parent=s, fontName=F_MONO,
        fontSize=8.6, leading=12.5, textColor=INK,
        alignment=TA_LEFT, spaceBefore=4, spaceAfter=8)
    S["KPIVal"] = ParagraphStyle("KPIVal", parent=s, fontName=F_BOLD,
        fontSize=44, leading=46, textColor=INK,
        alignment=TA_LEFT, spaceBefore=0, spaceAfter=0)
    S["KPILabel"] = ParagraphStyle("KPILabel", parent=s, fontName=F_BOLD,
        fontSize=8.5, leading=11, textColor=ACCENT,
        alignment=TA_LEFT, spaceBefore=6, spaceAfter=2)
    S["KPIHint"] = ParagraphStyle("KPIHint", parent=s, fontName=F_BODY,
        fontSize=8.4, leading=11.5, textColor=MUTED,
        alignment=TA_LEFT, spaceBefore=0, spaceAfter=0)
    S["DividerEye"] = ParagraphStyle("DividerEye", parent=s, fontName=F_BOLD,
        fontSize=12, leading=14, textColor=ACCENT,
        alignment=TA_LEFT, spaceBefore=0, spaceAfter=8)
    S["DividerTitle"] = ParagraphStyle("DividerTitle", parent=s, fontName=F_BOLD,
        fontSize=52, leading=56, textColor=PAPER,
        alignment=TA_LEFT, spaceBefore=0, spaceAfter=0)
    S["DividerSub"] = ParagraphStyle("DividerSub", parent=s, fontName=F_BODY,
        fontSize=14, leading=20, textColor=PAPER,
        alignment=TA_LEFT, spaceBefore=14, spaceAfter=0)
    return S

STYLES = make_styles()


# ── Custom flowables ──────────────────────────────────────────────────────
class FullBleedDivider(Flowable):
    """Full-bleed brand-coloured divider page.

    Used between major sections. Renders a single page with the section
    number, title, and a one-line teaser, all on a BRAND background.
    """

    def __init__(self, number: str, title: str, sub: str = "",
                 bg=BRAND, fg=PAPER, accent=ACCENT):
        super().__init__()
        self.number = number
        self.title = title
        self.sub = sub
        self.bg = bg
        self.fg = fg
        self.accent = accent
        self.width = PAGE_W
        self.height = PAGE_H

    def wrap(self, avail_w, avail_h):
        # Claim the actual frame content size — but draw outside it via
        # negative coordinates (the canvas is positioned at the frame's
        # bottom-left, so (-ML, -MB) lands at the page bottom-left).
        return avail_w, avail_h

    def draw(self):
        c = self.canv
        c.saveState()
        # Translate to page origin (we were positioned by the frame).
        # Easier : redraw the whole page background directly.
        c.setFillColor(self.bg)
        c.rect(-ML, -MB, PAGE_W, PAGE_H, stroke=0, fill=1)

        # Top-left : eyebrow + section number small.
        c.setFillColor(self.accent)
        c.setFont(F_BOLD, 11)
        c.drawString(0, PAGE_H - MT - 26, f"SECTION  ·  {self.number}")

        # Centre block : huge title, then subtitle below.  Number is the
        # eyebrow's job ; we no longer overlay it with the title.
        # Title sits ~38 % down the page so there's breathing room above.
        title_top_y = PAGE_H - MT - 260
        c.setFillColor(self.fg)

        # Auto-fit title : start at 60pt, shrink until the widest line
        # fits within the printable width.  Title can wrap to 3 lines.
        max_w = PAGE_W - ML - MR
        size = 60
        # Wrap on em-dash or smart wrap.
        if " — " in self.title:
            a, b = self.title.split(" — ", 1)
            lines = [a, "— " + b]
        else:
            lines = _wrap_text(self.title, 22)
        while max(c.stringWidth(ln, F_BOLD, size) for ln in lines) > max_w \
                and size > 32:
            size -= 4
        c.setFont(F_BOLD, size)
        y = title_top_y
        for ln in lines:
            c.drawString(0, y, ln)
            y -= size * 1.04

        # Subtitle.
        if self.sub:
            c.setFont(F_BODY, 14)
            y -= 6
            for ln in _wrap_text(self.sub, 70):
                c.drawString(0, y, ln)
                y -= 18

        # Giant ghosted number bottom-right — pure design element, no
        # overlap risk with copy.
        c.setFillColor(colors.HexColor("#0A2628"))  # subtle, near-bg
        c.setFont(F_BOLD, 320)
        ghost_w = c.stringWidth(self.number, F_BOLD, 320)
        c.drawString(PAGE_W - ML - MR - ghost_w + 30, -MB + 30,
                     self.number)

        # Re-paint the accent stripe at the bottom on top of ghost.
        c.setFillColor(self.accent)
        c.rect(-ML, -MB, PAGE_W, 10, stroke=0, fill=1)

        # Tiny meta strip just above the accent rule.
        c.setFillColor(self.fg)
        c.setFont(F_BOLD, 8)
        c.drawString(0, -MB + 26,
                     "EFE  ·  INVESTOR DUE DILIGENCE  ·  CONFIDENTIAL")
        c.setFont(F_BODY, 8)
        c.drawRightString(PAGE_W - ML - MR, -MB + 26,
                          "2026-05-16  ·  post-PR #25  ·  Edition 1")
        c.restoreState()


class BigKPI(Flowable):
    """One large KPI tile : huge numeral + label + hint."""

    def __init__(self, value: str, label: str, hint: str,
                 width: float, height: float = 130, color=INK):
        super().__init__()
        self.value = value
        self.label = label
        self.hint = hint
        self.width = width
        self.height = height
        self.color = color

    def wrap(self, *_args):
        return self.width, self.height

    def draw(self):
        c = self.canv
        c.saveState()
        # Top accent rule.
        c.setFillColor(ACCENT)
        c.rect(0, self.height - 3, self.width, 3, stroke=0, fill=1)
        # Numeral — auto-scale so it fits the tile width.
        max_w = self.width - 4
        size = 52
        while c.stringWidth(self.value, F_BOLD, size) > max_w and size > 22:
            size -= 2
        c.setFillColor(self.color)
        c.setFont(F_BOLD, size)
        c.drawString(0, self.height - 4 - size, self.value)
        # Label.
        c.setFillColor(ACCENT)
        c.setFont(F_BOLD, 9)
        c.drawString(0, self.height - 4 - size - 18, self.label.upper())
        # Hint — wrapped.
        c.setFillColor(MUTED)
        c.setFont(F_BODY, 8.4)
        y = self.height - 4 - size - 32
        for ln in _wrap_text(self.hint, max(int(self.width / 4), 18)):
            c.drawString(0, y, ln)
            y -= 11
        c.restoreState()


class HBarsViz(Flowable):
    """Horizontal-bar comparison chart for headline numbers.

    Renders two stacked horizontal bars (e.g. PR #24 67 defects vs PR #25
    8 defects) with axis labels and a Δ callout.
    """

    def __init__(self, pairs: list[tuple[str, float, str]],
                 width: float, height: float, title: str = "",
                 hilite_last: bool = True):
        super().__init__()
        self.pairs = pairs
        self.width = width
        self.height = height
        self.title = title
        self.hilite_last = hilite_last

    def wrap(self, *_args):
        return self.width, self.height

    def draw(self):
        c = self.canv
        c.saveState()
        n = len(self.pairs)
        label_w = 80
        note_w = 230          # reserved space on the right for the caption
        value_w = 50
        bar_track_w = self.width - label_w - value_w - note_w - 18
        row_h = self.height / (n + 1)
        max_val = max(v for _, v, _ in self.pairs)

        # Title.
        if self.title:
            c.setFillColor(MUTED)
            c.setFont(F_BOLD, 9)
            c.drawString(0, self.height - 14, self.title.upper())

        y = self.height - 38
        for i, (label, value, note) in enumerate(self.pairs):
            is_last = (i == n - 1)
            color = ACCENT if (is_last and self.hilite_last) else BRAND_LT
            # Row label.
            c.setFillColor(INK)
            c.setFont(F_BOLD, 10)
            c.drawString(0, y - 4, label)
            # Bar track (very light) + bar.
            bx0 = label_w + 6
            c.setFillColor(TINT_DEEP)
            c.rect(bx0, y - 14, bar_track_w, 18, stroke=0, fill=1)
            bar_len = (value / max_val) * bar_track_w
            c.setFillColor(color)
            c.rect(bx0, y - 14, bar_len, 18, stroke=0, fill=1)
            # Numeric value ALWAYS to the right of the bar, never inside.
            c.setFillColor(INK)
            c.setFont(F_BOLD, 11)
            c.drawString(bx0 + bar_track_w + 8, y - 9, f"{value:g}")
            # Right-side note in its own column — wraps to 2 lines if long.
            c.setFillColor(MUTED)
            c.setFont(F_BODY, 8.4)
            note_x = self.width - note_w
            note_lines = _wrap_text(note, max(int(note_w / 4.6), 28))[:2]
            ny = y - 4
            for ln in note_lines:
                c.drawString(note_x, ny, ln)
                ny -= 10
            y -= row_h * 0.85
        c.restoreState()


class TimelineStrip(Flowable):
    """Horizontal milestone strip (e.g. PRs #21-#25 shipped)."""

    def __init__(self, milestones: list[tuple[str, str, str]],
                 width: float, height: float = 90, title: str = ""):
        super().__init__()
        self.milestones = milestones
        self.width = width
        self.height = height
        self.title = title

    def wrap(self, *_args):
        return self.width, self.height

    def draw(self):
        c = self.canv
        c.saveState()
        n = len(self.milestones)
        if self.title:
            c.setFillColor(MUTED)
            c.setFont(F_BOLD, 9)
            c.drawString(0, self.height - 14, self.title.upper())
        y0 = self.height * 0.46
        x_start = 8
        x_end = self.width - 8
        # Spine.
        c.setStrokeColor(RULE)
        c.setLineWidth(1.6)
        c.line(x_start, y0, x_end, y0)
        # Markers.
        step = (x_end - x_start) / max(n - 1, 1)
        for i, (head, sub, foot) in enumerate(self.milestones):
            cx = x_start + i * step
            is_last = (i == n - 1)
            # Circle marker.
            c.setFillColor(ACCENT if is_last else BRAND)
            c.setStrokeColor(ACCENT if is_last else BRAND)
            c.circle(cx, y0, 6, stroke=0, fill=1)
            # Inner white pip for the highlighted one.
            if is_last:
                c.setFillColor(PAPER)
                c.circle(cx, y0, 2.5, stroke=0, fill=1)
            # Head above.
            c.setFillColor(INK)
            c.setFont(F_BOLD, 10)
            c.drawCentredString(cx, y0 + 16, head)
            c.setFillColor(MUTED)
            c.setFont(F_BODY, 7.6)
            c.drawCentredString(cx, y0 + 28, sub)
            # Foot below.
            c.setFillColor(ACCENT if is_last else INK_2)
            c.setFont(F_BOLD, 9)
            c.drawCentredString(cx, y0 - 18, foot)
        c.restoreState()


class StackDiagram(Flowable):
    """Layered architecture stack — coloured blocks with labels."""

    def __init__(self, layers: list[tuple[str, str, str]],
                 width: float, height: float, title: str = ""):
        # Each layer is (label, hint, palette_key in {brand, brand_lt, accent, tint}).
        super().__init__()
        self.layers = layers
        self.width = width
        self.height = height
        self.title = title

    def wrap(self, *_args):
        return self.width, self.height

    def draw(self):
        c = self.canv
        c.saveState()
        if self.title:
            c.setFillColor(MUTED)
            c.setFont(F_BOLD, 9)
            c.drawString(0, self.height - 14, self.title.upper())
        top = self.height - 28
        bottom = 4
        n = len(self.layers)
        gap = 6
        usable = top - bottom - gap * (n - 1)
        h = usable / n
        palette = {
            "brand": (BRAND, PAPER),
            "brand_lt": (BRAND_LT, PAPER),
            "accent": (ACCENT, PAPER),
            "tint": (TINT_DEEP, INK),
            "ink": (INK, PAPER),
        }
        for i, (label, hint, key) in enumerate(self.layers):
            fill, text = palette.get(key, palette["tint"])
            y = top - (i + 1) * h - i * gap
            c.setFillColor(fill)
            c.rect(0, y, self.width, h, stroke=0, fill=1)
            c.setFillColor(text)
            c.setFont(F_BOLD, 10.5)
            c.drawString(14, y + h - 16, label)
            c.setFont(F_BODY, 8.4)
            c.drawString(14, y + 8, hint)
        c.restoreState()


class FlowDiagram(Flowable):
    """Right-arrow chain : Source → Step → Step → Sink."""

    def __init__(self, steps: list[tuple[str, str]],
                 width: float, height: float = 90, title: str = ""):
        super().__init__()
        self.steps = steps
        self.width = width
        self.height = height
        self.title = title

    def wrap(self, *_args):
        return self.width, self.height

    def draw(self):
        c = self.canv
        c.saveState()
        if self.title:
            c.setFillColor(MUTED)
            c.setFont(F_BOLD, 9)
            c.drawString(0, self.height - 14, self.title.upper())
        n = len(self.steps)
        gap = 8
        arrow_w = 14
        usable = self.width - gap * (n - 1) - arrow_w * (n - 1)
        box_w = usable / n
        box_h = 56
        y = (self.height - 28 - box_h) / 2 + 8
        x = 0
        for i, (head, sub) in enumerate(self.steps):
            # Coloured box.
            c.setFillColor(BRAND_LT if i < n - 1 else ACCENT)
            c.rect(x, y, box_w, box_h, stroke=0, fill=1)
            c.setFillColor(PAPER)
            c.setFont(F_BOLD, 10)
            # Head text wrapped to 2 lines max.
            head_lines = _wrap_text(head, max(int(box_w / 5.4), 8))[:2]
            for j, ln in enumerate(head_lines):
                c.drawString(x + 8, y + box_h - 18 - j * 12, ln)
            c.setFont(F_BODY, 7.6)
            sub_lines = _wrap_text(sub, max(int(box_w / 4.4), 10))[:2]
            for j, ln in enumerate(sub_lines):
                c.drawString(x + 8, y + 14 - j * 9, ln)
            x += box_w
            if i < n - 1:
                # Arrow.
                ay = y + box_h / 2
                c.setFillColor(BRAND)
                c.setStrokeColor(BRAND)
                # Triangle path.
                tri_x = x + gap / 2
                p = c.beginPath()
                p.moveTo(tri_x, ay + 7)
                p.lineTo(tri_x + arrow_w, ay)
                p.lineTo(tri_x, ay - 7)
                p.close()
                c.drawPath(p, stroke=0, fill=1)
                x += gap + arrow_w
        c.restoreState()


def _wrap_text(text: str, max_chars: int) -> list[str]:
    words = text.split()
    if not words:
        return [""]
    out = []
    cur = ""
    for w in words:
        cand = (cur + " " + w).strip()
        if len(cand) > max_chars and cur:
            out.append(cur)
            cur = w
        else:
            cur = cand
    if cur:
        out.append(cur)
    return out


# ── Helpers : paragraphs, tables, code, callouts ──────────────────────────
def P(t: str, style: str = "Body") -> Paragraph:
    return Paragraph(t, STYLES[style])

def eyebrow(t: str) -> Paragraph:
    return Paragraph(t.upper(), STYLES["Eyebrow"])

def h1(t: str) -> Paragraph:
    return Paragraph(t, STYLES["H1"])

def h2(t: str) -> Paragraph:
    return Paragraph(t, STYLES["H2"])

def h3(t: str) -> Paragraph:
    return Paragraph(t, STYLES["H3"])

def lead(t: str) -> Paragraph:
    return Paragraph(t, STYLES["Lead"])

def caption(t: str) -> Paragraph:
    return Paragraph(t, STYLES["Caption"])

def pull(t: str) -> Paragraph:
    return Paragraph('<font color="#FF5B49">―</font> ' + t, STYLES["Pull"])

def bullets(items: list[str]) -> list[Paragraph]:
    return [Paragraph(x, STYLES["Bullet"], bulletText="•") for x in items]

def code_block(text: str) -> Table:
    body = Paragraph(text.replace("\n", "<br/>"), STYLES["Code"])
    t = Table([[body]], colWidths=[PAGE_W - ML - MR])
    t.setStyle(TableStyle([
        ("BACKGROUND", (0, 0), (-1, -1), TINT),
        ("LEFTPADDING", (0, 0), (-1, -1), 14),
        ("RIGHTPADDING", (0, 0), (-1, -1), 12),
        ("TOPPADDING", (0, 0), (-1, -1), 10),
        ("BOTTOMPADDING", (0, 0), (-1, -1), 10),
        ("LINEBEFORE", (0, 0), (0, -1), 2.5, ACCENT),
    ]))
    return t

def callout(eyebrow_text: str, body_text: str) -> Table:
    inner = [
        Paragraph(eyebrow_text.upper(),
                  ParagraphStyle("CalloutEye", parent=STYLES["Eyebrow"],
                                 textColor=PAPER, spaceBefore=0, spaceAfter=6)),
        Paragraph(body_text,
                  ParagraphStyle("CalloutBody", parent=STYLES["Body"],
                                 fontName=F_BOLD, fontSize=11.5, leading=16,
                                 textColor=PAPER, alignment=TA_LEFT)),
    ]
    t = Table([[inner]], colWidths=[PAGE_W - ML - MR])
    t.setStyle(TableStyle([
        ("BACKGROUND", (0, 0), (-1, -1), BRAND),
        ("LEFTPADDING", (0, 0), (-1, -1), 18),
        ("RIGHTPADDING", (0, 0), (-1, -1), 18),
        ("TOPPADDING", (0, 0), (-1, -1), 14),
        ("BOTTOMPADDING", (0, 0), (-1, -1), 14),
    ]))
    return t

def kpi_row(tiles: list[tuple[str, str, str]]) -> Table:
    total_w = PAGE_W - ML - MR
    tile_w = total_w / len(tiles)
    cells = [BigKPI(v, l, h, tile_w - 8) for v, l, h in tiles]
    t = Table([cells], colWidths=[tile_w] * len(tiles))
    t.setStyle(TableStyle([
        ("VALIGN", (0, 0), (-1, -1), "TOP"),
        ("LEFTPADDING", (0, 0), (-1, -1), 0),
        ("RIGHTPADDING", (0, 0), (-1, -1), 8),
        ("TOPPADDING", (0, 0), (-1, -1), 0),
        ("BOTTOMPADDING", (0, 0), (-1, -1), 0),
    ]))
    return t

def data_table(headers: list[str], rows: list[list[str]],
               col_widths: list[float] | None = None,
               numeric_cols: tuple[int, ...] = (),
               code_cols: tuple[int, ...] = (),
               caption_text: str = "") -> list:
    total_w = PAGE_W - ML - MR
    if col_widths is None:
        col_widths = [total_w / len(headers)] * len(headers)
    else:
        s = sum(col_widths)
        col_widths = [w * total_w / s for w in col_widths]
    head = [Paragraph(h, STYLES["TCellH"]) for h in headers]
    data = [head]
    for r in rows:
        row = []
        for i, v in enumerate(r):
            if i in numeric_cols:
                row.append(Paragraph(str(v), STYLES["TCellNum"]))
            elif i in code_cols:
                row.append(Paragraph(str(v), STYLES["TCellCode"]))
            else:
                row.append(Paragraph(str(v), STYLES["TCell"]))
        data.append(row)
    t = Table(data, colWidths=col_widths, repeatRows=1)
    style = [
        ("BACKGROUND", (0, 0), (-1, 0), BRAND),
        ("TOPPADDING", (0, 0), (-1, -1), 6),
        ("BOTTOMPADDING", (0, 0), (-1, -1), 6),
        ("LEFTPADDING", (0, 0), (-1, -1), 8),
        ("RIGHTPADDING", (0, 0), (-1, -1), 8),
        ("VALIGN", (0, 0), (-1, -1), "TOP"),
        ("LINEBELOW", (0, 0), (-1, 0), 0.5, BRAND_DK),
        ("LINEBELOW", (0, -1), (-1, -1), 0.5, RULE),
    ]
    for i in range(1, len(data)):
        if i % 2 == 0:
            style.append(("BACKGROUND", (0, i), (-1, i), TINT))
    t.setStyle(TableStyle(style))
    out: list = [t]
    if caption_text:
        out.append(caption(caption_text))
    out.append(Spacer(1, 4))
    return out


# ── Doc template : magazine pages ─────────────────────────────────────────
@dataclass
class DocMeta:
    title: str
    subtitle: str
    short_title: str
    edition: str
    audience: str
    author: str
    hero_kpi_value: str
    hero_kpi_label: str
    confidentiality: str = "CONFIDENTIAL · INVESTOR DUE DILIGENCE"


class MagDocTemplate(BaseDocTemplate):
    def __init__(self, filename: str, meta: DocMeta):
        self.meta = meta
        super().__init__(filename, pagesize=A4,
            leftMargin=ML, rightMargin=MR,
            topMargin=MT, bottomMargin=MB,
            title=meta.title, author=meta.author,
            subject=meta.subtitle,
            keywords="investor; due-diligence; EFE; PR #25; STABILITY v1.7",
            creator="EFE — investor-pdf builder (ReportLab, magazine pipeline)")
        cover = Frame(0, 0, PAGE_W, PAGE_H, 0, 0, 0, 0, id="cover")
        body = Frame(ML, MB, PAGE_W - ML - MR, PAGE_H - MT - MB,
                     0, 0, 0, 0, id="body")
        # Full-bleed frame for section dividers : drawer paints whole page.
        bleed = Frame(ML, MB, PAGE_W - ML - MR, PAGE_H - MT - MB,
                      0, 0, 0, 0, id="bleed")
        self.addPageTemplates([
            PageTemplate(id="cover", frames=[cover], onPage=self._draw_cover),
            PageTemplate(id="body", frames=[body], onPage=self._draw_body),
            PageTemplate(id="bleed", frames=[bleed], onPage=lambda *_a: None),
        ])

    def _draw_cover(self, c: canvas.Canvas, _doc: BaseDocTemplate) -> None:
        # Full-bleed brand colour background.
        c.setFillColor(BRAND_DK)
        c.rect(0, 0, PAGE_W, PAGE_H, stroke=0, fill=1)

        # Bottom 30% : white panel.
        c.setFillColor(PAPER)
        c.rect(0, 0, PAGE_W, PAGE_H * 0.34, stroke=0, fill=1)

        # Accent diagonal band (subtle visual element).
        c.setFillColor(ACCENT)
        c.rect(0, PAGE_H * 0.34, PAGE_W, 6, stroke=0, fill=1)

        # Top-left wordmark — derived from the meta's short_title.
        # The first word becomes the bold mark; the rest becomes the tag.
        c.setFillColor(PAPER)
        c.setFont(F_BOLD, 11)
        wordmark_full = self.meta.short_title or ""
        if " " in wordmark_full:
            mark, tag = wordmark_full.split(" ", 1)
        else:
            mark, tag = wordmark_full, ""
        c.drawString(ML, PAGE_H - MT - 8, mark.upper())
        mark_w = c.stringWidth(mark.upper(), F_BOLD, 11)
        c.setFont(F_BODY, 9.5)
        if tag:
            c.drawString(ML + mark_w + 8, PAGE_H - MT - 8,
                         "·  " + tag.upper())
        # Top-right edition.
        c.setFont(F_BODY, 9.5)
        c.drawRightString(PAGE_W - MR, PAGE_H - MT - 8,
                          self.meta.edition.upper())

        # Hero KPI on the dark band : huge numeral.
        # Placed lower-left of the top dark area, label stacked below.
        hero_y = PAGE_H * 0.34 + 70
        c.setFillColor(ACCENT)
        c.setFont(F_BOLD, 11)
        c.drawString(ML, hero_y + 132, "HEADLINE METRIC")
        c.setFillColor(PAPER)
        # Auto-fit the hero value (e.g. "8.4×").
        hero_size = 160
        while c.stringWidth(self.meta.hero_kpi_value, F_BOLD, hero_size) > \
                PAGE_W * 0.50 and hero_size > 80:
            hero_size -= 6
        c.setFont(F_BOLD, hero_size)
        c.drawString(ML - 6, hero_y, self.meta.hero_kpi_value)
        # Label — wrap aggressively so it never bleeds off the right edge.
        c.setFillColor(PAPER)
        c.setFont(F_BODY, 12)
        label_x = ML + c.stringWidth(self.meta.hero_kpi_value, F_BOLD,
                                     hero_size) + 18
        # Stay within the page : compute max chars-per-line from remaining width.
        avail = (PAGE_W - MR) - label_x
        # Approximate char width at 12pt Helvetica = 6.2pt.
        chars_per_line = max(int(avail / 6.4), 18)
        lines = _wrap_text(self.meta.hero_kpi_label, chars_per_line)[:4]
        for i, ln in enumerate(lines):
            c.drawString(label_x, hero_y + 28 - i * 18, ln)

        # Bottom panel : title + meta.
        # Title — auto-fit to width.
        title = self.meta.title
        title_max_w = PAGE_W - ML - MR
        size = 38
        # Wrap on em-dash if present.
        if " — " in title:
            t1, t2 = title.split(" — ", 1)
            lines = [t1, "— " + t2]
        else:
            lines = _wrap_text(title, 32)
        while max(c.stringWidth(ln, F_BOLD, size) for ln in lines) > title_max_w \
                and size > 22:
            size -= 2
        c.setFillColor(INK)
        c.setFont(F_BOLD, size)
        title_y = PAGE_H * 0.30
        for ln in lines:
            c.drawString(ML, title_y, ln)
            title_y -= size * 1.06
        # Subtitle.
        c.setFillColor(INK_2)
        c.setFont(F_BODY, 12)
        c.drawString(ML, title_y - 4, self.meta.subtitle)

        # Bottom meta strip.
        meta_y = MB + 12
        c.setStrokeColor(RULE)
        c.setLineWidth(0.5)
        c.line(ML, meta_y + 38, PAGE_W - MR, meta_y + 38)
        c.setFillColor(MUTED)
        c.setFont(F_BOLD, 8)
        c.drawString(ML, meta_y + 22, "ISSUED")
        c.drawString(ML + 130, meta_y + 22, "AUTHOR")
        c.drawString(ML + 320, meta_y + 22, "AUDIENCE")
        c.setFillColor(INK)
        c.setFont(F_BODY, 9)
        c.drawString(ML, meta_y + 8, _today_iso())
        c.drawString(ML + 130, meta_y + 8, self.meta.author)
        c.drawString(ML + 320, meta_y + 8,
                     _wrap_text(self.meta.audience, 40)[0])
        # Classification footer.
        c.setFillColor(MUTED)
        c.setFont(F_BODY, 7.6)
        c.drawString(ML, meta_y - 6, self.meta.confidentiality
                     + " · Not for redistribution.")
        c.drawRightString(PAGE_W - MR, meta_y - 6,
                          f"Built {_today_iso()}")

    def _draw_body(self, c: canvas.Canvas, doc: BaseDocTemplate) -> None:
        # Subtle running header.
        c.setFillColor(MUTED)
        c.setFont(F_BOLD, 8)
        c.drawString(ML, PAGE_H - 32, self.meta.short_title.upper())
        c.setFont(F_BODY, 8)
        c.drawRightString(PAGE_W - MR, PAGE_H - 32, self.meta.edition)
        c.setStrokeColor(RULE)
        c.setLineWidth(0.4)
        c.line(ML, PAGE_H - 38, PAGE_W - MR, PAGE_H - 38)
        # Footer.
        c.line(ML, MB - 18, PAGE_W - MR, MB - 18)
        c.setFillColor(MUTED)
        c.setFont(F_BODY, 7.6)
        c.drawString(ML, MB - 30, self.meta.confidentiality)
        c.drawRightString(PAGE_W - MR, MB - 30,
                          f"PAGE {c.getPageNumber()}")


def _today_iso() -> str:
    return _dt.date(2026, 5, 17).isoformat()




# ── KOVANT story (prose edition) ──────────────────────────────────────────
def build_kovant_story() -> list:
    s: list = []
    s.append(Spacer(1, 1))

    # ── Opening — letter to the reader ───────────────────────────────────
    s.append(NextPageTemplate("body"))
    s.append(PageBreak())

    s.append(eyebrow("A letter to the reader"))
    s.append(h1("Before you begin."))
    s.append(P(
        "The document you are about to read is an unusually "
        "long brief, written for an unusually patient reader, "
        "about a company whose argument cannot be made well in "
        "five pages and which we therefore have not tried to "
        "make in five pages. The company is named Kovant. It "
        "was founded approximately eighteen months ago by a "
        "small group of engineers and operators who had each "
        "spent the previous decade of their careers inside the "
        "largest technology firms in the world, and it has, "
        "over the eighteen months of its existence, moved from "
        "a sketch on a whiteboard to a globally available "
        "platform with a small but rapidly growing number of "
        "industrial customers and a first round of professional "
        "investment that closed in the late autumn of last year."
    ))
    s.append(P(
        "We have written this document because the questions "
        "an investor or partner naturally asks about a company "
        "at this stage of its life cannot honestly be answered "
        "in the form of a pitch deck. A pitch deck can tell you "
        "what we sell, who we sell it to, and how much money we "
        "have raised. It cannot tell you why we believe the "
        "specific problem we have chosen to solve is the right "
        "problem to choose, why we believe the specific shape "
        "of our solution is the right shape, why we believe the "
        "specific customers we have chosen to begin with are "
        "the right customers to begin with, or what we will do "
        "differently from the many, many other companies that "
        "have set out to do superficially similar things over "
        "the last few years. Those questions deserve real "
        "answers, and the answers take pages, not slides. This "
        "brief is the answer."
    ))
    s.append(P(
        "We have also chosen to write the brief in ordinary "
        "language. There is a vocabulary inside our industry "
        "that is dense, often inflated, and frequently used by "
        "companies that do not entirely understand what their "
        "own words mean. Whenever you read that a company "
        "offers an agentic orchestration layer for "
        "enterprise-grade autonomous operations, you are "
        "looking at words that mean something specific in some "
        "narrow technical context and almost nothing once you "
        "step outside that context. We have tried, throughout "
        "this brief, to avoid that kind of vocabulary, and to "
        "say what we actually do in ordinary words that any "
        "literate reader can follow. Where a technical concept "
        "genuinely is the right tool for explaining something, "
        "we introduce it slowly and define it the first time "
        "it appears, and we use it consistently from then on. "
        "Where the technical vocabulary is being used as a "
        "substitute for thought, we have not used it at all."
    ))
    s.append(P(
        "The result is a long document. We have done our best "
        "to make it readable rather than merely complete, but "
        "we will admit that some pages are denser than others "
        "and that a reader who attempts to absorb the whole "
        "brief in a single sitting will find their attention "
        "flagging somewhere around the midpoint. We have "
        "therefore structured the brief into eleven parts, "
        "each of which can be read independently of the others "
        "once you have absorbed the opening, and we have "
        "described in the third section below how a reader "
        "with a particular question in mind should choose "
        "which parts to read first."
    ))

    s.append(PageBreak())
    s.append(eyebrow("What this brief covers"))
    s.append(h1("Eleven parts, one argument."))
    s.append(P(
        "The argument we are making across the eleven parts of "
        "this brief can be stated in a single sentence, and we "
        "state it now so that the rest of the brief can be "
        "read as the elaboration of the sentence. The argument "
        "is that the largest opportunity in software at this "
        "moment in history is the opportunity to take the "
        "work that is currently done by office employees "
        "inside large industrial companies and to redistribute "
        "it between humans and software in a fundamentally "
        "different way than the way it has been redistributed "
        "for the last forty years, and that our company is "
        "among the very few teams in the world both "
        "technically capable of building the software that "
        "makes the redistribution possible and commercially "
        "credible enough to sell that software into the "
        "industrial enterprises where the impact will be the "
        "largest."
    ))
    s.append(P(
        "We acknowledge, before going any further, that this "
        "kind of claim has been made by a great many "
        "companies in the last three years, and that a reader "
        "who has been paying any attention at all to the "
        "investor landscape has heard a version of it from "
        "almost every artificial-intelligence start-up they "
        "have encountered. We are not asking you to believe "
        "us simply because we are saying it. We are asking "
        "you to read the brief and decide for yourself "
        "whether we have understood the problem more deeply "
        "than other teams making similar claims, whether the "
        "shape of our solution differs from theirs in ways "
        "that genuinely matter, and whether the early traction "
        "we have managed to produce gives you reason to "
        "believe we will continue to execute in a way that "
        "delivers on the claim."
    ))
    s.append(P(
        "The first part of the brief is the opening you are "
        "now reading, which explains what the document is, "
        "what it covers, and how to read it. The second part "
        "describes the company in ordinary language. It "
        "explains what we do, what problem we solve for our "
        "customers, why we believe the problem matters, and "
        "what specifically distinguishes us from the dozens "
        "of other companies that have made claims about "
        "solving versions of similar problems. The second "
        "part is the part most likely to be useful to a "
        "reader who is unfamiliar with our work and wants the "
        "context that any subsequent reading depends on."
    ))
    s.append(P(
        "The third part recounts the history of the company "
        "from its founding to the moment you are reading this. "
        "We tell the story in narrative form because the story "
        "is short enough that the narrative is the right "
        "shape, and because the choices we made along the way "
        "are easier to explain in the context of the moments "
        "when we made them than they are when extracted from "
        "their context. The story includes the founding, the "
        "long stealth year, the first customers, the funding "
        "round, and the global launch."
    ))
    s.append(P(
        "The fourth part is the most technical part of the "
        "brief, although we have tried to keep it accessible "
        "to a reader who has no background in the underlying "
        "technology. It describes the platform we have built, "
        "in concepts rather than in implementation detail. It "
        "explains what we mean when we say that we offer an "
        "operating system for an artificial workforce, what "
        "the components of the platform are, what each "
        "component does, and how they fit together to produce "
        "the experience our customers ultimately use."
    ))
    s.append(P(
        "The fifth part describes, in operational rather than "
        "architectural terms, how a customer actually uses "
        "the platform. It walks through the design of an "
        "operation, the deployment of the team of software "
        "workers that will run the operation, the day-to-day "
        "supervision of the team, the handling of exceptions "
        "and edge cases, and the audit trail that the entire "
        "experience produces as a byproduct. The fifth part "
        "is the part most likely to be useful to a reader "
        "who wants to understand what we are like to work "
        "with rather than what we are like to build."
    ))
    s.append(P(
        "The sixth part addresses the question of why this "
        "moment, and not some earlier or later moment, is the "
        "right time to be building what we are building. It "
        "is a careful account of the broader technology "
        "shifts that have made our approach possible and of "
        "the broader market conditions that have made our "
        "approach urgent. A reader who wants to understand "
        "the macro context in which the company exists "
        "should focus on the sixth part."
    ))
    s.append(P(
        "The seventh part is the economic argument. We "
        "describe how customers extract value from the "
        "platform, what kinds of cost the platform displaces, "
        "what the unit economics of a deployment look like, "
        "and what we believe the long-term economic structure "
        "of a customer relationship will look like once "
        "deployments are routine. The seventh part is the "
        "part most likely to be useful to a reader who is "
        "underwriting the financial case for the company."
    ))
    s.append(P(
        "The eighth part addresses the question of trust. "
        "Industrial customers do not adopt new operational "
        "software lightly, and they do so even less lightly "
        "when the software is going to make autonomous "
        "decisions on their behalf. We describe the "
        "deliberate choices we have made about where the "
        "software runs, what data it sees, who can see what "
        "it does, and what auditors and regulators can expect "
        "from the experience. The eighth part is the part "
        "most likely to be useful to a reader who is "
        "concerned about risk, regulation, or compliance."
    ))
    s.append(P(
        "The ninth part describes where we are operationally "
        "and where we intend to go over the next twelve to "
        "eighteen months. It is the forward-looking section "
        "of the brief. We name the geographies we serve "
        "today, the geographies we will enter next, the "
        "industries we have proven the platform in, the "
        "industries we will extend to, and the team we will "
        "use to do all of this."
    ))
    s.append(P(
        "The tenth part is about the people. We describe the "
        "founding team, their backgrounds, the investors who "
        "have backed us so far, the reasons we believe this "
        "group of people is the right group to build this "
        "specific company, and the ways in which we intend to "
        "grow the team over the next year. The tenth part is "
        "the most personal part of the brief, and it is "
        "intentionally placed near the end because the "
        "decision to back a company is partly a decision "
        "about the people building it and partly a decision "
        "about the work itself, and we wanted you to have "
        "absorbed the work first."
    ))
    s.append(P(
        "The eleventh and final part is a short closing that "
        "restates the argument in compressed form, offers a "
        "glossary of the most important terms we have used, "
        "and explains how you can get in touch with us if you "
        "want to continue the conversation."
    ))

    s.append(PageBreak())
    s.append(eyebrow("How to read this brief"))
    s.append(h1("Four reading paths, depending on what you most want to know."))
    s.append(P(
        "We have written the brief to be useful both to the "
        "reader who has time to read it from beginning to end "
        "and to the reader who has only a focused hour and "
        "wants to spend that hour on the parts most relevant "
        "to their particular question. For the second kind of "
        "reader, we describe in this section four reading "
        "paths that we have found correspond to the four most "
        "common kinds of question we encounter from investors, "
        "partners, and customers."
    ))
    s.append(P(
        "If your dominant question is whether the underlying "
        "technology is real and whether the team is "
        "technically credible, the path we recommend is to "
        "begin with this opening and then move directly to "
        "the fourth and fifth parts of the brief. The fourth "
        "part describes what the platform is in conceptual "
        "terms, and the fifth describes how the platform is "
        "used in operational terms. Read together, those two "
        "parts will give you a precise picture of what we "
        "have built and what working with it is like. The "
        "tenth part, which describes the team, will then "
        "give you the human side of the same answer. Roughly "
        "forty minutes of focused reading on those three "
        "parts will leave you with a defensible view on "
        "whether the company is technically what it claims "
        "to be."
    ))
    s.append(P(
        "If your dominant question is whether the market "
        "opportunity is real and whether the company has "
        "found a credible position within it, the path we "
        "recommend begins with this opening, continues with "
        "the second part for the company description, and "
        "then proceeds to the sixth part for the macro "
        "context and the seventh for the economic argument. "
        "Those four parts, read in that order, take about "
        "forty-five minutes and will give you a defensible "
        "view on whether we have understood the market we "
        "are entering and whether the position we have "
        "carved out within it is defensible against the "
        "considerable number of other companies pursuing "
        "adjacent positions."
    ))
    s.append(P(
        "If your dominant question is whether the company "
        "can scale through the next stages of its life and "
        "whether the team has the operational discipline to "
        "execute the plan, the path we recommend is this "
        "opening, then the third part for the history, then "
        "the ninth part for the forward plan, then the "
        "tenth part for the team, and finally the eighth "
        "part for the trust posture that determines how "
        "quickly we can enter regulated industries. About "
        "fifty minutes of focused reading on those five "
        "parts will give you a defensible view on whether we "
        "are likely to execute the plan we have laid out."
    ))
    s.append(P(
        "If your dominant question is the question of risk, "
        "the path we recommend is this opening, then the "
        "eighth part for the trust posture, then the relevant "
        "passages of the second part for the customer context, "
        "and then a re-reading of the company history in the "
        "third part with an eye specifically for the moments "
        "when something could have gone wrong but did not. "
        "About forty minutes of focused reading on those "
        "passages will give you a defensible view on whether "
        "we have understood the kinds of risk the company "
        "faces and whether we are managing them with "
        "appropriate seriousness."
    ))
    s.append(P(
        "Regardless of which path you take, the closing "
        "section contains a glossary of the most important "
        "terms used throughout the brief, a compressed "
        "restatement of the central argument, and contact "
        "information for the leadership team. We encourage "
        "you to glance at the glossary before you begin and "
        "to return to it as needed."
    ))
    s.append(callout("ONE EXPLICIT PROMISE",
        "Every factual claim in this brief is one we are "
        "prepared to defend in conversation, with supporting "
        "evidence drawn from our customer engagements, our "
        "internal records, or the publicly available "
        "documentation of our partners and investors. If you "
        "find a claim in this brief that you cannot reconcile "
        "with information you have from another source, we "
        "would consider it a personal favour to be told, so "
        "that we can either correct the brief or explain the "
        "reconciliation in a subsequent conversation."))

    # ── Part Two — The company in ordinary words ─────────────────────────
    s.append(NextPageTemplate("bleed"))
    s.append(PageBreak())
    s.append(FullBleedDivider("02", "The company in ordinary words",
        "What we do, who we do it for, and what specifically "
        "distinguishes us from the many other companies whose "
        "marketing language sounds superficially similar to ours."))

    s.append(NextPageTemplate("body"))
    s.append(PageBreak())
    s.append(eyebrow("Part Two · What we do"))
    s.append(h1("The company, described in a single paragraph."))
    s.append(P(
        "Our company builds software that performs, on behalf "
        "of large industrial enterprises, the kinds of office "
        "work that have traditionally been done by employees "
        "sitting at desks. The work in question is not the "
        "creative or strategic work that companies value most "
        "highly. It is the repetitive, procedural, "
        "exception-laden work that fills the working day of a "
        "vast number of office employees inside any "
        "manufacturing, distribution, or services firm of "
        "meaningful scale: the work of placing orders with "
        "suppliers, of chasing missing parts through a "
        "logistics chain, of reconciling invoices against "
        "purchase orders, of preparing the quarterly "
        "compliance returns that the regulators demand, of "
        "answering customer enquiries that have already been "
        "answered a thousand times in slightly different "
        "forms. The kind of work that nobody in the office "
        "particularly enjoys but that needs to be done every "
        "day for the company to keep functioning."))
    s.append(P(
        "Our claim, which we will defend at considerable "
        "length in the remainder of this brief, is that the "
        "recent generation of artificial-intelligence systems "
        "has reached the point where this kind of work can be "
        "performed by software, reliably enough and at a low "
        "enough cost that it is no longer economically "
        "rational for a large enterprise to staff entire "
        "departments to do it. The work still needs to be "
        "done, and it still needs to be done with care, but "
        "it does not need to be done by humans. The job of "
        "our company is to provide the software that does "
        "the work, the supervisory layer that ensures the "
        "work is being done correctly, and the audit trail "
        "that satisfies the compliance obligations the work "
        "entails."))
    s.append(P(
        "We have given this kind of software a specific name "
        "inside our own organisation, because we have found "
        "that the existing names for it are misleading. We do "
        "not call our software a chatbot, because a chatbot "
        "responds to single questions and then waits for the "
        "next one, and the work we are talking about is not "
        "a sequence of questions and answers but a continuous "
        "operation. We do not call it a copilot, because a "
        "copilot sits next to a human and helps that human do "
        "their job faster, and the work we are talking about "
        "is the work that nobody needs to be done faster, "
        "because nobody wants to be doing it at all. We call "
        "our software, instead, an artificial workforce: a "
        "team of digital workers that takes on entire "
        "operations end to end, runs them continuously, and "
        "asks a human for help only when the work crosses "
        "the boundary of what the workforce is empowered to "
        "handle by itself."))
    s.append(P(
        "The phrase artificial workforce is meant literally. "
        "When a customer uses our platform, they are not "
        "buying a tool that helps their employees work "
        "faster. They are buying additional staff who happen "
        "to be made of software rather than of flesh. The "
        "additional staff have job descriptions, written by "
        "the customer in plain language at the moment they "
        "are first deployed. The additional staff have "
        "managers, who happen to be the customer's existing "
        "human managers and who supervise the digital "
        "workers the same way they would supervise human "
        "ones. The additional staff have performance "
        "reviews, in the form of the supervisory dashboards "
        "the platform provides. They have escalation "
        "procedures, for the cases they cannot handle. They "
        "have an explicit boundary of authority, beyond "
        "which they must seek approval from a human. And "
        "they leave behind, every time they take an action, "
        "a complete audit record of what they did and why, "
        "in a form that an external auditor can review."))

    s.append(PageBreak())
    s.append(eyebrow("Part Two · What problem we are solving"))
    s.append(h1("The problem, as our customers experience it."))
    s.append(P(
        "Before we can describe what makes our company "
        "distinctive, we need to describe carefully the "
        "problem our customers are trying to solve, because "
        "the shape of the problem is not what most people "
        "outside the industrial world assume it is. The "
        "outside assumption is that large industrial "
        "companies want to reduce headcount, and that the "
        "value of software like ours lies in firing "
        "employees and pocketing the savings. This is not "
        "how our customers describe the problem to us, and "
        "it is not how we describe the value of our product "
        "to them."))
    s.append(P(
        "The problem our customers actually face is that the "
        "operational work of running a modern industrial "
        "enterprise has grown faster than their ability to "
        "hire the people who do it. Every year, the "
        "regulators demand more documentation. Every year, "
        "customers demand faster fulfilment and more "
        "frequent updates. Every year, the supply chain "
        "becomes more complex and more exception-prone. The "
        "work has expanded; the available workforce has not. "
        "In some of the geographies we serve, the "
        "working-age population is actively shrinking, and "
        "the competition for the kind of detail-oriented "
        "operations staff that industrial companies most "
        "depend on has become acute. Our customers are not "
        "trying to fire their existing staff. They are "
        "trying to find a way to do the work that there is "
        "nobody to hire to do."))
    s.append(P(
        "The conventional answers to this problem have all "
        "been tried, and have all hit ceilings. The first "
        "conventional answer is to invest in workflow "
        "automation: tools that allow the existing staff to "
        "perform repetitive tasks faster by automating the "
        "mechanical parts. This approach works, up to a "
        "point, but it requires every workflow to be "
        "scripted in advance, and the real operational work "
        "is dominated by exceptions that no script "
        "anticipates. A workflow automation tool that fails "
        "the moment it encounters an unusual invoice format, "
        "or a supplier who has not responded in the expected "
        "form, or a regulatory document with a slightly "
        "different structure, is a tool that hands the work "
        "back to the humans precisely at the moment the "
        "work becomes interesting. The aggregate productivity "
        "gain is much smaller than the vendors of these "
        "tools promise."))
    s.append(P(
        "The second conventional answer has been the wave of "
        "artificial-intelligence assistants released over "
        "the last few years. These assistants are genuinely "
        "useful for many things, but they are useful "
        "primarily as individual-productivity tools. They "
        "help a single human draft an email faster, "
        "summarise a document they did not have time to "
        "read, or generate a first draft of a piece of "
        "writing. They do not, by themselves, take on "
        "operational responsibility for an entire business "
        "process. They sit alongside human work; they do "
        "not replace it. The leadership teams of the "
        "industrial companies we serve have all experimented "
        "with these assistants and have, uniformly, "
        "reported that the experiments delivered marginal "
        "individual productivity gains but did not address "
        "the underlying capacity shortage."))
    s.append(P(
        "The third conventional answer has been to outsource "
        "more of the operational work to lower-cost "
        "geographies or to specialist business-process "
        "outsourcing firms. This is the strategy that has "
        "dominated the industrial back office for the last "
        "twenty years, and it has reached its limits. The "
        "cost arbitrage has narrowed as wages in the "
        "destination geographies have risen. The compliance "
        "overhead of operating across borders has expanded. "
        "The talent in the destination geographies has, "
        "sensibly, moved into the kind of work that creates "
        "more value for them than back-office outsourcing "
        "does. The industrial companies that leaned hardest "
        "on outsourcing are now actively looking for a way "
        "to bring the work back, ideally to a form of "
        "execution that does not require staffing the work "
        "with humans at all."))
    s.append(P(
        "This is the gap our company is built to fill. We "
        "are offering an answer that is not workflow "
        "automation, not individual-productivity artificial "
        "intelligence, and not outsourcing. We are offering "
        "an artificial workforce: a team of software "
        "workers who take on entire operations, run them "
        "continuously, handle exceptions as they arise, "
        "escalate to humans only when the situation "
        "genuinely requires human judgement, and leave "
        "behind a complete audit trail of everything they "
        "did. The closest analogy is to the introduction of "
        "mechanised power into manufacturing two centuries "
        "ago. The arrival of mechanical power did not, in "
        "the long run, reduce the number of humans employed "
        "in manufacturing; it vastly expanded what those "
        "humans could collectively produce. We believe the "
        "arrival of an artificial workforce will, in the "
        "long run, do the same for the office work of large "
        "enterprises."))

    s.append(PageBreak())
    s.append(eyebrow("Part Two · What makes us different"))
    s.append(h1("Why us, specifically."))
    s.append(P(
        "We owe you, having described the problem and the "
        "shape of the answer we are offering, an honest "
        "account of what specifically distinguishes our "
        "company from the considerable number of other "
        "companies who have made claims that sound, at a "
        "surface level, similar to ours. There are, by our "
        "count, at least sixty-five companies in the broader "
        "artificial-intelligence ecosystem who would "
        "describe themselves as building some version of an "
        "artificial workforce, and a careful reader is "
        "entitled to wonder what about us is genuinely "
        "different and what is merely better marketing."))
    s.append(P(
        "We believe there are four specific differences "
        "that matter. The first is that we have chosen to "
        "focus specifically on industrial enterprises "
        "rather than on the broader market of all "
        "businesses. This choice shapes everything else we "
        "do. Industrial enterprises have specific "
        "characteristics that the broader market does not "
        "share. Their operations are heavily "
        "exception-laden, because the physical world "
        "introduces exceptions that pure-digital businesses "
        "do not encounter. Their compliance burden is "
        "heavy, because they work in regulated supply chains "
        "and with physical-safety requirements. Their "
        "tolerance for system failures is low, because a "
        "failure in their operations means a real-world "
        "shipment is delayed or a real-world customer is "
        "left without a part. The design choices that work "
        "for a software-as-a-service business serving small "
        "companies do not transfer to an industrial "
        "enterprise serving real-world supply chains. We "
        "have built our product specifically for the second "
        "case, and the specificity is visible in almost "
        "every architectural decision we have made."))
    s.append(P(
        "The second difference is in our technical approach "
        "to the underlying intelligence. The dominant "
        "approach in the broader market has been to take a "
        "single large general-purpose model and to ask it "
        "to do everything the workforce needs to do, with "
        "the model's general knowledge and reasoning "
        "capacity standing in for specialised expertise. The "
        "approach has the advantage of simplicity and the "
        "disadvantage of unreliability: the larger and more "
        "general a model is, the more prone it is to "
        "producing plausible-sounding but incorrect outputs "
        "in any specific domain. Our approach is different. "
        "We use a set of smaller, specialised models, each "
        "one trained or fine-tuned for a specific "
        "operational domain, each one operating within "
        "explicit boundaries that prevent it from drifting "
        "outside its competence. The trade-off is that our "
        "approach is harder to build than the "
        "single-large-model approach; the benefit is that "
        "it is dramatically more reliable in the specific "
        "domains where industrial enterprises actually "
        "operate."))
    s.append(P(
        "The third difference is in the way we have "
        "approached deployment. The conventional approach "
        "in our industry is to operate the customer's "
        "workload from our own cloud infrastructure, which "
        "is operationally simpler for us but which requires "
        "the customer to send their data to a third party "
        "and to accept whatever security posture we are "
        "willing to commit to. Industrial enterprises, "
        "particularly the ones we serve, are deeply "
        "unwilling to do this. Their data is operationally "
        "sensitive, legally regulated, and often subject to "
        "specific obligations about where it can be "
        "processed and by whom. We have therefore designed "
        "our product to be deployed inside the customer's "
        "own cloud account, on infrastructure they control, "
        "with their data never leaving their environment. "
        "This is operationally harder for us, because we "
        "need to support deployment into many different "
        "customer environments rather than only our own, "
        "but it is the only structure that the customers we "
        "are serving will accept."))
    s.append(P(
        "The fourth difference is in the speed and shape of "
        "the customer onboarding. Industrial enterprises "
        "are famously slow to adopt new software. A typical "
        "enterprise software deployment in our space takes "
        "between six and eighteen months from initial "
        "contract to first measurable value, and the slow "
        "timeline is the single biggest reason large "
        "enterprises hesitate to commit. We have designed "
        "our product, our service offering, and our internal "
        "operations around a specific commitment that our "
        "first measurable value will be delivered within "
        "two to ten weeks of contract signature, depending "
        "on the complexity of the operation being addressed. "
        "We will return to this commitment in detail later "
        "in the brief, because it is one of the harder "
        "claims to defend and we want to give you the full "
        "argument for why we believe it is achievable. For "
        "now we want to register only that this is the "
        "fourth deliberate difference between us and the "
        "broader market."))
    s.append(P(
        "These four differences, taken together, are what "
        "we mean when we say that the company occupies a "
        "specific position in the broader "
        "artificial-workforce landscape that no other "
        "company we are aware of currently occupies in the "
        "same way. We do not claim to be the only company "
        "doing any single one of these things; for each of "
        "the four differences, we could name two or three "
        "competitors who do something similar. We claim "
        "that the combination of all four, in the specific "
        "way we have combined them, is what makes us the "
        "company we are."))
    s.append(pull(
        "We are not selling a tool that helps employees "
        "work faster. We are selling additional staff who "
        "happen to be made of software. The difference is "
        "not rhetorical. It is operational, financial, and "
        "organisational, and the rest of this brief is the "
        "elaboration of that difference."))

    # ── Part Three — The story so far ────────────────────────────────────
    s.append(NextPageTemplate("bleed"))
    s.append(PageBreak())
    s.append(FullBleedDivider("03", "The story so far",
        "How the company began, what we built in the long "
        "stealth year, and how we arrived at the position we "
        "describe in the rest of this brief."))

    s.append(NextPageTemplate("body"))
    s.append(PageBreak())
    s.append(eyebrow("Part Three · The founding"))
    s.append(h1("How the company came into being."))
    s.append(P(
        "The company was founded in the early months of "
        "twenty-twenty-five by a small group of engineers and "
        "operators based in Stockholm. The founding "
        "conversation took place between several people who "
        "had each, in their previous careers, been responsible "
        "for the operational technology inside one or more of "
        "the world's largest technology firms. Some of them "
        "had spent the preceding decade at a Swedish music "
        "streaming company that had grown from a regional "
        "experiment to a globally dominant platform; some had "
        "come from a German industrial conglomerate that had "
        "spent the preceding decade trying, with mixed "
        "results, to digitise its manufacturing operations; "
        "others had spent time at the major American "
        "technology firms whose names appear on every list of "
        "the largest companies in the world. The founding "
        "team was, by any reasonable measure, deeply "
        "experienced in the operational technology of large "
        "companies and in the artificial-intelligence systems "
        "that had begun, over the preceding two years, to "
        "transform what that operational technology could "
        "plausibly do."))
    s.append(P(
        "The chief executive of the company, who continues "
        "to lead it today, had spent the preceding several "
        "years running the artificial-intelligence efforts "
        "of the streaming company we mentioned above. He had "
        "subsequently founded and grown a smaller "
        "artificial-intelligence company that had, over a "
        "period of several years, transformed the back-office "
        "operations of a number of large Nordic enterprises, "
        "including a major telecommunications equipment "
        "manufacturer, a leading apparel retailer, a national "
        "airline, and a major Nordic bank. That smaller "
        "company had been acquired before our company was "
        "founded, and the chief executive had then taken "
        "approximately a year to think carefully about what "
        "the next iteration of his work should look like. The "
        "conclusion he reached, after many conversations with "
        "the kinds of customers he had previously served, was "
        "that the time had come to attempt something more "
        "ambitious than the previous company had been able to "
        "attempt within its initial constraints. The new "
        "company, the one whose brief you are now reading, "
        "is that attempt."))
    s.append(P(
        "The founding thesis, as we have already described, "
        "was that the moment had arrived in which an entire "
        "artificial workforce could be built and deployed "
        "inside a large enterprise, taking on real "
        "operational work end to end rather than serving as "
        "an assistant to existing human work. The thesis "
        "rested on three observations about the state of the "
        "relevant technology in early twenty-twenty-five. The "
        "first observation was that the smaller, specialised "
        "language models had matured to the point where they "
        "could be trusted to perform narrow operational tasks "
        "with high reliability, in a way that had not been "
        "true even one year earlier. The second observation "
        "was that the surrounding tooling for orchestrating "
        "such models, supervising them, and integrating them "
        "with existing enterprise systems had also matured to "
        "the point where building a coherent platform on top "
        "was a question of engineering rather than of "
        "fundamental research. The third observation was that "
        "the large industrial enterprises the founding team "
        "knew best had begun to express, with increasing "
        "urgency, that the existing approaches were not "
        "working and that they were actively looking for "
        "something fundamentally new."))
    s.append(P(
        "These three observations, taken together, convinced "
        "the founding team that the moment had arrived. They "
        "formed the company in February of twenty-twenty-five, "
        "with a small amount of personal capital, and "
        "committed to spending the following year building "
        "the platform in stealth before announcing the "
        "company's existence to the public. The decision to "
        "spend a year in stealth was deliberate. The founding "
        "team had observed that the broader "
        "artificial-intelligence sector was suffering from a "
        "surplus of premature announcements, in which "
        "companies announced products before they had built "
        "anything worth using and then spent the following "
        "year trying to live up to the announcement. We chose "
        "the opposite approach: to build the platform first, "
        "to secure the first paying customers second, and to "
        "announce the company's existence only once we had "
        "real evidence that the approach worked."))

    s.append(PageBreak())
    s.append(eyebrow("Part Three · The stealth year"))
    s.append(h1("What happened during the year of building."))
    s.append(P(
        "The first year of the company's life was spent doing "
        "two kinds of work in parallel: building the platform "
        "in its initial form, and finding the first customers "
        "who would be willing to deploy the platform on real "
        "operational work. The two kinds of work fed each "
        "other in a particular way that is worth describing, "
        "because it shaped the platform's eventual shape more "
        "than any of the founding team's prior intentions did."))
    s.append(P(
        "The building work began with a sketch of the "
        "platform's intended architecture, drawn on a "
        "whiteboard in the company's first office in early "
        "twenty-twenty-five. The sketch identified the four "
        "or five major components the platform would need, "
        "the ways they would compose, and the rough order in "
        "which they would be built. The sketch was, in "
        "retrospect, about sixty percent correct. The "
        "remaining forty percent was discovered only when the "
        "team began trying to deploy early versions of the "
        "platform against real customer operations and "
        "discovered that some of the assumptions in the "
        "original sketch were wrong. The platform's eventual "
        "shape, which we describe in the fourth part of this "
        "brief, is the result of approximately a year of "
        "iteration against real operational use, not the "
        "result of pure architectural thinking."))
    s.append(P(
        "The customer-finding work began with conversations "
        "between the chief executive and several executives "
        "at the kinds of large Nordic industrial firms the "
        "company intended to serve. The conversations were "
        "deliberately exploratory. Rather than presenting a "
        "finished product and asking the executives to buy "
        "it, the chief executive presented the founding "
        "thesis and asked the executives what would have to "
        "be true for them to consider deploying a software "
        "platform that took on entire operational "
        "responsibility. The executives' answers were "
        "remarkably consistent. They said, in various forms, "
        "that they would need the platform to deploy inside "
        "their own infrastructure, that they would need a "
        "complete audit trail of every action the platform "
        "took, that they would need the ability to take "
        "control back at any moment, and that they would "
        "need to see measurable value within the first two "
        "to three months of any engagement or they would "
        "cancel the project. Each of these requirements "
        "shaped the platform's design in fundamental ways."))
    s.append(P(
        "By the autumn of twenty-twenty-five, the platform "
        "had reached a state where it could be deployed on "
        "specific operational use cases inside a customer's "
        "infrastructure within a few weeks, with a clear "
        "supervisory layer, complete audit logging, and the "
        "ability for human operators to intervene at any "
        "point. The company began deploying the platform "
        "with its first paying customers, a small number of "
        "Nordic industrial firms who had been part of the "
        "early conversations and who agreed to commit "
        "commercially in exchange for a deep collaborative "
        "engagement during the deployment. The early "
        "deployments were not always smooth. The company "
        "discovered, during the first few engagements, that "
        "the platform needed capabilities the founding team "
        "had not originally anticipated, and that several of "
        "the capabilities they had built were less useful in "
        "practice than they had expected. The platform was "
        "modified in response to each deployment, and by "
        "the end of twenty-twenty-five the company had a "
        "set of working customer engagements, a platform "
        "that had been refined against real use, and "
        "approximately one million euros of accumulated "
        "revenue across the first several customer "
        "commitments."))

    s.append(PageBreak())
    s.append(eyebrow("Part Three · The funding round"))
    s.append(h1("The first round of professional capital."))
    s.append(P(
        "Late in twenty-twenty-five, with the platform "
        "working in production at several customers and with "
        "the first million euros of revenue in hand, the "
        "company began conversations with several venture "
        "capital firms about a first round of professional "
        "capital. The conversations were deliberately "
        "narrow. The founding team had decided early that "
        "they wanted to raise a small first round rather "
        "than the largest possible round, on the theory "
        "that a small round forces discipline about what to "
        "build and what to defer, while a large round "
        "introduces the temptation to spend money on things "
        "that do not yet have product-market evidence to "
        "justify them."))
    s.append(P(
        "The round closed in November of twenty-twenty-five, "
        "at a size of one and a half million euros, against "
        "a company valuation of approximately ten million "
        "euros. The lead investor was a Stockholm-based "
        "venture capital firm that had previously invested "
        "in several of the most successful Nordic technology "
        "companies of the preceding decade. Two other "
        "venture capital firms participated in the round, "
        "and several individual angel investors with "
        "particular relevance to the kinds of customers the "
        "company serves joined as well. The board now "
        "includes, in addition to the founding team, a "
        "senior figure with extensive experience on the "
        "boards of major Nordic enterprises, whose presence "
        "is intended to bring discipline and connection to "
        "the customer side of the company's strategy."))
    s.append(P(
        "The round was modest by the standards of the "
        "broader artificial-intelligence sector, where "
        "companies at comparable stages have routinely raised "
        "ten or twenty times as much capital. We chose this "
        "shape deliberately. We believed, and continue to "
        "believe, that the next eighteen months of the "
        "company's life are best spent extracting maximum "
        "learning from a small number of deep customer "
        "engagements, rather than attempting to scale "
        "rapidly to a much larger number of shallow "
        "engagements. The capital we have raised is "
        "sufficient to fund the team and the operations we "
        "anticipate needing through the second half of "
        "twenty-twenty-six, by which point we expect to "
        "have enough accumulated revenue and enough proven "
        "customer outcomes to raise a larger second round "
        "from a position of evidence rather than promise."))

    s.append(PageBreak())
    s.append(eyebrow("Part Three · The global launch"))
    s.append(h1("Coming out of stealth."))
    s.append(P(
        "Shortly after the funding round closed, the company "
        "announced its existence to the public for the first "
        "time. The announcement was accompanied by a public "
        "version of the company's website, by coverage in "
        "the technology press, and by a small number of "
        "deliberate conversations with industry analysts who "
        "cover the artificial-intelligence sector. The "
        "company was, from that point forward, no longer "
        "operating in stealth."))
    s.append(P(
        "The decision to launch globally rather than to "
        "expand regionally was, again, deliberate. The "
        "platform itself is geography-agnostic: it can be "
        "deployed inside any customer's infrastructure "
        "regardless of where that infrastructure is located. "
        "The bottleneck on growth is not the platform but "
        "the team's capacity to engage with new customers "
        "and to support them through their deployments. We "
        "concluded that announcing globally and then "
        "carefully selecting which inbound conversations to "
        "pursue would give us the best signal about where "
        "the strongest demand lies, without committing us to "
        "open offices in specific geographies before we had "
        "evidence that those geographies were the right "
        "places to focus."))
    s.append(P(
        "The months since the launch have, as a result, been "
        "spent answering inbound conversations from a wide "
        "range of geographies and selecting a small number of "
        "them for deep engagement. The current geographies "
        "of active customer work are the Nordic countries "
        "where we have been operating from the beginning, "
        "together with the early stages of work in "
        "Switzerland and the Benelux region. We will say "
        "more about the geographic expansion plan in the "
        "ninth part of this brief."))
    s.append(P(
        "The state of the company at the moment of this "
        "writing, which is the late spring of "
        "twenty-twenty-six, is therefore one of careful, "
        "deliberate, evidence-driven growth. We have a "
        "platform that has been tested against real "
        "operational use, a small but growing roster of "
        "paying customers, a first round of professional "
        "capital, and a forward plan that we believe is "
        "both ambitious and realistic. The remainder of "
        "this brief describes the platform, the operations, "
        "the customer experience, the economic argument, "
        "the trust posture, the forward plan, and the team "
        "in detail."))

    # ── Part Four — The platform in concepts ─────────────────────────────
    s.append(NextPageTemplate("bleed"))
    s.append(PageBreak())
    s.append(FullBleedDivider("04", "The platform in concepts",
        "What we have actually built, described in concepts "
        "rather than in implementation detail, and explained "
        "carefully enough that any literate reader can follow."))

    s.append(NextPageTemplate("body"))
    s.append(PageBreak())
    s.append(eyebrow("Part Four · The metaphor we use"))
    s.append(h1("An operating system for an artificial workforce."))
    s.append(P(
        "We have, throughout this brief, referred to our "
        "platform as an operating system for an artificial "
        "workforce. The phrase is meant carefully, and it is "
        "worth explaining what we mean by it before we "
        "describe the platform's specific components, because "
        "the metaphor shapes everything else in this part of "
        "the document."))
    s.append(P(
        "When you use a personal computer, you do not "
        "interact directly with the underlying hardware. You "
        "interact with an operating system that sits between "
        "you and the hardware, presenting you with a "
        "consistent set of tools for managing files, "
        "launching applications, configuring permissions, "
        "monitoring resource use, and recovering from errors. "
        "The operating system also presents a consistent set "
        "of tools to the applications running on it, so that "
        "an application written for one computer can run on "
        "another computer with similar hardware. The "
        "operating system is the layer that makes the "
        "personal computer usable, both for humans and for "
        "the applications they run."))
    s.append(P(
        "Our platform plays an analogous role in the "
        "relationship between a customer and the artificial "
        "workforce they intend to deploy. The customer does "
        "not interact directly with the underlying "
        "intelligence; they interact with our platform, which "
        "sits between them and the intelligence and which "
        "presents them with a consistent set of tools for "
        "describing the work the workforce should do, "
        "deploying the workforce, supervising it, "
        "configuring its boundaries of authority, monitoring "
        "what it produces, and intervening when "
        "intervention is needed. The platform also presents "
        "a consistent set of tools to the individual digital "
        "workers, so that a worker designed for one "
        "operational task can be combined with others into "
        "a coherent team without each worker needing to "
        "know about the others. The platform is the layer "
        "that makes the artificial workforce usable, both "
        "for the humans who supervise it and for the workers "
        "themselves."))
    s.append(P(
        "The metaphor is not perfect. An operating system "
        "for a personal computer is invisible to the "
        "ordinary user once the computer is configured; our "
        "platform is, by contrast, the visible layer through "
        "which our customer continuously interacts with the "
        "workforce. But the structural parallel holds: in "
        "both cases, the operating system is the "
        "consistent, durable layer that translates between "
        "the underlying capability and the surface "
        "interaction, and that absorbs the complexity that "
        "would otherwise spill into every individual "
        "application or every individual worker."))

    s.append(PageBreak())
    s.append(eyebrow("Part Four · The four major components"))
    s.append(h1("What the platform is made of."))
    s.append(P(
        "The platform is organised into four major "
        "components, each of which has a single "
        "responsibility, a well-defined boundary with its "
        "neighbours, and a name we use consistently inside "
        "the company. We describe each of the four in this "
        "section, in the order in which a customer "
        "encounters them during a typical deployment."))
    s.append(P(
        "The first component is the personal assistant that "
        "individual humans inside the customer organisation "
        "use to interact with the rest of the platform. This "
        "component is what a human operator sees when they "
        "want to design a new operation, supervise an "
        "existing one, handle an exception that the workforce "
        "has escalated to them, or ask a question about what "
        "the workforce has been doing recently. The personal "
        "assistant is deliberately designed to feel like a "
        "single coherent interface, even though it draws on "
        "the full capability of the platform behind it. From "
        "the operator's point of view, the personal "
        "assistant is the platform; from the platform's "
        "point of view, the personal assistant is the human "
        "interface."))
    s.append(P(
        "The second component is the supervisory dashboard "
        "that lets the customer oversee the workforce as a "
        "whole. This component is what a manager sees when "
        "they want to understand how the workforce is "
        "performing across its various operations, where the "
        "bottlenecks are, which exceptions are being "
        "escalated most frequently, and which areas of the "
        "workforce are working most efficiently. The "
        "supervisory dashboard is the layer at which the "
        "human accountability for the workforce lives: a "
        "manager using the dashboard has the same "
        "responsibility for the workforce that they would "
        "have for a team of human employees, and the "
        "dashboard gives them the visibility they need to "
        "exercise that responsibility."))
    s.append(P(
        "The third component is the runtime environment in "
        "which the individual digital workers actually "
        "execute. This is the part of the platform that does "
        "the operational work, taking instructions from the "
        "supervisory dashboard, drawing on the underlying "
        "intelligence to make decisions, and producing the "
        "outputs the customer's operations require. The "
        "runtime is the component a customer is least "
        "likely to interact with directly, because it sits "
        "below the dashboard and the personal assistant. But "
        "it is the component that does the actual work, and "
        "much of the platform's value comes from the care "
        "with which the runtime is built."))
    s.append(P(
        "The fourth component is the integration layer that "
        "connects the platform to the customer's existing "
        "operational systems. Industrial enterprises run on "
        "a complex stack of pre-existing software: their "
        "enterprise resource planning systems, their "
        "customer relationship management systems, their "
        "supply-chain management systems, their document "
        "repositories, their communication systems. The "
        "integration layer is the part of our platform that "
        "lets the digital workers actually read from and "
        "write to those pre-existing systems. Without the "
        "integration layer, the workforce would be locked "
        "out of the data and processes it needs to do its "
        "work. With the integration layer, the workforce "
        "becomes an active participant in the customer's "
        "existing operations."))
    s.append(P(
        "These four components compose into a single "
        "platform that the customer experiences as a "
        "coherent whole. The customer does not, in normal "
        "use, think about which component is doing what at "
        "any given moment; the components hand work to each "
        "other through clean internal interfaces and the "
        "complexity stays inside the platform. But each of "
        "the four components is independently buildable, "
        "independently testable, and independently "
        "replaceable as the platform evolves, which is what "
        "makes the platform as a whole sustainable to "
        "develop over the long horizons we anticipate."))

    s.append(PageBreak())
    s.append(eyebrow("Part Four · The blueprints that define operations"))
    s.append(h1("How a customer describes the work they want done."))
    s.append(P(
        "The most consequential design decision in the entire "
        "platform, and the one we are most often asked about "
        "by customers and prospective investors, concerns the "
        "way a customer describes the operation they want the "
        "workforce to take on. The decision was consequential "
        "because the platform's whole approach depends on it: "
        "if customers cannot describe their operations in a "
        "form the platform can understand, the platform "
        "cannot deploy a workforce to run those operations, "
        "and the entire value proposition collapses."))
    s.append(P(
        "We considered, during the platform's early design, "
        "two extreme approaches to this problem. The first "
        "extreme was to ask customers to write detailed, "
        "step-by-step scripts describing every action the "
        "workforce should take, every condition under which "
        "it should take each action, and every exception "
        "the workforce should handle. This is the approach "
        "that workflow automation tools have used for "
        "decades, and we have already explained why we "
        "rejected it: the scripts are brittle, exceptions "
        "are infinite, and the scripts themselves become a "
        "maintenance burden that grows over time. The "
        "second extreme was to ask customers to write a "
        "single sentence describing what they want and "
        "leave it entirely to the platform's intelligence "
        "to figure out the rest. This is the approach the "
        "broader artificial-intelligence sector has tended "
        "toward, and we rejected it as well: a single "
        "sentence does not give the platform enough "
        "structure to be reliable, and the workforce "
        "produced by such a sentence tends to behave "
        "unpredictably in ways that industrial customers "
        "will not tolerate."))
    s.append(P(
        "The approach we settled on is the middle path. The "
        "customer describes their operation through what "
        "we call an operations blueprint: a structured "
        "document, written in plain language, that "
        "describes the mission of the operation, the roles "
        "that make up the workforce running it, the "
        "responsibilities of each role, the boundaries of "
        "authority each role has, the handoffs between "
        "roles, the rules for escalation to a human "
        "supervisor, and the success criteria by which the "
        "operation will be measured. The blueprint is not a "
        "script; it does not specify the exact actions the "
        "workforce should take. The blueprint is, instead, "
        "the kind of document a thoughtful human manager "
        "would write if they were starting a new team of "
        "human employees and wanted to give them the "
        "context they would need to run the operation "
        "themselves. The intelligence inside the platform "
        "then takes the blueprint and uses it to drive the "
        "actual moment-to-moment decisions."))
    s.append(P(
        "The blueprint approach has, in our experience, "
        "three significant advantages. The first is that "
        "the blueprint is something the customer can write "
        "without specialised technical training. Any "
        "operational manager who is capable of writing job "
        "descriptions and onboarding documents for human "
        "employees is capable of writing a blueprint. The "
        "second is that the blueprint is durable: once "
        "written, it remains valid as long as the "
        "operation's structure remains the same, and it "
        "does not need to be updated every time the "
        "underlying technology evolves. The third is that "
        "the blueprint is auditable: a human supervisor "
        "looking at the workforce's behaviour can compare "
        "what the workforce did against what the blueprint "
        "said it should do, and any deviation is "
        "immediately visible."))
    s.append(P(
        "We have refined the blueprint format substantially "
        "over the course of the company's life. The "
        "original version, used during the early stealth "
        "year, was rougher than what we use today, and "
        "many of the refinements were prompted by specific "
        "moments when an early customer found the original "
        "format inadequate. The current version is, we "
        "believe, the right shape for the kinds of "
        "industrial operations we serve, although we "
        "expect it to continue to evolve as we encounter "
        "new operational domains."))

    s.append(PageBreak())
    s.append(eyebrow("Part Four · The foundation workers"))
    s.append(h1("The library of pre-built digital workers we ship."))
    s.append(P(
        "Even with a well-written blueprint, a customer "
        "cannot deploy an entire artificial workforce "
        "from nothing. The platform ships, as part of its "
        "core capability, a library of pre-built digital "
        "workers each of which is specialised for a "
        "particular kind of operational task. The library "
        "is not infinite; we have deliberately kept it "
        "small, in the belief that a small library of "
        "well-tested workers is more valuable than a large "
        "library of unreliable ones. Each worker in the "
        "library is the result of significant engineering "
        "investment, careful testing against real "
        "operational scenarios, and continuous refinement "
        "based on the experience of using the worker in "
        "production at our customers."))
    s.append(P(
        "The pre-built workers in the library cover the "
        "operational territory we have prioritised: workers "
        "that read and interpret incoming documents such "
        "as invoices, purchase orders, and supplier "
        "communications; workers that compose outgoing "
        "communications such as enquiries to suppliers, "
        "responses to customers, and internal status "
        "updates; workers that monitor specific data feeds "
        "and surface anomalies for human review; workers "
        "that compare records across systems and reconcile "
        "discrepancies; workers that prepare structured "
        "reports for regulatory or compliance purposes; "
        "and a small number of workers specialised for "
        "particular industrial domains where the operational "
        "patterns are distinctive enough to warrant a "
        "dedicated worker."))
    s.append(P(
        "When a customer's blueprint is processed by the "
        "platform, the platform identifies which of the "
        "pre-built workers are appropriate for the roles "
        "described in the blueprint, configures each "
        "worker with the specific context, integrations, "
        "and boundaries the blueprint requires, and "
        "composes them into a team that can run the "
        "operation. The composition step is one of the "
        "places where the platform's intelligence is most "
        "visible: the platform decides not only which "
        "workers to use but how they should hand work to "
        "each other, in what order, and under what "
        "conditions. The customer can override any of "
        "these composition decisions if they have a "
        "specific preference, but in our experience "
        "customers rarely override; the platform's "
        "default composition is, in the vast majority of "
        "cases, what the customer would have chosen "
        "themselves had they known to choose it."))
    s.append(P(
        "The library is one of the components of the "
        "platform we expect to grow most rapidly over the "
        "next eighteen months. Each new industrial domain "
        "we enter, and each new operational pattern we "
        "encounter inside our existing customers, "
        "potentially calls for the addition of one or "
        "more new pre-built workers. We have a deliberate "
        "process for deciding when to add a new worker to "
        "the library and when to expect customers to "
        "compose existing workers in new ways, and the "
        "library grows only when a new worker is "
        "warranted by the actual needs of our customer "
        "base rather than by speculative future demand."))

    s.append(PageBreak())
    s.append(eyebrow("Part Four · The integration with existing systems"))
    s.append(h1("How the workforce connects to what the customer already has."))
    s.append(P(
        "We have repeatedly mentioned that the workforce "
        "needs to read from and write to the customer's "
        "existing operational systems, and we want to "
        "describe in this section how that connection is "
        "actually achieved. The description matters because "
        "the connection is, in practical terms, the place "
        "where most of the technical risk of a deployment "
        "concentrates. A workforce that cannot reliably "
        "read the customer's data and cannot reliably write "
        "back into the customer's systems is a workforce "
        "that cannot do its job."))
    s.append(P(
        "Our approach to the integration problem is "
        "deliberately pragmatic. We do not require the "
        "customer to replace any of their existing systems. "
        "We do not require the customer to install custom "
        "software inside their existing systems. We do "
        "require the customer to give the platform "
        "appropriate access credentials to the existing "
        "systems, scoped to the specific data and actions "
        "the workforce will need, and we require the "
        "customer to define the scope precisely. The "
        "platform then accesses the existing systems "
        "through their standard application programming "
        "interfaces, in exactly the same way an authorised "
        "human user would access them, with one important "
        "addition: every access is logged in our platform's "
        "audit trail, with the specific worker, the "
        "specific action, the specific data, and the "
        "specific blueprint clause that motivated the "
        "access."))
    s.append(P(
        "We have invested heavily in the breadth of systems "
        "the platform integrates with out of the box. The "
        "list includes the major enterprise resource "
        "planning systems used by industrial firms, the "
        "major customer relationship management systems, "
        "the major office productivity suites, the major "
        "communications platforms, and several specialised "
        "industrial systems with which our customers "
        "frequently work. For systems that are not on the "
        "standard list, the platform has a general-purpose "
        "integration mechanism that allows us to add a new "
        "integration in a matter of days when a customer "
        "needs one. We have, in fact, added several new "
        "integrations over the course of customer "
        "deployments, and each new integration is "
        "subsequently available to other customers who need "
        "it."))
    s.append(P(
        "The integration with the customer's communications "
        "systems is worth a particular mention, because it "
        "is the integration that customers most often "
        "remark on positively. The platform integrates with "
        "the major messaging platforms used inside large "
        "enterprises, which means that a human supervisor "
        "can interact with the workforce through the same "
        "messaging system they use to interact with their "
        "human colleagues. A worker that needs human "
        "guidance can ask for it through the messaging "
        "system; a supervisor who wants to give a worker "
        "an additional instruction can do so through the "
        "same channel. The integration eliminates the "
        "friction of asking supervisors to log into yet "
        "another piece of software, and it has, in "
        "practice, been one of the design choices that "
        "customers report most positively about."))

    # ── Part Five — How the work actually happens ────────────────────────
    s.append(NextPageTemplate("bleed"))
    s.append(PageBreak())
    s.append(FullBleedDivider("05", "How the work actually happens",
        "The customer's experience, from the moment they sign "
        "with us to the moment the workforce is running their "
        "operation as a routine part of their business."))

    s.append(NextPageTemplate("body"))
    s.append(PageBreak())
    s.append(eyebrow("Part Five · The first conversations"))
    s.append(h1("How an engagement begins."))
    s.append(P(
        "A new customer engagement begins, in almost every "
        "case, with a conversation between a senior member "
        "of our team and a senior member of the customer "
        "organisation. The conversation is not a sales "
        "conversation in the traditional sense. We do not "
        "begin by describing our product. We begin by "
        "asking the customer to describe, in detail, the "
        "operational pain they would most like to relieve. "
        "The conversation typically takes between an hour "
        "and a half and three hours, depending on how much "
        "context the customer wants to provide, and at the "
        "end of it we are usually able to identify a "
        "specific operation, or a specific pair of "
        "operations, that we believe would be a strong "
        "first deployment."))
    s.append(P(
        "The criteria we use for choosing a first "
        "deployment are deliberate. The operation should "
        "be one where the customer is currently spending "
        "more human effort than they would like; one where "
        "the work is repetitive enough that a digital "
        "workforce can plausibly take it on; one where the "
        "tolerance for occasional errors is high enough "
        "that the early deployment does not need to be "
        "perfect to be valuable; and one where the customer "
        "has a specific person inside their organisation "
        "who is willing to spend time with our team during "
        "the deployment. The last criterion is the one "
        "that customers most often overlook, and it is the "
        "one we are most insistent about. A deployment "
        "without a committed customer participant fails, "
        "regardless of how good the platform is."))
    s.append(P(
        "Once the first deployment is chosen, the customer "
        "and our team draft together the initial version of "
        "the blueprint that will govern the operation. The "
        "drafting typically takes one or two working "
        "sessions, each of three or four hours, with the "
        "customer's operational expert in the room. The "
        "drafting is collaborative: we contribute our "
        "knowledge of how the platform handles different "
        "kinds of operational pattern, the customer "
        "contributes their knowledge of their specific "
        "business, and the result is a document that "
        "neither side could have produced alone. The "
        "drafting is also the moment at which the customer "
        "begins to understand, in operational rather than "
        "abstract terms, what the workforce will and will "
        "not do. Many of the most valuable conversations "
        "happen during this drafting, because the act of "
        "writing down what the operation actually does "
        "often reveals that the customer's existing "
        "understanding of their own operation is incomplete."))

    s.append(PageBreak())
    s.append(eyebrow("Part Five · The deployment"))
    s.append(h1("How the workforce is actually put into production."))
    s.append(P(
        "Once the initial blueprint is drafted, the "
        "deployment itself can begin. The first step is "
        "the technical setup: we provision the platform "
        "inside the customer's cloud account, configure the "
        "necessary access credentials to the customer's "
        "existing systems, and verify that all of the "
        "integration points are working. This step is "
        "almost always the slowest part of the deployment, "
        "because it requires coordination with the "
        "customer's information security team, their "
        "compliance team, and their information technology "
        "operations team. We have learned, over the course "
        "of many deployments, how to navigate these "
        "coordinations efficiently, and we have a "
        "deployment playbook that we use to ensure nothing "
        "is overlooked. The technical setup typically "
        "takes between three and seven business days."))
    s.append(P(
        "The second step is the configuration of the "
        "specific workers that will run the customer's "
        "operation. The platform takes the blueprint as "
        "input and produces a configured team of workers "
        "as output. The customer reviews the configuration "
        "and confirms that each worker has the expected "
        "scope of authority, the expected escalation "
        "rules, and the expected handoffs to other "
        "workers. This step typically takes between one "
        "and three business days, depending on the "
        "complexity of the operation."))
    s.append(P(
        "The third step is a supervised dry run, in which "
        "the configured workforce processes a small number "
        "of representative real cases under close human "
        "observation. The customer's operational expert "
        "watches what each worker does, compares it to "
        "what they would have expected, and flags any "
        "discrepancies. We then adjust the configuration "
        "or, if necessary, refine the blueprint itself "
        "until the workforce's behaviour matches the "
        "customer's expectations. The dry run typically "
        "runs for three to ten business days, depending "
        "on how many edge cases the operation entails."))
    s.append(P(
        "The fourth step is the gradual ramp-up of "
        "production traffic. We do not, in any deployment, "
        "switch the entire operation over to the digital "
        "workforce in a single moment. We instead route a "
        "small percentage of real cases through the "
        "workforce while the human team continues to "
        "handle the rest, and we gradually increase the "
        "percentage as the customer's confidence in the "
        "workforce grows. The ramp typically takes between "
        "one and four weeks, depending on the operation's "
        "tolerance for change and the customer's appetite "
        "for speed."))
    s.append(P(
        "The fifth and final step is the formal handover "
        "to routine operations. The workforce becomes the "
        "primary handler of the operation; the human team "
        "transitions from doing the work to supervising the "
        "workforce that does the work. The supervisory "
        "responsibility is real: the human team continues "
        "to be accountable for the operation's outcomes, "
        "and the platform's supervisory tools give them "
        "the visibility they need to exercise that "
        "accountability. The handover is a meaningful "
        "moment in any deployment, because it is the "
        "moment at which the customer's organisation "
        "begins to relate to the digital workforce as a "
        "permanent fixture rather than as a temporary "
        "experiment."))
    s.append(P(
        "The entire deployment, from contract signature "
        "to formal handover, typically takes between two "
        "and ten weeks. The variation reflects the "
        "complexity of the operation being addressed: "
        "simpler commercial processes are at the lower end "
        "of the range, more complex logistics or financial "
        "processes are at the upper end. The two-week "
        "lower bound is a deliberate commitment on our "
        "part; we do not begin engagements where we "
        "anticipate a longer timeline than ten weeks "
        "without explicit advance discussion with the "
        "customer about why and a written agreement on the "
        "extended schedule."))

    s.append(PageBreak())
    s.append(eyebrow("Part Five · Day-to-day supervision"))
    s.append(h1("What life looks like once the workforce is running."))
    s.append(P(
        "Once a workforce is in routine production, the "
        "customer's relationship with the platform changes "
        "shape. The early intensity of the deployment "
        "gives way to a steady operational rhythm, in "
        "which the workforce runs continuously, the human "
        "supervisor monitors its behaviour through the "
        "supervisory dashboard, and exceptions are handled "
        "as they arise. The day-to-day experience is, in "
        "the most successful deployments, deeply boring: "
        "the workforce does its work, the operation "
        "produces its outcomes, and nothing dramatic "
        "happens. This is the steady state we are aiming "
        "for in every engagement."))
    s.append(P(
        "The exceptions, when they do arise, fall into a "
        "small number of categories. The most common is "
        "the case where a worker encounters an input it "
        "does not know how to process: an unusual document "
        "format, a supplier who has not responded in the "
        "expected form, a record that is missing data the "
        "worker needs. In such cases, the worker stops, "
        "explains the situation in plain language through "
        "the supervisor's preferred messaging channel, and "
        "waits for human guidance. The supervisor either "
        "provides the missing information, instructs the "
        "worker how to handle the specific case, or takes "
        "over the case themselves. The platform records "
        "the entire interaction in the audit trail, so "
        "that subsequent occurrences of the same situation "
        "can be handled automatically by the worker on the "
        "basis of the human's earlier guidance."))
    s.append(P(
        "A less common but more interesting kind of "
        "exception is the case where a worker handles a "
        "situation in a way that is technically correct "
        "but operationally undesirable. In such cases, "
        "the supervisor reviews the worker's behaviour, "
        "decides that the existing blueprint does not "
        "fully capture the customer's intent, and refines "
        "the blueprint accordingly. The platform processes "
        "the refined blueprint and updates the worker's "
        "behaviour going forward. This kind of "
        "blueprint refinement is, in the long-running "
        "deployments we have, a routine activity: the "
        "blueprint is a living document, not a static "
        "specification, and the platform is designed to "
        "accommodate continuous refinement without "
        "disruption."))
    s.append(P(
        "The rarest kind of exception is the case where "
        "the platform itself behaves in an unexpected way. "
        "These cases are, in our experience, almost always "
        "the consequence of a worker encountering a "
        "situation we had not anticipated when we built "
        "that worker. We treat each such case as an "
        "opportunity to improve the underlying worker, "
        "and the improvements we make become available "
        "to all of our customers over the subsequent "
        "weeks. The continuous improvement of the "
        "underlying workers is one of the reasons we "
        "believe a customer who chooses the platform "
        "today is making a decision whose value will "
        "grow over time rather than diminish."))

    # ── Part Six — Why now, and not earlier or later ─────────────────────
    s.append(NextPageTemplate("bleed"))
    s.append(PageBreak())
    s.append(FullBleedDivider("06", "Why now",
        "What has changed in the world to make this approach "
        "possible, and why we believe the window is open "
        "specifically at this moment."))

    s.append(NextPageTemplate("body"))
    s.append(PageBreak())
    s.append(eyebrow("Part Six · The technical conditions"))
    s.append(h1("What had to be true for the platform to work."))
    s.append(P(
        "The platform we have built was not technically "
        "possible five years ago. The intelligence on which "
        "it depends did not exist in a form reliable enough "
        "for production use, the tooling for orchestrating "
        "such intelligence had not matured, and the "
        "operational patterns the platform draws on were "
        "still being worked out by researchers rather than "
        "deployed in production. The technology has moved "
        "rapidly over the last three years, and the result "
        "is that the platform we are building today would "
        "have been impossible to build with the same team "
        "and the same capital even two years earlier. We "
        "describe in this section the specific technical "
        "shifts that have made our approach possible."))
    s.append(P(
        "The first shift is the maturation of the "
        "underlying intelligence itself. The artificial "
        "intelligence systems of three years ago were "
        "capable, but they were not reliable: they would, "
        "given a difficult input, produce plausible-sounding "
        "outputs that were nonetheless wrong, and there was "
        "no easy way to predict when they would do this. "
        "Industrial customers, who cannot tolerate "
        "plausible-sounding errors in their operational "
        "work, would not have deployed such systems "
        "regardless of how marketing departments described "
        "them. The intelligence has, over the last two "
        "years, matured to the point where reliable "
        "operation in narrow domains is feasible. The "
        "models we use today, particularly the smaller, "
        "specialised models we have favoured, produce "
        "outputs that are reliable enough in their specific "
        "domains that an industrial customer can build "
        "operational dependencies on them."))
    s.append(P(
        "The second shift is the development of the "
        "techniques for keeping the intelligence within "
        "explicit boundaries. The earliest deployments of "
        "artificial intelligence in production environments "
        "tended to put the intelligence in charge of "
        "everything: an end-to-end system that took inputs, "
        "processed them through a large model, and produced "
        "outputs without any structural constraint on what "
        "the model was allowed to do. This approach is "
        "unsuited to industrial operations, where every "
        "action has consequences in the physical world and "
        "where unconstrained behaviour is unacceptable. The "
        "techniques for putting structured boundaries "
        "around model behaviour, for forcing the model to "
        "operate only within an explicit zone of "
        "competence, for routing decisions through human "
        "review when they cross the zone's edge, have "
        "matured to the point where building a "
        "production-grade workforce on top of them is "
        "feasible. We did not invent these techniques; we "
        "have composed them carefully into our platform."))
    s.append(P(
        "The third shift is the broader maturation of the "
        "surrounding software ecosystem. The platforms on "
        "which our customers' existing systems run have "
        "themselves evolved to expose richer programming "
        "interfaces. The communications systems we "
        "integrate with have expanded their capabilities "
        "for software agents. The compliance frameworks "
        "that govern industrial operations have begun to "
        "explicitly address the use of artificial "
        "intelligence in decision-making. Each of these "
        "ecosystem-level shifts has made our platform "
        "easier to build and easier to deploy, and they "
        "would not have been in place even one year "
        "earlier."))

    s.append(PageBreak())
    s.append(eyebrow("Part Six · The commercial conditions"))
    s.append(h1("Why customers are ready to listen now."))
    s.append(P(
        "Technical possibility is not the only condition "
        "that matters. A technology that is technically "
        "possible but commercially uninteresting does not "
        "become a company. Our approach has, in addition "
        "to the technical conditions we have just "
        "described, benefited from a set of commercial "
        "conditions inside our target customer base that "
        "have moved decisively over the last eighteen "
        "months."))
    s.append(P(
        "The first commercial condition is the shift in "
        "how senior executives at industrial companies "
        "think about artificial intelligence. Two years "
        "ago, the dominant attitude was caution and "
        "scepticism: most senior executives had heard "
        "extravagant claims about what artificial "
        "intelligence would do, had been disappointed by "
        "the gap between the claims and the reality, and "
        "had decided to wait until the technology proved "
        "itself in less critical contexts. The attitude "
        "has shifted. Senior executives at industrial "
        "companies now feel actively behind: they "
        "perceive that other industries have begun to "
        "deploy artificial intelligence in serious ways, "
        "that their competitors may already be doing so "
        "quietly, and that continuing to wait risks "
        "leaving their organisations structurally behind. "
        "The shift is recent, and it is large, and we "
        "believe it is durable."))
    s.append(P(
        "The second commercial condition is the labour "
        "shortage we have already described in detail. "
        "The kind of detail-oriented operations staff "
        "that industrial companies most depend on has "
        "become structurally scarce in the geographies "
        "where these companies operate. The conventional "
        "responses are exhausted. The remaining "
        "alternative, which is to find a way to do the "
        "work without hiring the people who do it today, "
        "is no longer a hypothetical strategic option; "
        "it is an operational necessity."))
    s.append(P(
        "The third commercial condition is the increasing "
        "operational complexity of industrial enterprises. "
        "The supply chains have become longer, the "
        "regulatory burdens have grown, the customer "
        "expectations have intensified, and the "
        "competitive pressure has not relented. The result "
        "is that the operational work each enterprise "
        "needs to do has expanded, even as the workforce "
        "available to do it has not. The gap between "
        "what needs to be done and what can be done with "
        "existing approaches is widening, and any "
        "credible new approach finds a willing audience."))
    s.append(P(
        "The fourth commercial condition, which we "
        "describe with some hesitation because it is "
        "harder to defend objectively, is the broader "
        "willingness of large industrial organisations to "
        "engage with smaller, more recent companies than "
        "they would have a decade ago. The dominant "
        "purchasing behaviour of industrial enterprises "
        "in the previous era was to buy from established "
        "vendors with long histories and dense customer "
        "bases. The recent generation of large industrial "
        "buyers has, in our experience, become more open "
        "to engaging with smaller and more recently "
        "founded vendors, particularly in domains where "
        "the established vendors have failed to keep up. "
        "We are the beneficiary of this shift, although "
        "we would not claim to have caused it."))
    s.append(P(
        "These four conditions together explain why we "
        "believe the moment is right, and why we did not "
        "wait. A company that delays will, in our view, "
        "miss the moment: by the time the platform has "
        "been built and the customer base has been "
        "established, the competitive landscape will have "
        "filled in, and the position we currently occupy "
        "will be occupied by someone else."))

    # ── Part Seven — The economic argument ───────────────────────────────
    s.append(NextPageTemplate("bleed"))
    s.append(PageBreak())
    s.append(FullBleedDivider("07", "The economic argument",
        "How customers extract value, what kinds of cost we "
        "displace, and what the long-term economic shape of "
        "a customer relationship looks like."))

    s.append(NextPageTemplate("body"))
    s.append(PageBreak())
    s.append(eyebrow("Part Seven · The value to the customer"))
    s.append(h1("How customers measure what they get from us."))
    s.append(P(
        "We have made the choice, throughout our customer "
        "engagements, to frame the value of the platform "
        "in operational rather than purely financial terms. "
        "The framing matters because it shapes which "
        "deployments we pursue, how we negotiate "
        "commercial arrangements, and how we measure "
        "success. We describe in this section the "
        "specific operational and financial measures by "
        "which our customers evaluate what we provide."))
    s.append(P(
        "The most common measure, and the one we encourage "
        "customers to focus on first, is the volume of "
        "operational work handled per unit of human "
        "supervisory time. A typical operation, before our "
        "platform arrives, consumes the full attention of "
        "a small team of human employees: they read the "
        "incoming work, process it, handle the exceptions, "
        "and prepare the outgoing communications. After "
        "our platform has been deployed and the digital "
        "workforce has taken on the routine work, the "
        "human team's role shifts to supervision: they "
        "review what the workforce has done, handle the "
        "exceptions the workforce has escalated, and "
        "intervene when judgement is needed. The ratio "
        "between the volume of work the human team can "
        "supervise and the volume they could previously "
        "process directly is, in our experience, between "
        "five and twenty depending on the operation. A "
        "team that could previously process one hundred "
        "cases a day can, with the digital workforce in "
        "place, supervise the processing of between five "
        "hundred and two thousand cases a day. The "
        "specific multiplier depends on the complexity of "
        "the operation and the rate at which exceptions "
        "arise."))
    s.append(P(
        "The second measure is the time elapsed between "
        "the moment an operational input arrives and the "
        "moment the corresponding output is produced. "
        "Many of the operations we serve are characterised "
        "by long queues: incoming work accumulates faster "
        "than the human team can process it, and the "
        "queue length grows until customer service "
        "deteriorates noticeably. The digital workforce, "
        "which can scale to whatever volume the customer "
        "needs, eliminates the queue almost immediately. "
        "An operation whose typical response time was "
        "measured in days before the deployment is, "
        "after the deployment, responding within hours "
        "or even minutes. The reduction in response time "
        "often has knock-on benefits the customer had not "
        "originally targeted: suppliers respond faster "
        "because they are paid faster, customer "
        "satisfaction improves because enquiries are "
        "addressed quickly, and downstream operations "
        "are no longer blocked by upstream delays."))
    s.append(P(
        "The third measure is the accuracy of the work "
        "produced. We are careful, when we discuss "
        "accuracy with customers, not to claim that the "
        "digital workforce never makes mistakes. It does "
        "make mistakes, although less often than human "
        "teams typically do on the same operations. The "
        "more important property is that the digital "
        "workforce's mistakes are visible: every action "
        "is logged, every decision can be traced back to "
        "the reasoning that produced it, and any mistake "
        "can be diagnosed precisely. Human mistakes in "
        "complex office operations are typically "
        "invisible until they surface much later as "
        "downstream consequences. The reduction in "
        "invisible-mistake risk is, in many of our "
        "customer engagements, the property the customer "
        "values most highly."))
    s.append(P(
        "The fourth measure, and the one that ultimately "
        "matters for the customer's profit and loss "
        "statement, is the cost displacement. A customer "
        "who would otherwise have spent additional "
        "resources on hiring new operations staff, on "
        "expanding their outsourcing arrangements, or on "
        "buying conventional workflow automation tools "
        "can, by deploying our platform instead, displace "
        "those costs. The specific magnitude of the "
        "displacement varies enormously by customer and "
        "by operation, but in the deployments we have "
        "observed it has typically been between three and "
        "ten times the cost of the platform itself, on a "
        "twelve-month horizon. We are deliberate about "
        "not over-promising on this measure during sales "
        "conversations; we describe it in the realistic "
        "terms we use here, and we let the early "
        "deployment surface the actual magnitude for each "
        "specific customer."))

    s.append(PageBreak())
    s.append(eyebrow("Part Seven · The shape of the commercial relationship"))
    s.append(h1("How we charge and how the contract is structured."))
    s.append(P(
        "Our commercial arrangements with customers are "
        "structured around an annual subscription to the "
        "platform, with the subscription size determined "
        "primarily by the number of digital workers the "
        "customer is deploying and the scope of "
        "operations those workers are running. We have "
        "chosen this structure for several reasons that "
        "are worth describing, because the structure of "
        "the commercial relationship is itself one of the "
        "things our customers and prospective customers "
        "discuss with us most."))
    s.append(P(
        "We considered, during the early commercial "
        "design, several alternative structures. One "
        "alternative was per-transaction pricing, in "
        "which the customer would pay a small amount for "
        "each unit of work the workforce processed. We "
        "rejected this because it discourages the "
        "customer from giving the workforce additional "
        "work to do, which is exactly the opposite of "
        "what we want them to do. A second alternative "
        "was per-outcome pricing, in which the customer "
        "would pay only when a specific outcome was "
        "achieved. We rejected this because the outcomes "
        "of industrial operations are entangled with "
        "many factors outside our platform's control, "
        "and the resulting commercial disputes would "
        "have been impossible to manage. The annual "
        "subscription, modulated by deployment size, was "
        "the structure that minimised the friction of "
        "the commercial conversation and gave the "
        "customer the right incentive to use the "
        "platform broadly within their organisation."))
    s.append(P(
        "The subscription includes the platform itself, "
        "the library of pre-built digital workers, the "
        "ongoing improvements to those workers, the "
        "deployment support during the first engagement, "
        "and the ongoing support during normal operations. "
        "Customers can add additional workers, additional "
        "operations, and additional integrations on top "
        "of the base subscription, with each addition "
        "priced to reflect the incremental work it "
        "represents. We have deliberately kept the "
        "pricing structure simple, in the belief that "
        "industrial customers value pricing they can "
        "understand and budget against more than they "
        "value clever pricing that captures every "
        "marginal unit of value."))
    s.append(P(
        "The typical contract length is one year, with "
        "automatic renewal in subsequent years unless "
        "the customer elects to cancel. The cancellation "
        "terms are explicit and customer-friendly: a "
        "customer who decides the platform is not "
        "delivering value can cancel at the end of the "
        "annual term without penalty. We have made this "
        "choice deliberately, in the belief that a "
        "company whose product is genuinely valuable has "
        "no reason to lock customers in, and that a "
        "customer who feels locked in is a customer who "
        "will eventually leave anyway and tell their "
        "industry peers about the experience."))
    s.append(P(
        "The economic shape of the customer relationship "
        "over time is, we believe, deeply favourable to "
        "both sides. The customer's value from the "
        "platform grows over time: as the workforce "
        "encounters more cases, it becomes better at "
        "handling them; as the customer's organisation "
        "becomes more comfortable with the workforce, "
        "they expand its scope; as our library of "
        "pre-built workers grows, new capabilities "
        "become available without additional engineering "
        "effort. Our cost of serving the customer "
        "decreases over time: the early engagement is "
        "the most expensive moment, and subsequent years "
        "of the relationship require substantially less "
        "support effort per unit of revenue. The "
        "combination produces what we believe will be a "
        "structurally attractive customer relationship "
        "on both sides, with high renewal rates and "
        "expanding revenue per customer over multi-year "
        "horizons."))

    # ── Part Eight — The trust posture ───────────────────────────────────
    s.append(NextPageTemplate("bleed"))
    s.append(PageBreak())
    s.append(FullBleedDivider("08", "The trust posture",
        "What we have done to make a sophisticated industrial "
        "buyer comfortable trusting a small, recently founded "
        "company with their operational data."))

    s.append(NextPageTemplate("body"))
    s.append(PageBreak())
    s.append(eyebrow("Part Eight · Where the data lives"))
    s.append(h1("Why we deploy inside the customer's environment."))
    s.append(P(
        "Industrial enterprises have, over the last "
        "decade, become deeply sceptical of software "
        "vendors who require their data to leave the "
        "customer's environment. The scepticism is well "
        "founded: data that leaves the environment is "
        "subject to the vendor's security posture, the "
        "vendor's operational discipline, the vendor's "
        "regulatory exposure, and the vendor's "
        "long-term existence. Each of these is a "
        "category of risk the customer cannot fully "
        "evaluate and cannot fully mitigate. The "
        "preference for keeping data inside the "
        "customer's own environment is therefore "
        "structural, and it is one we share."))
    s.append(P(
        "Our platform is deployed, in every customer "
        "engagement, inside the customer's own cloud "
        "account or on-premises infrastructure. The "
        "specific arrangement varies by customer: some "
        "customers use a public cloud provider, in "
        "which case we deploy inside the customer's own "
        "account on that provider; some customers use a "
        "private cloud, in which case we deploy inside "
        "their private cloud; some customers require "
        "on-premises deployment, in which case we "
        "deploy on their own physical hardware. In all "
        "cases, the data the platform processes never "
        "leaves the customer's control. The platform "
        "itself is delivered to the customer in a "
        "form that runs entirely inside their "
        "environment, with no required outbound network "
        "connection back to us during normal operation."))
    s.append(P(
        "The choice is operationally harder for us than "
        "the alternative. A vendor who operates a "
        "single shared cloud environment for all "
        "customers has a simpler operational life: one "
        "environment to monitor, one set of software "
        "versions to maintain, one performance profile "
        "to understand. A vendor who deploys into many "
        "different customer environments has to support "
        "each environment individually, manage version "
        "drift across the fleet, and accept that any "
        "given customer's environment may have "
        "characteristics no other customer has. We have "
        "accepted these costs because they are the "
        "costs of working with the kind of customer we "
        "have chosen to serve. An industrial customer "
        "who cannot deploy the platform inside their own "
        "environment will not deploy it at all, "
        "regardless of how compelling the product is."))
    s.append(P(
        "The in-environment deployment has, beyond the "
        "primary benefit of customer comfort, two "
        "secondary benefits worth mentioning. The first "
        "is that the platform's performance is bounded "
        "only by the customer's infrastructure, which "
        "many customers find more predictable than the "
        "shared performance of a vendor cloud. The "
        "second is that the customer's compliance "
        "posture toward the platform is exactly the same "
        "as their compliance posture toward any other "
        "software running inside their environment, "
        "which simplifies the conversations with their "
        "compliance and audit teams considerably."))

    s.append(PageBreak())
    s.append(eyebrow("Part Eight · The audit trail"))
    s.append(h1("Why every action the workforce takes is recorded."))
    s.append(P(
        "The second pillar of our trust posture, after "
        "the in-environment deployment, is the complete "
        "audit trail the platform produces. Every "
        "action a digital worker takes, every decision "
        "it makes, every escalation it raises, every "
        "data point it reads, and every output it "
        "produces is recorded in a structured form that "
        "the customer can inspect at any time. The "
        "audit trail is not a marketing claim; it is a "
        "core property of the platform's design, "
        "implemented from the platform's earliest "
        "versions and refined over the course of every "
        "customer engagement."))
    s.append(P(
        "The audit trail serves several purposes. The "
        "first is compliance: many of our customers "
        "operate in regulated industries where they are "
        "required to demonstrate, to external auditors "
        "or regulators, how specific operational "
        "decisions were made. The audit trail gives "
        "them this capability without additional effort: "
        "the records the platform produces are already "
        "in a structured form, already linked to the "
        "blueprint clauses that motivated each action, "
        "and already attributable to specific workers. "
        "Customers have reported that the audit trail "
        "the platform produces is, in many cases, "
        "richer and more useful than the audit trails "
        "their pre-existing systems produced for the "
        "same operations."))
    s.append(P(
        "The second purpose is operational diagnosis. "
        "When something goes wrong, the audit trail "
        "lets the customer and our team reconstruct "
        "what happened, why it happened, and what the "
        "platform did in response. We have, in the "
        "course of many deployments, used the audit "
        "trail to diagnose specific issues that would "
        "have been impossible to diagnose in a less "
        "instrumented system. The audit trail also "
        "lets us, with the customer's permission, "
        "study patterns across many cases and use those "
        "patterns to improve the underlying workers."))
    s.append(P(
        "The third purpose is human oversight. A "
        "supervisor reviewing the workforce's behaviour "
        "can use the audit trail to verify that the "
        "workforce is behaving as the blueprint "
        "specified, to identify cases where the "
        "blueprint may need refinement, and to track "
        "the workforce's performance over time. The "
        "audit trail is, in operational terms, the "
        "primary instrument through which the human "
        "supervisor remains in control of the digital "
        "workforce. Without the audit trail, the "
        "supervisor would have to either trust the "
        "workforce blindly or replicate its work "
        "themselves to verify it; neither of these is "
        "an acceptable basis for a long-term "
        "operational dependence."))

    s.append(PageBreak())
    s.append(eyebrow("Part Eight · The regulatory posture"))
    s.append(h1("How we engage with the regulations that govern our customers."))
    s.append(P(
        "The regulations that govern our customers' use "
        "of artificial intelligence are, as of the time "
        "of this writing, in a state of active evolution. "
        "The European Union has passed comprehensive "
        "legislation that imposes specific obligations "
        "on the use of artificial intelligence in "
        "high-risk decision-making, with the obligations "
        "phasing in over the next few years. Various "
        "national jurisdictions are passing supplementary "
        "regulations. The industrial regulators that "
        "govern our customers' specific sectors are "
        "issuing guidance on how the broader "
        "artificial-intelligence regulations apply to "
        "their specific industries. The regulatory "
        "landscape is, in short, dense and changing."))
    s.append(P(
        "Our approach to this landscape is to be ahead "
        "of the curve rather than behind it. We have "
        "designed the platform from the beginning to "
        "comply with the existing data-protection "
        "obligations that already apply across our "
        "target geographies. We have obtained the "
        "relevant independent certifications of our "
        "security posture, certifications that our "
        "customers and their auditors recognise. We "
        "have aligned the platform's design with the "
        "specific requirements of the artificial-"
        "intelligence legislation that has already "
        "passed, in advance of the obligations becoming "
        "binding. And we have built our internal "
        "operations to be able to demonstrate "
        "compliance to a regulator who asks, with the "
        "documentation already in place rather than "
        "needing to be assembled retroactively."))
    s.append(P(
        "We have also chosen to engage proactively "
        "with the regulators in our key jurisdictions. "
        "We participate in industry working groups, we "
        "respond to public consultations on draft "
        "regulations, and we speak openly about our "
        "approach to compliance in the hope that the "
        "regulators will, over time, develop a clear "
        "understanding of what good practice looks like "
        "in our specific category. The investment is "
        "operationally significant; we believe it will "
        "pay off, both in terms of the regulatory "
        "treatment we receive and in terms of the "
        "credibility our customers gain by working "
        "with a vendor who is taking the regulations "
        "seriously."))
    s.append(P(
        "The combination of in-environment deployment, "
        "complete audit trails, established independent "
        "certifications, proactive regulatory engagement, "
        "and the explicit human-in-the-loop design we "
        "have described elsewhere in this brief is what "
        "constitutes our trust posture. Each of the "
        "five elements alone would be a reasonable "
        "industry practice; the combination is what we "
        "believe makes us specifically suitable for the "
        "industrial customers we serve."))

    # ── Part Nine — Trajectory and expansion ─────────────────────────────
    s.append(NextPageTemplate("bleed"))
    s.append(PageBreak())
    s.append(FullBleedDivider("09", "Trajectory and expansion",
        "Where we are now, where we will be in twelve to "
        "eighteen months, and how we will get from one to "
        "the other."))

    s.append(NextPageTemplate("body"))
    s.append(PageBreak())
    s.append(eyebrow("Part Nine · Where we are today"))
    s.append(h1("The state of the company at the moment of this writing."))
    s.append(P(
        "The company is, at the moment of this writing, "
        "in a state we would describe as <i>early "
        "post-launch</i>. The product has been built, "
        "tested against real operational use, and "
        "deployed at a small but growing number of "
        "paying customers. The first round of "
        "professional capital has closed. The company "
        "has been publicly announced. The customers we "
        "currently serve are primarily in the Nordic "
        "industrial sector, with the earliest customers "
        "having been with us since the autumn of "
        "twenty-twenty-five and newer customers being "
        "added at a rate that accelerated noticeably "
        "after the public announcement."))
    s.append(P(
        "The team is, as of this writing, comprised of "
        "fewer than twenty people, with engineering and "
        "operations roughly evenly distributed. The "
        "office is in Stockholm, with several team "
        "members working remotely from elsewhere in "
        "Europe. The infrastructure on which we operate "
        "the platform is straightforward and "
        "well-understood; we have made deliberate "
        "choices to avoid the kind of operational "
        "complexity that would distract us from the "
        "customer-facing work."))
    s.append(P(
        "The financial position is sound. The capital "
        "we raised in the pre-seed round, combined with "
        "the revenue we have already accumulated, gives "
        "us a financial runway that extends well into "
        "the second half of twenty-twenty-six. The "
        "monthly burn is in line with what we projected "
        "to our investors at the time of the round, and "
        "the revenue growth has been faster than we "
        "projected. We anticipate that the combination "
        "of the existing capital, the growing revenue, "
        "and the next funding round we expect to raise "
        "later this year will fund the company "
        "comfortably through to the point of operational "
        "profitability, which we currently project for "
        "the latter half of twenty-twenty-seven."))

    s.append(PageBreak())
    s.append(eyebrow("Part Nine · The twelve-month plan"))
    s.append(h1("What we intend to do in the year ahead."))
    s.append(P(
        "The year ahead is organised around three "
        "concurrent objectives, each of which has "
        "specific milestones associated with it. We "
        "describe each of the three in this section, "
        "with enough detail that a reader who is "
        "underwriting the company's near-term execution "
        "can evaluate whether the objectives are "
        "realistic and whether the milestones are "
        "appropriate."))
    s.append(P(
        "The first objective is to deepen the existing "
        "customer relationships in the Nordic markets "
        "where we are already operating. The customers "
        "we serve today are, in almost every case, "
        "deploying the platform on a single operation "
        "within their organisation, often as a "
        "deliberate experiment. The pattern we observe, "
        "and expect to continue observing, is that "
        "customers who succeed with the first operation "
        "expand to additional operations within a few "
        "months. The deepening of existing relationships "
        "is therefore both the source of our most "
        "predictable near-term revenue growth and the "
        "primary evidence we will use to demonstrate "
        "product-market fit to our next round of "
        "investors. We aim to have, by the end of the "
        "twelve-month plan, at least two-thirds of our "
        "existing customers running the platform on "
        "two or more concurrent operations."))
    s.append(P(
        "The second objective is to expand our customer "
        "base into the adjacent geographies we have "
        "identified as the natural next markets. The "
        "Nordic markets where we began are well-suited "
        "to our approach, but they are not where the "
        "largest portion of the addressable opportunity "
        "lies. We have identified the Swiss, German, "
        "Dutch, and Belgian markets as the natural "
        "first expansion, with the United Kingdom and "
        "the French markets following thereafter. We "
        "are deliberately not attempting all of these "
        "expansions simultaneously; we will enter one "
        "market at a time, secure a small number of "
        "anchor customers, build the relationships and "
        "the operational infrastructure that the market "
        "requires, and only then move to the next "
        "market. The pace will be set by the time "
        "required to secure each market rather than by "
        "an arbitrary schedule."))
    s.append(P(
        "The third objective is to extend the library "
        "of pre-built digital workers into adjacent "
        "operational domains. The library today covers "
        "the operational territory we initially "
        "prioritised, but we have already identified "
        "several adjacent domains where our existing "
        "customers have asked us to extend. The "
        "extensions are not arbitrary additions; each "
        "one is motivated by a specific customer "
        "demand, and each one is built only when we "
        "have a customer committed to deploying it once "
        "built. The pace of the library's extension is "
        "therefore set by customer pull rather than by "
        "speculative roadmapping."))
    s.append(P(
        "These three objectives are mutually reinforcing. "
        "The deepening of existing relationships gives "
        "us the case studies we will use to enter new "
        "geographies. The new geographies give us the "
        "operational demands that drive the extension "
        "of the library. The extended library gives the "
        "existing customers more operations to deploy "
        "the platform against. Each cycle of the loop "
        "is expected to take a few months, and we "
        "anticipate completing two or three cycles over "
        "the twelve-month horizon of the plan."))

    s.append(PageBreak())
    s.append(eyebrow("Part Nine · The capital we will need"))
    s.append(h1("The next funding round and what it will buy."))
    s.append(P(
        "We anticipate raising a second round of "
        "professional capital in the latter half of "
        "twenty-twenty-six. The size of the round will "
        "be set by the team and infrastructure we "
        "expect to need over the subsequent eighteen "
        "to twenty-four months, calibrated against the "
        "accumulated revenue and the customer evidence "
        "we will be able to demonstrate by then. We "
        "expect the round to be substantially larger "
        "than the first round, because the next phase "
        "of the company's life involves the kind of "
        "team expansion and geographic expansion that "
        "the first round was not sized to support, but "
        "we will continue to favour capital efficiency "
        "over scale-at-any-cost."))
    s.append(P(
        "The capital from the next round will be "
        "directed primarily at three categories of "
        "investment. The first is the engineering and "
        "research talent required to extend the platform "
        "into the new operational domains we have "
        "identified, and to deepen the underlying "
        "intelligence in the ways the next phase of "
        "customer demand will require. The second is "
        "the customer-facing team required to support "
        "the expansion into new geographies and the "
        "deepening of relationships in existing ones. "
        "The third is the operational infrastructure "
        "required to support a larger fleet of customer "
        "deployments without compromising the "
        "deployment-speed commitments we make."))
    s.append(P(
        "We will not, in the next round, raise capital "
        "for purposes that do not have direct customer "
        "or product justification. We will not raise "
        "capital to fund a major marketing campaign "
        "that we do not yet have a clear customer "
        "demand pulling us toward. We will not raise "
        "capital to fund speculative product "
        "extensions for which we do not have a "
        "customer committed. We will not raise capital "
        "to fund an office in a geography we have not "
        "yet entered. The discipline of raising only "
        "what we will spend on work that has customer "
        "justification has served us well so far, and "
        "we intend to continue it."))

    # ── Part Ten — The people ────────────────────────────────────────────
    s.append(NextPageTemplate("bleed"))
    s.append(PageBreak())
    s.append(FullBleedDivider("10", "The people",
        "The founding team, the investors who have backed "
        "us, and the team we will build over the next year."))

    s.append(NextPageTemplate("body"))
    s.append(PageBreak())
    s.append(eyebrow("Part Ten · The founding team"))
    s.append(h1("Who we are and what we have done before."))
    s.append(P(
        "The company's founding team consists of a small "
        "group of people who each, in their previous "
        "careers, accumulated the specific experience the "
        "current company requires. We describe each of "
        "them briefly in this section, with enough detail "
        "that a reader can form a judgement about whether "
        "the team is qualified for the work it has "
        "undertaken. We have chosen, throughout this "
        "section, to focus on the team's collective "
        "capability rather than on individual biographies; "
        "the specific individual biographies are available "
        "to any reader who wishes to dig further, and we "
        "are happy to provide them."))
    s.append(P(
        "The chief executive of the company is a serial "
        "entrepreneur who previously led the "
        "artificial-intelligence efforts of a major "
        "European music streaming service, and who "
        "subsequently founded and led a smaller "
        "artificial-intelligence company that was acquired "
        "before the present company was founded. The "
        "smaller company had, during its independent "
        "existence, transformed the operational work of a "
        "number of large Nordic enterprises across the "
        "telecommunications, retail, transportation, and "
        "banking sectors. The experience of building and "
        "running that previous company is, in a precise "
        "and specific sense, the experience this company "
        "is now building on. The chief executive has, "
        "throughout his career, focused on the "
        "intersection of artificial intelligence and "
        "operational work inside large enterprises, and "
        "the present company is the most ambitious "
        "expression to date of that focus."))
    s.append(P(
        "The remainder of the founding team is drawn "
        "from a deliberate cross-section of the "
        "technology industry. Several members come from "
        "the same major European music streaming service "
        "the chief executive previously led, where they "
        "had been responsible for various parts of the "
        "operational and engineering infrastructure that "
        "allowed the service to scale globally. Several "
        "members come from a major German industrial "
        "conglomerate, where they had spent the "
        "preceding decade trying to digitise the "
        "operational work of one of the world's most "
        "complex industrial enterprises. Several members "
        "come from the major American technology firms, "
        "where they had been responsible for the "
        "engineering and product development of systems "
        "that have become household names. The "
        "combination of these backgrounds is what gives "
        "the team its specific texture: deep knowledge "
        "of what it takes to build artificial "
        "intelligence at scale, deep knowledge of what "
        "it takes to make such intelligence work inside "
        "an industrial enterprise, and deep knowledge of "
        "how the largest technology firms have organised "
        "themselves to ship products to global markets."))
    s.append(P(
        "We have been deliberate, throughout the team's "
        "growth, about hiring for the specific gaps in "
        "our collective capability rather than for "
        "general aptitude. Every team member has a "
        "specific reason to be at the company, and the "
        "collective team is constructed to cover the "
        "capability surface the company needs without "
        "redundant strength in any one area. This will, "
        "we expect, change as the company grows: a "
        "company of fifty people cannot afford to be as "
        "specifically constructed as a company of "
        "twenty. But for now, the team is small enough "
        "that we have been able to hire each person "
        "against a specific need, and the result is a "
        "team whose capability is, we believe, "
        "considerably greater than its size suggests."))

    s.append(PageBreak())
    s.append(eyebrow("Part Ten · The investors"))
    s.append(h1("Who has backed us, and why."))
    s.append(P(
        "The first round of professional capital, which "
        "closed in November of twenty-twenty-five, was "
        "led by a Stockholm-based venture capital firm "
        "with a portfolio of successful Nordic "
        "technology companies built over the preceding "
        "decade. The lead investor was chosen for two "
        "specific reasons. The first is their genuine "
        "understanding of the industrial customer base "
        "we are pursuing: their portfolio includes "
        "several companies that have grown by selling to "
        "similar customers, and the partners we work "
        "with have developed, over many such "
        "engagements, a specific perspective on what "
        "makes industrial enterprise sales succeed or "
        "fail. The second is their willingness to invest "
        "at the stage and size we wanted, in the "
        "structure we wanted, without the larger "
        "commitment that would have introduced the "
        "temptation to scale prematurely."))
    s.append(P(
        "Two other venture capital firms participated in "
        "the round, each with a specific angle on our "
        "work. One brings particular expertise in the "
        "kinds of artificial-intelligence companies that "
        "have succeeded at the intersection of consumer "
        "and enterprise technology. The other brings "
        "particular expertise in the European industrial "
        "sector specifically. The combination of the "
        "three firms gives us a board and an investor "
        "base that collectively understand the spectrum "
        "of considerations we will need to navigate over "
        "the next phase of the company's life."))
    s.append(P(
        "Several individual angel investors also joined "
        "the round. The angels were selected, in each "
        "case, for the specific contribution they could "
        "make beyond the capital itself. One is a "
        "long-serving board member at several major "
        "Nordic enterprises, whose presence on our own "
        "board brings discipline and connection to the "
        "customer side of our strategy. One is a former "
        "senior executive of a major "
        "artificial-intelligence research organisation, "
        "whose perspective on the evolution of the "
        "underlying technology is particularly valuable "
        "to us. The others are individuals who have "
        "themselves built and led companies at the "
        "intersection of artificial intelligence and "
        "enterprise software, and whose lived "
        "experience of the road we are now walking is "
        "operationally useful to us."))
    s.append(P(
        "The board, as constituted after the round, "
        "consists of the chief executive, two members "
        "drawn from the lead investor, and the senior "
        "Nordic-enterprise board veteran. The board "
        "meets quarterly, and the cadence and shape of "
        "the meetings is structured around the specific "
        "operational questions the company faces at "
        "each moment. We do not, as a matter of policy, "
        "use the board for ceremonial purposes; every "
        "board meeting addresses specific decisions and "
        "specific operational questions, and the board "
        "members are expected to contribute "
        "substantively rather than merely to observe."))

    s.append(PageBreak())
    s.append(eyebrow("Part Ten · The team we will build"))
    s.append(h1("How the team grows from here."))
    s.append(P(
        "The team will grow over the next year, but "
        "deliberately and in a specific shape. We have "
        "decided early that the company should remain "
        "small relative to its revenue and to the "
        "number of customers it serves. The reason for "
        "the decision is that, in our experience, the "
        "companies that ship the best industrial "
        "software are companies whose product team is "
        "small enough that everyone in it can hold the "
        "entire product in their head, whose "
        "customer-facing team is small enough that "
        "every team member knows every customer by "
        "name, and whose internal operations are small "
        "enough that decisions can be made quickly "
        "without requiring a complicated approval "
        "chain. The advantages of this kind of "
        "smallness are, in our view, considerable, and "
        "we want to retain them as long as we possibly "
        "can."))
    s.append(P(
        "We will, over the year ahead, hire selectively "
        "in three areas. The first is the engineering "
        "team that builds and maintains the platform; "
        "we expect this team to roughly double over "
        "the year, with the new hires concentrated in "
        "the specific capabilities the platform will "
        "need as it extends into new operational "
        "domains. The second is the deployment team "
        "that supports customers through their first "
        "engagements; we expect this team to grow "
        "somewhat faster than the engineering team, "
        "because each new customer engagement requires "
        "direct human support and the customer base is "
        "growing. The third is a small commercial team "
        "that will support the expansion into new "
        "geographies; we have, so far, sold the "
        "platform entirely through inbound interest, "
        "but we anticipate needing a modest outbound "
        "capability to enter the new markets "
        "effectively."))
    s.append(P(
        "We will not, over the year ahead, hire in "
        "several areas that other companies at our "
        "stage often emphasise. We will not build a "
        "large marketing function; we believe our most "
        "credible marketing is the direct testimony of "
        "our existing customers, and we will continue "
        "to invest in cultivating that testimony "
        "rather than in producing marketing content. "
        "We will not build a large "
        "business-development function aimed at "
        "strategic partnerships; we have so far found "
        "that the most useful partnerships emerge "
        "naturally from customer relationships, and we "
        "will continue to pursue them in that way. We "
        "will not build a large administrative "
        "function; the operational overhead of the "
        "company is, by deliberate choice, kept light, "
        "and we intend to keep it that way for as long "
        "as the size of the company allows."))

    # ── Part Eleven — Closing ────────────────────────────────────────────
    s.append(NextPageTemplate("bleed"))
    s.append(PageBreak())
    s.append(FullBleedDivider("11", "Closing",
        "The argument restated, a glossary of the most "
        "important terms, and how to get in touch with us."))

    s.append(NextPageTemplate("body"))
    s.append(PageBreak())
    s.append(eyebrow("Part Eleven · The argument restated"))
    s.append(h1("Five sentences for a busy reader."))
    s.append(P(
        "Having taken you through ten parts and many "
        "thousands of words, we owe you a compressed "
        "restatement, in case you find it useful to "
        "carry away from the brief. We give the "
        "restatement in five sentences, each of which "
        "summarises one of the brief's central claims."))
    s.append(P(
        "First: our company builds an artificial "
        "workforce for large industrial enterprises, "
        "specifically designed to take on entire "
        "operations end to end rather than to assist "
        "human employees with individual tasks."))
    s.append(P(
        "Second: we have arrived at a moment in which "
        "the technical conditions for such a workforce "
        "are met, the commercial conditions inside our "
        "target customer base are favourable, and the "
        "competitive landscape has not yet filled in "
        "with credible alternatives."))
    s.append(P(
        "Third: our specific position within that "
        "landscape is defined by four deliberate "
        "choices: a focus on industrial customers, an "
        "approach to intelligence that favours small "
        "specialised models over large generalist "
        "ones, a deployment posture that keeps the "
        "customer's data inside the customer's "
        "environment, and a commitment to deliver first "
        "measurable value within two to ten weeks of "
        "contract signature."))
    s.append(P(
        "Fourth: we have, in the first eighteen months "
        "of the company's life, built the platform, "
        "secured the first paying customers, "
        "accumulated more than one million euros of "
        "early revenue, raised a first round of "
        "professional capital, and launched globally."))
    s.append(P(
        "Fifth: the next twelve to eighteen months "
        "will be spent deepening existing customer "
        "relationships in the Nordic markets, "
        "expanding into adjacent European geographies "
        "in a deliberate sequence, extending the "
        "platform into adjacent operational domains as "
        "customer demand pulls us toward them, and "
        "raising the next round of capital at a moment "
        "of demonstrated rather than promised "
        "product-market evidence."))
    s.append(pull(
        "An artificial workforce, for industrial "
        "enterprises, deployed inside the customer's "
        "environment, delivering measurable value in "
        "weeks rather than months. That sentence is "
        "the company, in compressed form, and the rest "
        "of this brief is the elaboration."))

    s.append(PageBreak())
    s.append(eyebrow("Part Eleven · Glossary"))
    s.append(h1("Glossary of the most important terms."))
    s.append(P(
        "We collect here the most important "
        "ordinary-language terms used throughout the "
        "brief, in case the reader wants to refer back "
        "to them. The glossary is intentionally short; "
        "the brief is written in plain language "
        "throughout, and most of the vocabulary "
        "explains itself in context."))
    s.append(P(
        "<b>An artificial workforce.</b> A team of "
        "software workers that takes on entire "
        "operational responsibilities inside a customer "
        "organisation, runs them continuously, handles "
        "exceptions as they arise, and escalates to "
        "humans only when the situation genuinely "
        "requires human judgement."))
    s.append(P(
        "<b>A digital worker.</b> An individual software "
        "agent within the artificial workforce, "
        "specialised for a particular operational role, "
        "with an explicit job description and an "
        "explicit boundary of authority."))
    s.append(P(
        "<b>An operations blueprint.</b> The structured "
        "plain-language document, written by the "
        "customer with our team's assistance, that "
        "describes the mission, the roles, the "
        "responsibilities, the handoffs, the escalation "
        "rules, and the success criteria of an "
        "operation the workforce will run."))
    s.append(P(
        "<b>The supervisory dashboard.</b> The "
        "component of our platform through which a "
        "human manager oversees the workforce, monitors "
        "its performance across operations, and "
        "intervenes when intervention is needed."))
    s.append(P(
        "<b>The personal assistant.</b> The component "
        "of our platform through which an individual "
        "human operator interacts with the rest of the "
        "platform, designs operations, supervises the "
        "workforce, and handles escalations."))
    s.append(P(
        "<b>The library of pre-built workers.</b> The "
        "collection of digital workers we ship as part "
        "of the platform, each one specialised for a "
        "particular kind of operational task and "
        "available to be composed into customer "
        "workforces without being built from scratch."))
    s.append(P(
        "<b>The integration layer.</b> The component "
        "of our platform that connects the workforce "
        "to the customer's existing operational "
        "systems, so that the digital workers can read "
        "from and write to the customer's data without "
        "the customer having to install custom software "
        "inside their existing systems."))
    s.append(P(
        "<b>The in-environment deployment.</b> Our "
        "policy of deploying the platform inside the "
        "customer's own cloud account or on-premises "
        "infrastructure, so that the customer's data "
        "never leaves their control."))
    s.append(P(
        "<b>The audit trail.</b> The complete "
        "structured record, produced automatically by "
        "the platform, of every action the workforce "
        "has ever taken, every decision it has made, "
        "and every datum it has read, with each entry "
        "linked to the blueprint clause that motivated "
        "the corresponding action."))
    s.append(P(
        "<b>Small specialised models.</b> The "
        "technical choice we have made to build the "
        "workforce on a set of smaller, "
        "domain-specific intelligences rather than on "
        "a single large general-purpose model. The "
        "approach trades implementation complexity for "
        "operational reliability, in a direction we "
        "believe is decisively correct for the kind "
        "of customer we serve."))
    s.append(P(
        "<b>Human in the loop.</b> The design "
        "principle that every consequential decision "
        "the workforce makes is either explicitly "
        "within the boundary of authority the "
        "blueprint establishes for the relevant "
        "worker, or is escalated to a human supervisor "
        "for review before the decision is acted on."))

    s.append(PageBreak())
    s.append(eyebrow("Part Eleven · Closing"))
    s.append(h1("Final words and contact."))
    s.append(P(
        "We thank you for reading a document this "
        "long. We are aware that long documents have, "
        "in our industry, become unfashionable, and "
        "that a reader who has reached this point has "
        "made an investment of time and attention that "
        "we do not take for granted. The choice to "
        "write at this length was deliberate, and we "
        "hope the investment has been worthwhile. We "
        "have tried to be honest about what we know "
        "and what we do not, to explain the reasoning "
        "behind every non-obvious decision, and to "
        "avoid the kind of marketing language that "
        "obscures rather than illuminates."))
    s.append(P(
        "If, after reading the brief, you have "
        "follow-up questions for the founding team, we "
        "would be glad to receive them and to engage "
        "with them in whatever forum you find most "
        "useful. The team is small and our calendar "
        "is finite, but questions from serious readers "
        "of this brief have priority over almost any "
        "other work we do, because the credibility of "
        "this brief depends on our ability to answer "
        "them."))
    s.append(P(
        "The brief was written by the founding team "
        "during the late spring of twenty-twenty-six "
        "and reflects the state of the company at "
        "that moment. We will produce subsequent "
        "editions as the company evolves, and we will "
        "note in each edition the specific changes "
        "from the previous one so that a reader "
        "comparing two editions can see what has "
        "moved. The current edition is the first."))
    s.append(P(
        "We look forward to continuing the "
        "conversation, and we wish you well with "
        "whatever decision this brief is part of."))
    s.append(callout("CONTACT",
        "The founding team can be reached through the "
        "contact channels published on the company's "
        "public website. We respond to enquiries "
        "personally and promptly. We particularly "
        "welcome enquiries from readers who have "
        "specific questions about the technical or "
        "operational claims made in this brief, and "
        "from prospective customers in the industrial "
        "sectors and geographies we have described."))

    return s


def main() -> int:
    out_dir = pathlib.Path("docs/investors")
    out_dir.mkdir(parents=True, exist_ok=True)
    today = _today_iso()

    kovant_meta = DocMeta(
        title="Kovant — Investor brief",
        subtitle="Stockholm. Agentic operations for industrial enterprises. Edition One.",
        short_title="Kovant investor brief",
        edition=f"{today} · Ed. 1 · pre-seed",
        audience="Investors and partners",
        author="Founding team · Kovant",
        hero_kpi_value="€1.5M",
        hero_kpi_label="Pre-seed round closed November 2025, led by J12 Ventures, at a €10M valuation.",
    )
    target = out_dir / f"Kovant-investor-brief-{today}.pdf"
    doc = MagDocTemplate(str(target), kovant_meta)
    doc.build(build_kovant_story())
    print(f"  built {target}  ({target.stat().st_size:,} bytes)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
