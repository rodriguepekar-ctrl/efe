#!/usr/bin/env python3.12
# ─────────────────────────────────────────────────────────────────────
#  build_kovant_deck_v2.py
#
#  Kovant pitch deck v2 — English, solo founder, no tests narrative,
#  EFE + meta-swarm focus, honest current status.
#
#  Output : EFE-Kovant-Pitch-v2-2026-05-21.pptx (12 slides)
# ─────────────────────────────────────────────────────────────────────

from pptx import Presentation
from pptx.util import Inches, Pt
from pptx.dml.color import RGBColor
from pptx.enum.text import PP_ALIGN, MSO_ANCHOR
from pptx.enum.shapes import MSO_SHAPE

# Palette
BLACK = RGBColor(0x14, 0x14, 0x14)
DARK_BLUE = RGBColor(0x0a, 0x2e, 0x5c)
ACCENT = RGBColor(0x2d, 0x8a, 0x5c)
WARNING = RGBColor(0xc4, 0x4d, 0x3b)
LIGHT_GREY = RGBColor(0x6c, 0x6c, 0x6c)
BG_WHITE = RGBColor(0xff, 0xff, 0xff)
SOFT_BG = RGBColor(0xf5, 0xf7, 0xfa)

prs = Presentation()
prs.slide_width = Inches(13.333)
prs.slide_height = Inches(7.5)
SW, SH = prs.slide_width, prs.slide_height


def add_text(slide, left, top, width, height, text, *, size=18,
             bold=False, color=BLACK, align=PP_ALIGN.LEFT, anchor=MSO_ANCHOR.TOP,
             font="Calibri"):
    box = slide.shapes.add_textbox(left, top, width, height)
    tf = box.text_frame
    tf.word_wrap = True
    tf.vertical_anchor = anchor
    for i, line in enumerate(text.split("\n")):
        p = tf.paragraphs[0] if i == 0 else tf.add_paragraph()
        p.alignment = align
        run = p.add_run()
        run.text = line
        run.font.size = Pt(size)
        run.font.bold = bold
        run.font.color.rgb = color
        run.font.name = font
    return box


def add_box(slide, left, top, width, height, color, line_color=None):
    shape = slide.shapes.add_shape(MSO_SHAPE.RECTANGLE, left, top, width, height)
    shape.fill.solid()
    shape.fill.fore_color.rgb = color
    if line_color:
        shape.line.color.rgb = line_color
        shape.line.width = Pt(0.5)
    else:
        shape.line.fill.background()
    return shape


def add_footer(slide, n, total=12):
    add_text(slide, Inches(0.4), SH - Inches(0.4), Inches(8), Inches(0.3),
             "EFE — Kovant Pitch — 21 May 2026", size=9, color=LIGHT_GREY)
    add_text(slide, SW - Inches(1.2), SH - Inches(0.4), Inches(0.8), Inches(0.3),
             f"{n}/{total}", size=9, color=LIGHT_GREY, align=PP_ALIGN.RIGHT)


def add_header_bar(slide):
    bar = slide.shapes.add_shape(MSO_SHAPE.RECTANGLE,
                                  Inches(0), Inches(0), SW, Inches(0.08))
    bar.fill.solid()
    bar.fill.fore_color.rgb = DARK_BLUE
    bar.line.fill.background()


# ─────────────────────────────────────────────────────────────────────
#  Slide 1 — Title
# ─────────────────────────────────────────────────────────────────────
slide = prs.slides.add_slide(prs.slide_layouts[6])

top_bar = slide.shapes.add_shape(MSO_SHAPE.RECTANGLE,
                                  Inches(0), Inches(0), SW, Inches(2.0))
top_bar.fill.solid()
top_bar.fill.fore_color.rgb = DARK_BLUE
top_bar.line.fill.background()

add_text(slide, Inches(0.8), Inches(0.4), Inches(11), Inches(0.5),
         "EFE", size=52, bold=True, color=BG_WHITE)
add_text(slide, Inches(0.8), Inches(1.25), Inches(11), Inches(0.6),
         "Federated AI infrastructure for regulated industries",
         size=22, color=BG_WHITE)

add_text(slide, Inches(0.8), Inches(2.7), Inches(11.7), Inches(1.5),
         "Production agentic AI — multi-tenant isolation, audit trail,\n"
         "federated execution. By construction. European-built.",
         size=28, bold=False, color=BLACK)

add_box(slide, Inches(0.8), Inches(5.5), Inches(5.8), Inches(1.4),
        BG_WHITE, line_color=DARK_BLUE)
add_text(slide, Inches(1.0), Inches(5.65), Inches(5.5), Inches(0.4),
         "Rodrigue Pekar — Solo Founder",
         size=15, bold=True, color=DARK_BLUE)
add_text(slide, Inches(1.0), Inches(6.05), Inches(5.5), Inches(0.4),
         "EPFL Lausanne", size=12, color=BLACK)
add_text(slide, Inches(1.0), Inches(6.4), Inches(5.5), Inches(0.4),
         "Kovant Pitch — Thursday 21 May 2026 — 14:00 CEST",
         size=12, color=BLACK)

add_footer(slide, 1)

# ─────────────────────────────────────────────────────────────────────
#  Slide 2 — The Problem
# ─────────────────────────────────────────────────────────────────────
slide = prs.slides.add_slide(prs.slide_layouts[6])
add_header_bar(slide)

add_text(slide, Inches(0.5), Inches(0.5), Inches(12), Inches(0.8),
         "The problem", size=38, bold=True, color=DARK_BLUE)
add_text(slide, Inches(0.5), Inches(1.35), Inches(12), Inches(0.6),
         "Deploying AI in regulated industries is broken.",
         size=20, color=LIGHT_GREY)

problems = [
    ("Multi-tenant isolation",
     "Hard tenant boundaries (per-business-unit for banks, per-study for pharma) "
     "require kernel-level isolation, per-tenant cost caps, and provable separation."),
    ("Reproducible audit trail",
     "Regulators (FINMA, FDA, Swissmedic) demand reproducibility 7+ years later. "
     "Decisions must be replayable from inputs, with cryptographic provenance."),
    ("Federated execution",
     "Workloads span multiple participants : hospitals, business units, jurisdictions. "
     "Coordinating typed messages with audit traces over a mesh is non-trivial."),
]

for i, (title, desc) in enumerate(problems):
    y = Inches(2.4 + i * 1.35)
    add_box(slide, Inches(0.6), y, Inches(12.2), Inches(1.15),
            SOFT_BG, line_color=DARK_BLUE)
    add_text(slide, Inches(0.9), y + Inches(0.1), Inches(11.5), Inches(0.5),
             title, size=18, bold=True, color=DARK_BLUE)
    add_text(slide, Inches(0.9), y + Inches(0.55), Inches(11.5), Inches(0.55),
             desc, size=13, color=BLACK)

add_text(slide, Inches(0.5), Inches(6.7), Inches(12.3), Inches(0.4),
         "Every enterprise rebuilds this stack. 12+ months of platform engineering. Per customer.",
         size=14, bold=True, color=WARNING, align=PP_ALIGN.CENTER)

add_footer(slide, 2)

# ─────────────────────────────────────────────────────────────────────
#  Slide 3 — What is EFE
# ─────────────────────────────────────────────────────────────────────
slide = prs.slides.add_slide(prs.slide_layouts[6])
add_header_bar(slide)

add_text(slide, Inches(0.5), Inches(0.5), Inches(12), Inches(0.8),
         "What EFE is", size=38, bold=True, color=DARK_BLUE)
add_text(slide, Inches(0.5), Inches(1.35), Inches(12), Inches(0.6),
         "A three-layer platform. 35 Rust crates. 15 binaries. 17 months of focused work.",
         size=18, color=LIGHT_GREY)

# 3 layers stacked
layers = [
    ("LAYER 1 — Classification & inference API",
     "M14-M21",
     "Classify, embed, route to LLM providers. MCP server. Prompts versioning + A/B. Cost caps.",
     RGBColor(0xe6, 0xee, 0xfa)),
    ("LAYER 2 — Meta-swarm federated execution",
     "M14, M20, M25",
     "Controller orchestrates N participants over typed gRPC. Firecracker microVM isolation. "
     "Tool whitelist. Step approval. The unique architectural piece.",
     RGBColor(0xe6, 0xf2, 0xeb)),
    ("LAYER 3 — Audit, compliance, bench platform",
     "M22-M28",
     "HMAC audit chain. DPIA + AI Act compliance gating. Signed quarterly evidence (cosign + Rekor). "
     "Active learning loop. Backup + DR drill harness.",
     RGBColor(0xfa, 0xee, 0xe6)),
]

for i, (title, span, desc, bg) in enumerate(layers):
    y = Inches(2.3 + i * 1.5)
    add_box(slide, Inches(0.6), y, Inches(12.2), Inches(1.3),
            bg, line_color=DARK_BLUE)
    add_text(slide, Inches(0.9), y + Inches(0.1), Inches(8.5), Inches(0.5),
             title, size=16, bold=True, color=DARK_BLUE)
    add_text(slide, Inches(9.5), y + Inches(0.1), Inches(3), Inches(0.5),
             span, size=14, bold=True, color=ACCENT, align=PP_ALIGN.RIGHT)
    add_text(slide, Inches(0.9), y + Inches(0.65), Inches(11.5), Inches(0.6),
             desc, size=12, color=BLACK)

add_footer(slide, 3)

# ─────────────────────────────────────────────────────────────────────
#  Slide 4 — The Meta-swarm layer (the unique piece)
# ─────────────────────────────────────────────────────────────────────
slide = prs.slides.add_slide(prs.slide_layouts[6])
add_header_bar(slide)

add_text(slide, Inches(0.5), Inches(0.5), Inches(12), Inches(0.8),
         "The meta-swarm layer — what makes EFE unique",
         size=32, bold=True, color=DARK_BLUE)
add_text(slide, Inches(0.5), Inches(1.3), Inches(12), Inches(0.6),
         "L2 federated execution over typed gRPC mesh.",
         size=18, color=LIGHT_GREY)

# Architecture diagram (simplified ASCII-style)
add_box(slide, Inches(0.6), Inches(2.2), Inches(12.2), Inches(2.6),
        RGBColor(0xf8, 0xf9, 0xfa), line_color=DARK_BLUE)

# Controller box
add_box(slide, Inches(5.3), Inches(2.5), Inches(2.7), Inches(0.7),
        DARK_BLUE)
add_text(slide, Inches(5.3), Inches(2.6), Inches(2.7), Inches(0.5),
         "CONTROLLER", size=14, bold=True, color=BG_WHITE,
         align=PP_ALIGN.CENTER)

# 3 participants
for i, x in enumerate([1.5, 5.5, 9.5]):
    add_box(slide, Inches(x), Inches(3.7), Inches(2.4), Inches(0.9),
            ACCENT)
    add_text(slide, Inches(x), Inches(3.8), Inches(2.4), Inches(0.4),
             f"Participant {i+1}", size=12, bold=True, color=BG_WHITE,
             align=PP_ALIGN.CENTER)
    add_text(slide, Inches(x), Inches(4.2), Inches(2.4), Inches(0.4),
             "Firecracker microVM", size=10, color=BG_WHITE,
             align=PP_ALIGN.CENTER)

# Arrows label
add_text(slide, Inches(0.6), Inches(3.2), Inches(12), Inches(0.4),
         "gRPC mesh — typed protocol — audit trace per call",
         size=11, color=LIGHT_GREY, align=PP_ALIGN.CENTER)

# Properties
properties = [
    ("Tenant isolation",   "Firecracker microVM per tenant — hard kernel boundary"),
    ("Tool whitelist",      "Each participant restricted to a closed set of operations"),
    ("Step approval",       "Human-in-the-loop on actions above configurable thresholds"),
    ("Cost cap",            "Per-participant + per-tenant LLM call budgets"),
    ("Audit trace",         "HMAC chain on every gRPC call, replayable years later"),
]

for i, (k, v) in enumerate(properties):
    y = Inches(5.1 + i * 0.35)
    add_text(slide, Inches(0.8), y, Inches(3.5), Inches(0.3),
             k, size=12, bold=True, color=DARK_BLUE)
    add_text(slide, Inches(4.5), y, Inches(8), Inches(0.3),
             v, size=11, color=BLACK)

# Use case footer
add_text(slide, Inches(0.5), Inches(6.95), Inches(12.3), Inches(0.4),
         "Example : hospital network — each hospital runs a participant, controller orchestrates clinical decision support across sites.",
         size=11, color=LIGHT_GREY, align=PP_ALIGN.CENTER)

add_footer(slide, 4)

# ─────────────────────────────────────────────────────────────────────
#  Slide 5 — Why regulated industries (with sectors)
# ─────────────────────────────────────────────────────────────────────
slide = prs.slides.add_slide(prs.slide_layouts[6])
add_header_bar(slide)

add_text(slide, Inches(0.5), Inches(0.5), Inches(12), Inches(0.8),
         "Target market — regulated industries",
         size=36, bold=True, color=DARK_BLUE)
add_text(slide, Inches(0.5), Inches(1.3), Inches(12), Inches(0.6),
         "Compliance is the moat. Speed is not.",
         size=18, color=LIGHT_GREY)

sectors = [
    ("Pharma R&D",         "Roche, Novartis, Lonza, Galderma",            "EUR 1-2 M/yr",  "★★★★★"),
    ("Hedge funds / quant","Pictet, Lombard Odier, BlueOrchard",          "EUR 200 k-1 M/yr", "★★★★★"),
    ("Cybersecurity MSSP", "Kudelski, Exeon, Eviden",                     "EUR 200-600 k/yr", "★★★★★"),
    ("Big 4 consulting",   "KPMG, PwC, Deloitte, EY (Switzerland)",       "EUR 500 k-1.5 M/yr","★★★★"),
    ("Legal RegTech",      "Apiax, Eraneos, Luminance",                   "EUR 200-800 k/yr", "★★★★"),
    ("Insurance claims",   "Zurich, Helvetia, Vaudoise, Mobilière",       "EUR 200 k-1 M/yr", "★★★★"),
]

add_text(slide, Inches(0.6), Inches(2.3), Inches(2.8), Inches(0.3),
         "Sector", size=11, bold=True, color=LIGHT_GREY)
add_text(slide, Inches(3.5), Inches(2.3), Inches(5.3), Inches(0.3),
         "Swiss target prospects", size=11, bold=True, color=LIGHT_GREY)
add_text(slide, Inches(9.0), Inches(2.3), Inches(2.3), Inches(0.3),
         "Ticket size", size=11, bold=True, color=LIGHT_GREY)
add_text(slide, Inches(11.4), Inches(2.3), Inches(1.5), Inches(0.3),
         "Fit", size=11, bold=True, color=LIGHT_GREY)

for i, (sector, prospects, ticket, fit) in enumerate(sectors):
    y = Inches(2.7 + i * 0.55)
    if i % 2 == 0:
        add_box(slide, Inches(0.4), y - Inches(0.05), Inches(12.5), Inches(0.55),
                RGBColor(0xf8, 0xf9, 0xfa))
    add_text(slide, Inches(0.6), y, Inches(2.8), Inches(0.4),
             sector, size=13, bold=True, color=BLACK)
    add_text(slide, Inches(3.5), y, Inches(5.3), Inches(0.4),
             prospects, size=11, color=LIGHT_GREY)
    add_text(slide, Inches(9.0), y, Inches(2.3), Inches(0.4),
             ticket, size=12, color=ACCENT, bold=True)
    add_text(slide, Inches(11.4), y, Inches(1.5), Inches(0.4),
             fit, size=14, color=ACCENT)

add_text(slide, Inches(0.5), Inches(6.5), Inches(12.3), Inches(0.4),
         "All prospects are within 3 hours of Lausanne. Local sales motion. Regulatory alignment.",
         size=12, color=LIGHT_GREY, align=PP_ALIGN.CENTER)

add_footer(slide, 5)

# ─────────────────────────────────────────────────────────────────────
#  Slide 6 — Where I am today (the honest status)
# ─────────────────────────────────────────────────────────────────────
slide = prs.slides.add_slide(prs.slide_layouts[6])
add_header_bar(slide)

add_text(slide, Inches(0.5), Inches(0.5), Inches(12), Inches(0.8),
         "Where I am today — honest status",
         size=36, bold=True, color=DARK_BLUE)
add_text(slide, Inches(0.5), Inches(1.3), Inches(12), Inches(0.6),
         "May 2026. Pre-revenue. Pre-design-partner. Product built.",
         size=18, color=LIGHT_GREY)

# Done (left)
add_text(slide, Inches(0.6), Inches(2.3), Inches(6), Inches(0.5),
         "Built", size=20, bold=True, color=ACCENT)
add_box(slide, Inches(0.6), Inches(2.9), Inches(5.9), Inches(3.7),
        RGBColor(0xe6, 0xf2, 0xeb))
done_text = """35 Rust crates, fully implemented

15 binaries running locally

3 layers : classification + meta-swarm + audit

Architecture documented (M14 to M30+)

Bench platform with quarterly publish pipeline
(PDF + JSON + cosign signature)

Compliance gating ready (DPIA, AI Act Article 9)

Demo reproducible end-to-end
(git clone + cargo build → working stack)

17 months of focused solo development"""
add_text(slide, Inches(0.8), Inches(3.0), Inches(5.5), Inches(3.5),
         done_text, size=12, color=BLACK)

# Not yet (right)
add_text(slide, Inches(7.0), Inches(2.3), Inches(6), Inches(0.5),
         "Not yet built", size=20, bold=True, color=WARNING)
add_box(slide, Inches(7.0), Inches(2.9), Inches(5.7), Inches(3.7),
        RGBColor(0xfa, 0xea, 0xe6))
not_yet = """First paying customer

First design partner contract signed

Technical co-founder

First 2-3 senior hires

Sales motion and customer development

SOC 2 / ISO 27001 certifications

Marketing site, demos, case studies

Pricing validated against real customers

A Series A story with traction proof"""
add_text(slide, Inches(7.2), Inches(3.0), Inches(5.3), Inches(3.5),
         not_yet, size=12, color=BLACK)

add_text(slide, Inches(0.5), Inches(6.8), Inches(12.3), Inches(0.4),
         "I've built the product. Now I'm building the company. That is what this round is for.",
         size=14, bold=True, color=DARK_BLUE, align=PP_ALIGN.CENTER)

add_footer(slide, 6)

# ─────────────────────────────────────────────────────────────────────
#  Slide 7 — Why now
# ─────────────────────────────────────────────────────────────────────
slide = prs.slides.add_slide(prs.slide_layouts[6])
add_header_bar(slide)

add_text(slide, Inches(0.5), Inches(0.5), Inches(12), Inches(0.8),
         "Why now", size=38, bold=True, color=DARK_BLUE)
add_text(slide, Inches(0.5), Inches(1.3), Inches(12), Inches(0.6),
         "The opportunity window is 2026-2027.",
         size=18, color=LIGHT_GREY)

catalysts = [
    ("EU AI Act enforcement",
     "2 August 2026",
     "All high-risk AI systems in the EU must be auditable and reproducible. "
     "Banks, insurers, pharma, government : legal obligation, not best practice."),
    ("Agentic AI explosion",
     "2024 to 2026",
     "Harvey AI raised USD 600 M. Sierra USD 4.5 B. Both vertical-narrow (legal, customer support). "
     "Horizontal infrastructure for regulated industries is open."),
    ("Swiss regulatory pull",
     "FINMA + Swissmedic + FDPIC",
     "DORA in effect since 17 January 2025. Solvency II + Swiss Solvency Test. GxP for pharma. "
     "All demanding by-construction audit trails for AI-driven decisions."),
]

for i, (title, when, desc) in enumerate(catalysts):
    y = Inches(2.3 + i * 1.4)
    add_box(slide, Inches(0.6), y, Inches(12.2), Inches(1.2),
            SOFT_BG, line_color=DARK_BLUE)
    add_text(slide, Inches(0.9), y + Inches(0.1), Inches(7), Inches(0.5),
             title, size=18, bold=True, color=DARK_BLUE)
    add_text(slide, Inches(9.0), y + Inches(0.1), Inches(3.5), Inches(0.5),
             when, size=15, bold=True, color=ACCENT, align=PP_ALIGN.RIGHT)
    add_text(slide, Inches(0.9), y + Inches(0.65), Inches(11.5), Inches(0.5),
             desc, size=12, color=BLACK)

add_text(slide, Inches(0.5), Inches(6.7), Inches(12.3), Inches(0.4),
         "The European infrastructure layer for regulated agentic AI is open. Whoever ships first wins.",
         size=14, bold=True, color=DARK_BLUE, align=PP_ALIGN.CENTER)

add_footer(slide, 7)

# ─────────────────────────────────────────────────────────────────────
#  Slide 8 — Competition
# ─────────────────────────────────────────────────────────────────────
slide = prs.slides.add_slide(prs.slide_layouts[6])
add_header_bar(slide)

add_text(slide, Inches(0.5), Inches(0.5), Inches(12), Inches(0.8),
         "Competitive landscape", size=36, bold=True, color=DARK_BLUE)
add_text(slide, Inches(0.5), Inches(1.3), Inches(12), Inches(0.6),
         "No competitor combines audit + federation + compliance in one stack.",
         size=18, color=LIGHT_GREY)

headers = ["", "Harvey AI", "LangChain", "AWS Bedrock", "EFE"]
col_widths = [3.4, 2.3, 2.3, 2.3, 2.3]
col_x = [0.5, 3.9, 6.2, 8.5, 10.8]

for i, h in enumerate(headers):
    y = Inches(2.3)
    is_efe = (i == 4)
    if i > 0:
        bg = ACCENT if is_efe else DARK_BLUE
        add_box(slide, Inches(col_x[i]), y, Inches(col_widths[i]), Inches(0.5),
                bg)
        add_text(slide, Inches(col_x[i]), y + Inches(0.07), Inches(col_widths[i]),
                 Inches(0.4), h, size=13, bold=True, color=BG_WHITE,
                 align=PP_ALIGN.CENTER)

axes = [
    ("By-construction audit trail (HMAC)",       "—",       "—",       "Partial", "Yes"),
    ("Multi-tenant kernel isolation",            "—",       "—",       "Partial", "Yes"),
    ("Federated execution (meta-swarm)",         "—",       "—",       "—",       "Yes"),
    ("DPIA + AI Act compliance gating",          "—",       "—",       "—",       "Yes"),
    ("LLM cost governance + circuit breakers",   "—",       "Partial", "Partial", "Yes"),
    ("Signed quarterly evidence (cosign)",       "—",       "—",       "—",       "Yes"),
    ("European-built + regulator-aligned",       "—",       "—",       "—",       "Yes"),
    ("Vertical scope",                            "Legal",   "Generic", "Generic", "Regulated horiz."),
]

for i, row in enumerate(axes):
    y = Inches(2.95 + i * 0.42)
    add_text(slide, Inches(col_x[0]), y, Inches(col_widths[0]), Inches(0.4),
             row[0], size=11, color=BLACK)
    for j in range(1, 5):
        val = row[j]
        if val == "Yes":
            color = ACCENT
        elif val in ("—",):
            color = LIGHT_GREY
        elif val in ("Partial",):
            color = WARNING
        else:
            color = BLACK
        is_bold = (val in ("Yes", "Partial"))
        add_text(slide, Inches(col_x[j]), y, Inches(col_widths[j]), Inches(0.4),
                 val, size=12, bold=is_bold, color=color, align=PP_ALIGN.CENTER)

add_text(slide, Inches(0.5), Inches(6.4), Inches(12.3), Inches(0.4),
         "Harvey is vertical-narrow. LangChain is OSS without compliance.",
         size=12, color=LIGHT_GREY, align=PP_ALIGN.CENTER)
add_text(slide, Inches(0.5), Inches(6.75), Inches(12.3), Inches(0.4),
         "Bedrock is cloud-locked without audit trail. EFE is the regulated-horizontal layer that does not exist yet.",
         size=12, bold=True, color=DARK_BLUE, align=PP_ALIGN.CENTER)

add_footer(slide, 8)

# ─────────────────────────────────────────────────────────────────────
#  Slide 9 — Business model
# ─────────────────────────────────────────────────────────────────────
slide = prs.slides.add_slide(prs.slide_layouts[6])
add_header_bar(slide)

add_text(slide, Inches(0.5), Inches(0.5), Inches(12), Inches(0.8),
         "Business model", size=38, bold=True, color=DARK_BLUE)
add_text(slide, Inches(0.5), Inches(1.3), Inches(12), Inches(0.6),
         "B2B subscription. Three tiers. Per-tenant pricing.",
         size=18, color=LIGHT_GREY)

tiers = [
    ("Design Partner",
     "EUR 100-200 k/year",
     "First 3-5 customers. Co-build the customer-specific agent integration. "
     "12-month engagement. Quarterly business review.",
     RGBColor(0xe6, 0xf2, 0xeb)),
    ("Enterprise",
     "EUR 500 k-1.5 M/year",
     "Multi-tenant, dedicated VPC, custom SLA, full audit trail. "
     "Banking + pharma flagship contracts.",
     RGBColor(0xe6, 0xee, 0xfa)),
    ("Self-hosted / air-gapped",
     "EUR 200-400 k/year + setup",
     "Full on-premise deployment. Sovereign customers (defense, gov). "
     "Higher margin, longer sales cycle.",
     RGBColor(0xf5, 0xf5, 0xfa)),
]

for i, (name, price, desc, bg) in enumerate(tiers):
    y = Inches(2.3 + i * 1.4)
    add_box(slide, Inches(0.5), y, Inches(12.3), Inches(1.2),
            bg, line_color=DARK_BLUE)
    add_text(slide, Inches(0.8), y + Inches(0.15), Inches(4.5), Inches(0.5),
             name, size=20, bold=True, color=DARK_BLUE)
    add_text(slide, Inches(8), y + Inches(0.15), Inches(4.5), Inches(0.5),
             price, size=18, bold=True, color=ACCENT, align=PP_ALIGN.RIGHT)
    add_text(slide, Inches(0.8), y + Inches(0.7), Inches(11.7), Inches(0.5),
             desc, size=13, color=BLACK)

add_text(slide, Inches(0.5), Inches(6.7), Inches(12.3), Inches(0.4),
         "18-month target : 3 design partners + 1-2 enterprise = EUR 1.5-3 M ARR run-rate.",
         size=14, bold=True, color=DARK_BLUE, align=PP_ALIGN.CENTER)

add_footer(slide, 9)

# ─────────────────────────────────────────────────────────────────────
#  Slide 10 — Solo founder
# ─────────────────────────────────────────────────────────────────────
slide = prs.slides.add_slide(prs.slide_layouts[6])
add_header_bar(slide)

add_text(slide, Inches(0.5), Inches(0.5), Inches(12), Inches(0.8),
         "Solo founder — by choice, not necessity",
         size=32, bold=True, color=DARK_BLUE)
add_text(slide, Inches(0.5), Inches(1.3), Inches(12), Inches(0.6),
         "Built the product alone. Now building the team.",
         size=18, color=LIGHT_GREY)

# Left : me
add_box(slide, Inches(0.5), Inches(2.2), Inches(6), Inches(4.4),
        SOFT_BG, line_color=DARK_BLUE)
add_text(slide, Inches(0.7), Inches(2.35), Inches(5.7), Inches(0.5),
         "Rodrigue Pekar — Founder",
         size=20, bold=True, color=DARK_BLUE)
add_text(slide, Inches(0.7), Inches(2.85), Inches(5.7), Inches(0.4),
         "EPFL Lausanne",
         size=12, color=LIGHT_GREY)

bio = """17 months of focused work on EFE :

  • Architected 35 Rust crates from scratch
  • Implemented all 3 layers
  • Designed the meta-swarm L2 protocol
  • Built the compliance + audit infrastructure
  • Wrote the bench-platform end-to-end

Background : EPFL engineering, full-stack
Rust + distributed systems + ML ops.

What I optimize for : technical depth
and audit-defensibility. Not speed of
shipping demos."""
add_text(slide, Inches(0.7), Inches(3.3), Inches(5.6), Inches(3.2),
         bio, size=12, color=BLACK)

# Right : what I'm looking for
add_text(slide, Inches(7), Inches(2.2), Inches(5.8), Inches(0.5),
         "What I'm looking for (in this order)",
         size=18, bold=True, color=ACCENT)

asks = [
    ("1.", "First design partner", "Pharma or finance, willing to co-build. Roche, Pictet, Kudelski are candidates."),
    ("2.", "Co-founder candidate", "Business + sales background. Regulated industry network preferred."),
    ("3.", "Lead investor", "Anchor pre-seed round. Strategic introductions to Swiss enterprise."),
    ("4.", "First senior hire", "Platform engineer. Joins after first design partner signs."),
]

for i, (n, title, desc) in enumerate(asks):
    y = Inches(2.85 + i * 0.95)
    add_text(slide, Inches(7), y, Inches(0.5), Inches(0.4),
             n, size=18, bold=True, color=ACCENT)
    add_text(slide, Inches(7.5), y, Inches(5.3), Inches(0.4),
             title, size=14, bold=True, color=DARK_BLUE)
    add_text(slide, Inches(7.5), y + Inches(0.35), Inches(5.3), Inches(0.55),
             desc, size=11, color=LIGHT_GREY)

add_footer(slide, 10)

# ─────────────────────────────────────────────────────────────────────
#  Slide 11 — Ask
# ─────────────────────────────────────────────────────────────────────
slide = prs.slides.add_slide(prs.slide_layouts[6])
add_header_bar(slide)

add_text(slide, Inches(0.5), Inches(0.5), Inches(12), Inches(0.8),
         "Pre-seed ask", size=38, bold=True, color=DARK_BLUE)
add_text(slide, Inches(0.5), Inches(1.3), Inches(12), Inches(0.6),
         "Anchor round to convert technical product into customer traction.",
         size=18, color=LIGHT_GREY)

# Box gauche : amount + valuation
add_box(slide, Inches(0.6), Inches(2.3), Inches(5.9), Inches(2.0),
        RGBColor(0xe6, 0xee, 0xfa), line_color=DARK_BLUE)
add_text(slide, Inches(0.9), Inches(2.5), Inches(5.5), Inches(0.4),
         "Round size", size=14, color=LIGHT_GREY)
add_text(slide, Inches(0.9), Inches(2.85), Inches(5.5), Inches(0.5),
         "CHF 500 k — 1 M",
         size=28, bold=True, color=DARK_BLUE)
add_text(slide, Inches(0.9), Inches(3.6), Inches(5.5), Inches(0.4),
         "Valuation : CHF 4-6 M post-money",
         size=14, color=BLACK)

# Box droite : runway + lead
add_box(slide, Inches(6.7), Inches(2.3), Inches(6.1), Inches(2.0),
        RGBColor(0xe6, 0xf2, 0xeb), line_color=DARK_BLUE)
add_text(slide, Inches(7.0), Inches(2.5), Inches(5.5), Inches(0.4),
         "Runway", size=14, color=LIGHT_GREY)
add_text(slide, Inches(7.0), Inches(2.85), Inches(5.5), Inches(0.5),
         "18 months",
         size=28, bold=True, color=ACCENT)
add_text(slide, Inches(7.0), Inches(3.6), Inches(5.5), Inches(0.4),
         "Lead investor : Kovant (proposed)",
         size=14, color=BLACK)

# Use of funds
add_text(slide, Inches(0.5), Inches(4.5), Inches(12), Inches(0.5),
         "Use of funds",
         size=18, bold=True, color=DARK_BLUE)

uses = [
    ("50 %", "Engineering",   "Technical co-founder + first senior hire"),
    ("30 %", "Sales + customer dev", "Design partner outreach, regulated-industry pilots"),
    ("20 %", "Compliance",     "SOC 2 Type I prep, legal, ISO 27001 readiness"),
]

for i, (pct, cat, desc) in enumerate(uses):
    y = Inches(5.05 + i * 0.45)
    add_text(slide, Inches(0.7), y, Inches(1.0), Inches(0.4),
             pct, size=16, bold=True, color=ACCENT)
    add_text(slide, Inches(1.8), y, Inches(3.5), Inches(0.4),
             cat, size=14, bold=True, color=BLACK)
    add_text(slide, Inches(5.5), y, Inches(7.3), Inches(0.4),
             desc, size=12, color=LIGHT_GREY)

# Milestones
add_text(slide, Inches(0.5), Inches(6.55), Inches(12), Inches(0.4),
         "18-month milestones : 3 design partners signed, EUR 1.5-3 M ARR, Series A conversation opened.",
         size=12, bold=True, color=DARK_BLUE, align=PP_ALIGN.CENTER)

add_footer(slide, 11)

# ─────────────────────────────────────────────────────────────────────
#  Slide 12 — Vision / close
# ─────────────────────────────────────────────────────────────────────
slide = prs.slides.add_slide(prs.slide_layouts[6])

top_bar = slide.shapes.add_shape(MSO_SHAPE.RECTANGLE,
                                  Inches(0), Inches(0), SW, Inches(2.0))
top_bar.fill.solid()
top_bar.fill.fore_color.rgb = DARK_BLUE
top_bar.line.fill.background()

add_text(slide, Inches(0.8), Inches(0.6), Inches(11), Inches(0.6),
         "Vision", size=42, bold=True, color=BG_WHITE)
add_text(slide, Inches(0.8), Inches(1.3), Inches(11), Inches(0.5),
         "Audit by construction. Federation by design. European-built.",
         size=18, color=BG_WHITE)

add_text(slide, Inches(0.8), Inches(2.6), Inches(11.7), Inches(2.7),
         '"In five years, deploying AI in regulated industries\n'
         'without by-construction audit and multi-tenant isolation\n'
         'will not pass EU AI Act compliance.\n\n'
         'EFE is positioned to be the European standard\n'
         'infrastructure layer for that future."',
         size=22, color=BLACK, anchor=MSO_ANCHOR.MIDDLE,
         align=PP_ALIGN.CENTER)

add_box(slide, Inches(2.0), Inches(5.6), Inches(9.3), Inches(1.4),
        SOFT_BG, line_color=DARK_BLUE)
add_text(slide, Inches(2.2), Inches(5.75), Inches(9), Inches(0.4),
         "Rodrigue Pekar — rodrigue.pekar@icloud.com",
         size=15, bold=True, color=DARK_BLUE, align=PP_ALIGN.CENTER)
add_text(slide, Inches(2.2), Inches(6.15), Inches(9), Inches(0.4),
         "EFE — Federated AI infrastructure for regulated industries",
         size=12, color=BLACK, align=PP_ALIGN.CENTER)
add_text(slide, Inches(2.2), Inches(6.55), Inches(9), Inches(0.4),
         "Thank you. Questions ?",
         size=14, bold=True, color=ACCENT, align=PP_ALIGN.CENTER)

add_footer(slide, 12)

# ─────────────────────────────────────────────────────────────────────
output = "/Users/rodrigue.pekar/Downloads/jjjj/efe/docs/marketing/EFE-Kovant-Pitch-v2-2026-05-21.pptx"
prs.save(output)
print(f"Saved : {output}")
print(f"Slides : {len(prs.slides)}")
