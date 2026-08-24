#!/usr/bin/env python3
"""Convert ARCHITECTURE_GLOBALE.md to PDF using reportlab (pure Python, no system deps).

Custom markdown-to-flowables converter tuned for the features actually used
in this doc: headers, code blocks, tables, lists, bold/italic, inline code,
blockquotes, links, horizontal rules.
"""
from __future__ import annotations
import re
from pathlib import Path
from html import escape

from reportlab.lib import colors
from reportlab.lib.pagesizes import A4
from reportlab.lib.styles import ParagraphStyle, getSampleStyleSheet
from reportlab.lib.units import cm, mm
from reportlab.platypus import (
    BaseDocTemplate, Frame, PageTemplate,
    Paragraph, Preformatted, Table, TableStyle, Spacer,
    PageBreak, HRFlowable, ListFlowable, ListItem, KeepTogether,
)
from reportlab.lib.enums import TA_LEFT, TA_JUSTIFY
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont

# Register macOS Monaco (single TTF, full Unicode incl. box-drawing chars)
# so code blocks render diagrams correctly.
MONO_FONT = "Monaco"
try:
    pdfmetrics.registerFont(TTFont(MONO_FONT, "/System/Library/Fonts/Monaco.ttf"))
except Exception as e:
    # Fallback to Andale Mono if Monaco unavailable.
    try:
        pdfmetrics.registerFont(TTFont(MONO_FONT,
            "/System/Library/Fonts/Supplemental/Andale Mono.ttf"))
    except Exception:
        MONO_FONT = "Courier"  # last resort, no Unicode box-drawing

import sys

# CLI: python3 _md_to_pdf.py [input.md] [output.pdf]
if len(sys.argv) >= 2:
    SRC = Path(sys.argv[1]).resolve()
    OUT = Path(sys.argv[2]).resolve() if len(sys.argv) >= 3 else SRC.with_suffix(".pdf")
else:
    SRC = Path(__file__).parent / "ARCHITECTURE_GLOBALE.md"
    OUT = Path(__file__).parent / "ARCHITECTURE_GLOBALE.pdf"

# ───────────────────────── Styles ─────────────────────────

base = getSampleStyleSheet()

st_h1 = ParagraphStyle("h1", parent=base["Heading1"],
    fontSize=20, leading=24, spaceBefore=18, spaceAfter=10,
    textColor=colors.HexColor("#1d1d1f"), borderPadding=4)
st_h2 = ParagraphStyle("h2", parent=base["Heading2"],
    fontSize=15, leading=18, spaceBefore=14, spaceAfter=6,
    textColor=colors.HexColor("#1d1d1f"))
st_h3 = ParagraphStyle("h3", parent=base["Heading3"],
    fontSize=12, leading=15, spaceBefore=10, spaceAfter=4,
    textColor=colors.HexColor("#424245"))
st_h4 = ParagraphStyle("h4", parent=base["Heading4"],
    fontSize=10.5, leading=13, spaceBefore=8, spaceAfter=3,
    textColor=colors.HexColor("#424245"))
st_body = ParagraphStyle("body", parent=base["BodyText"],
    fontSize=9.5, leading=13, spaceBefore=2, spaceAfter=4,
    textColor=colors.HexColor("#1d1d1f"), alignment=TA_LEFT)
st_code = ParagraphStyle("code", parent=base["Code"],
    fontName=MONO_FONT, fontSize=7, leading=9,
    leftIndent=8, rightIndent=8,
    backColor=colors.HexColor("#f5f5f7"),
    borderColor=colors.HexColor("#e5e5ea"), borderWidth=0.5,
    borderPadding=6, spaceBefore=6, spaceAfter=8,
    textColor=colors.HexColor("#1d1d1f"))
st_quote = ParagraphStyle("quote", parent=st_body,
    leftIndent=14, rightIndent=8,
    fontSize=9.5, leading=13, textColor=colors.HexColor("#424245"),
    backColor=colors.HexColor("#f5f5f7"),
    borderPadding=6, spaceBefore=6, spaceAfter=8,
    borderColor=colors.HexColor("#007aff"), borderWidth=0)
st_list = ParagraphStyle("list", parent=st_body, leftIndent=14, bulletIndent=4)

# ───────────────────────── Inline formatting ─────────────────────────

INLINE_PATTERNS = [
    # Code (must run before bold/italic)
    (re.compile(r"`([^`]+)`"),
     lambda m: f'<font name="Courier" size="8" color="#d63384">{escape(m.group(1))}</font>'),
    # Bold
    (re.compile(r"\*\*([^*]+)\*\*"), lambda m: f"<b>{escape(m.group(1))}</b>"),
    # Italic (avoid eating bold)
    (re.compile(r"(?<!\*)\*([^*]+)\*(?!\*)"), lambda m: f"<i>{escape(m.group(1))}</i>"),
    # Links → [text](url) → underlined blue
    (re.compile(r"\[([^\]]+)\]\(([^)]+)\)"),
     lambda m: f'<link href="{m.group(2)}"><font color="#007aff">{escape(m.group(1))}</font></link>'),
]

# Emoji → text label substitutions (Monaco/Helvetica can't render color emoji)
EMOJI_MAP = {
    "❌": "[N/A]",
    "✅": "[OK]",
    "🟢": "[OK]",
    "🟡": "[STUB]",
    "🔴": "[MISSING]",
    "🔧": "[SHIPPED]",
    "🧩": "[STUB]",
    "📐": "[S&S]",
    "⭐": "*",
    "🔍": "→",
    "✨": "*",
    "📍": "[local]",
    "→": "->",
    "←": "<-",
    "▶": ">",
    "◀": "<",
    "🤖": "[bot]",
    "📚": "[docs]",
    "🛠️": "[tools]",
    "🔑": "[key]",
    "📊": "[data]",
    "📦": "[pkg]",
    "🚀": "[ship]",
}

def strip_emoji(text: str) -> str:
    for emo, repl in EMOJI_MAP.items():
        text = text.replace(emo, repl)
    return text

def render_inline(text: str) -> str:
    """Apply inline markdown. Protects inline `code` BEFORE bold/italic so that
    asterisks inside code are not mistaken for italic markers."""
    text = strip_emoji(text)

    # Phase 1: extract inline code with NUL-byte placeholders so they survive
    # the subsequent bold/italic substitutions intact.
    code_slots: list[str] = []
    def store_code(m):
        idx = len(code_slots)
        code_slots.append(m.group(1))
        return f"\x00CODE{idx}\x00"
    text = re.sub(r"`([^`]+)`", store_code, text)

    # Phase 2: escape HTML (placeholders survive — they have no HTML chars)
    out = escape(text)

    # Phase 3: bold then italic
    out = re.sub(r"\*\*([^*]+)\*\*", r"<b>\1</b>", out)
    out = re.sub(r"(?<![\*\w])\*([^*\n]+)\*(?![\*\w])", r"<i>\1</i>", out)

    # Phase 4: restore code slots with monospace styling (content re-escaped)
    for idx, raw in enumerate(code_slots):
        ph = f"\x00CODE{idx}\x00"
        out = out.replace(
            ph,
            f'<font face="{MONO_FONT}" size="8" color="#d63384">{escape(raw)}</font>')

    # Phase 5: links. Internal anchors (#foo) and relative paths become styled text.
    def _link(m):
        ltext, url = m.group(1), m.group(2)
        if url.startswith("#") or not re.match(r"^(https?|mailto|file)://", url, re.I):
            return f'<font color="#007aff"><u>{ltext}</u></font>'
        return f'<link href="{url}"><font color="#007aff"><u>{ltext}</u></font></link>'
    out = re.sub(r"\[([^\]]+)\]\(([^)]+)\)", _link, out)
    return out

# ───────────────────────── Block parsing ─────────────────────────

H_RE   = re.compile(r"^(#{1,4})\s+(.+?)\s*$")
HR_RE  = re.compile(r"^---+\s*$")
LI_RE  = re.compile(r"^[\-\*]\s+(.+)$")
OL_RE  = re.compile(r"^(\d+)\.\s+(.+)$")
TBL_SEP = re.compile(r"^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)+\|?\s*$")
QUOTE_RE = re.compile(r"^>\s?(.*)$")
FENCE_RE = re.compile(r"^```")

def split_table_row(line: str) -> list[str]:
    s = line.strip()
    if s.startswith("|"): s = s[1:]
    if s.endswith("|"):   s = s[:-1]
    return [c.strip() for c in s.split("|")]

def build_table(rows: list[list[str]]) -> Table:
    """Turn a list of rows into a reportlab Table. First row is header."""
    if not rows: return Spacer(1, 0)
    n_cols = max(len(r) for r in rows)
    rows = [r + [""] * (n_cols - len(r)) for r in rows]
    # Convert cell content (apply inline md) and wrap in Paragraph
    cell_style = ParagraphStyle("cell", parent=st_body, fontSize=8, leading=10, spaceBefore=0, spaceAfter=0)
    head_style = ParagraphStyle("hcell", parent=cell_style, fontName="Helvetica-Bold")
    data = []
    for i, r in enumerate(rows):
        style = head_style if i == 0 else cell_style
        data.append([Paragraph(render_inline(c), style) for c in r])
    # Column widths : distribute available width
    avail = A4[0] - 2 * 1.8 * cm
    col_w = [avail / n_cols] * n_cols
    t = Table(data, colWidths=col_w, repeatRows=1)
    t.setStyle(TableStyle([
        ("BACKGROUND", (0,0), (-1,0), colors.HexColor("#f5f5f7")),
        ("TEXTCOLOR", (0,0), (-1,0), colors.HexColor("#1d1d1f")),
        ("GRID", (0,0), (-1,-1), 0.4, colors.HexColor("#d2d2d7")),
        ("VALIGN", (0,0), (-1,-1), "TOP"),
        ("TOPPADDING", (0,0), (-1,-1), 4),
        ("BOTTOMPADDING", (0,0), (-1,-1), 4),
        ("LEFTPADDING", (0,0), (-1,-1), 6),
        ("RIGHTPADDING", (0,0), (-1,-1), 6),
        ("ROWBACKGROUNDS", (0,1), (-1,-1), [colors.white, colors.HexColor("#fafafa")]),
    ]))
    return t

def parse_blocks(md: str):
    """Yield flowables from markdown source."""
    lines = md.splitlines()
    i = 0
    n = len(lines)
    first_h1_seen = False
    while i < n:
        line = lines[i]
        stripped = line.rstrip()

        # Blank line
        if not stripped:
            i += 1
            yield Spacer(1, 3)
            continue

        # Fenced code block
        if FENCE_RE.match(stripped):
            i += 1
            buf = []
            while i < n and not FENCE_RE.match(lines[i].rstrip()):
                buf.append(lines[i])
                i += 1
            if i < n: i += 1   # consume closing fence
            code_text = strip_emoji("\n".join(buf))
            yield Preformatted(code_text, st_code)
            yield Spacer(1, 2)
            continue

        # Headers
        m = H_RE.match(stripped)
        if m:
            level = len(m.group(1))
            text = render_inline(m.group(2))
            style = {1: st_h1, 2: st_h2, 3: st_h3, 4: st_h4}[level]
            if level == 1:
                if first_h1_seen:
                    yield PageBreak()
                first_h1_seen = True
            yield Paragraph(text, style)
            i += 1
            continue

        # Horizontal rule
        if HR_RE.match(stripped):
            yield HRFlowable(width="100%", thickness=0.5,
                             color=colors.HexColor("#d2d2d7"),
                             spaceBefore=6, spaceAfter=6)
            i += 1
            continue

        # Blockquote (multi-line)
        if QUOTE_RE.match(stripped):
            buf = []
            while i < n and (QUOTE_RE.match(lines[i].rstrip()) or lines[i].strip() == ""):
                m = QUOTE_RE.match(lines[i].rstrip())
                if m: buf.append(m.group(1))
                elif buf and lines[i].strip() == "":
                    # blank line ends quote
                    break
                i += 1
            text = " ".join(buf).strip()
            if text:
                yield Paragraph(render_inline(text), st_quote)
            continue

        # Table (header row followed by separator row)
        if "|" in stripped and i + 1 < n and TBL_SEP.match(lines[i+1]):
            rows = [split_table_row(stripped)]
            i += 2
            while i < n and "|" in lines[i] and lines[i].strip():
                rows.append(split_table_row(lines[i]))
                i += 1
            yield build_table(rows)
            yield Spacer(1, 4)
            continue

        # Unordered list
        if LI_RE.match(stripped):
            items = []
            while i < n:
                cur = lines[i].rstrip()
                m = LI_RE.match(cur)
                if not m: break
                items.append(Paragraph(render_inline(m.group(1)), st_list))
                i += 1
            yield ListFlowable(items, bulletType="bullet",
                              start="•", leftIndent=12, bulletFontSize=8)
            continue

        # Ordered list
        if OL_RE.match(stripped):
            items = []
            while i < n:
                cur = lines[i].rstrip()
                m = OL_RE.match(cur)
                if not m: break
                items.append(Paragraph(render_inline(m.group(2)), st_list))
                i += 1
            yield ListFlowable(items, bulletType="1", leftIndent=14)
            continue

        # Paragraph — coalesce continuation lines until blank
        buf = [stripped]
        i += 1
        while i < n and lines[i].strip() and not H_RE.match(lines[i]) and \
              not FENCE_RE.match(lines[i]) and not HR_RE.match(lines[i]) and \
              not LI_RE.match(lines[i]) and not OL_RE.match(lines[i]) and \
              not QUOTE_RE.match(lines[i]) and "|" not in lines[i][:80]:
            buf.append(lines[i].strip())
            i += 1
        text = " ".join(buf)
        yield Paragraph(render_inline(text), st_body)

# ───────────────────────── Page template ─────────────────────────

# Title used in header — first H1 of the MD, otherwise the filename
DOC_TITLE_DEFAULT = SRC.stem.replace("_", " ")
def _first_h1(md_text: str) -> str:
    for line in md_text.splitlines():
        m = re.match(r"^#\s+(.+?)\s*$", line)
        if m: return m.group(1)
    return DOC_TITLE_DEFAULT

def header_footer(canvas, doc):
    canvas.saveState()
    canvas.setFont("Helvetica", 8)
    canvas.setFillColor(colors.HexColor("#86868b"))
    # Header
    canvas.drawString(1.8 * cm, A4[1] - 1.2 * cm, DOC_TITLE)
    canvas.drawRightString(A4[0] - 1.8 * cm, A4[1] - 1.2 * cm, "2026")
    canvas.setStrokeColor(colors.HexColor("#d2d2d7"))
    canvas.setLineWidth(0.3)
    canvas.line(1.8 * cm, A4[1] - 1.4 * cm, A4[0] - 1.8 * cm, A4[1] - 1.4 * cm)
    # Footer
    canvas.drawCentredString(A4[0] / 2, 1.2 * cm, f"— {doc.page} —")
    canvas.restoreState()

DOC_TITLE = DOC_TITLE_DEFAULT  # filled at runtime by main()

def main() -> None:
    global DOC_TITLE
    md_text = SRC.read_text(encoding="utf-8")
    # Truncate the title for header (keep readable)
    full_title = _first_h1(md_text)
    DOC_TITLE = full_title[:80] + ("…" if len(full_title) > 80 else "")

    doc = BaseDocTemplate(
        str(OUT),
        pagesize=A4,
        leftMargin=1.8 * cm, rightMargin=1.8 * cm,
        topMargin=2 * cm, bottomMargin=1.8 * cm,
        title=full_title,
        author="Rodrigue Pekar",
        subject=full_title,
    )
    frame = Frame(doc.leftMargin, doc.bottomMargin,
                  doc.width, doc.height, id="normal",
                  leftPadding=0, rightPadding=0, topPadding=0, bottomPadding=0)
    doc.addPageTemplates([PageTemplate(id="main", frames=[frame],
                                       onPage=header_footer)])

    flowables = list(parse_blocks(md_text))
    doc.build(flowables)
    print(f"Wrote {OUT} ({OUT.stat().st_size / 1024:.1f} KB)")

if __name__ == "__main__":
    main()
