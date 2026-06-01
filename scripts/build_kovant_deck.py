#!/usr/bin/env python3.12
# ─────────────────────────────────────────────────────────────────────
#  build_kovant_deck.py
#
#  Construit le pitch deck Kovant — 12 slides PowerPoint (.pptx)
#
#  Sortie : EFE-Kovant-Pitch-2026-05-21.pptx
#
#  Design : minimaliste, professional, audit-grade tone
#    - Police : Calibri (par défaut)
#    - Couleurs : noir + bleu sombre + accent vert audit-grade
#    - Pas d'emoji ni de fluff
#    - Numbers when possible
#
#  Slides :
#    1.  Title
#    2.  Problem
#    3.  Solution
#    4.  How : 7 layers structural pinning
#    5.  Why Now : EU AI Act + agentic explosion
#    6.  Market sectors (Swiss focus)
#    7.  Competition
#    8.  Proof : 174 tests live
#    9.  Business model
#    10. Traction + design partner targets
#    11. Team + Ask
#    12. Vision / close
# ─────────────────────────────────────────────────────────────────────

from pptx import Presentation
from pptx.util import Inches, Pt, Emu
from pptx.dml.color import RGBColor
from pptx.enum.text import PP_ALIGN, MSO_ANCHOR
from pptx.enum.shapes import MSO_SHAPE

# Couleurs (palette audit-grade)
BLACK = RGBColor(0x14, 0x14, 0x14)
DARK_BLUE = RGBColor(0x0a, 0x2e, 0x5c)
ACCENT = RGBColor(0x2d, 0x8a, 0x5c)  # vert "audit-grade"
WARNING = RGBColor(0xc4, 0x4d, 0x3b)  # rouge "régressé"
LIGHT_GREY = RGBColor(0x6c, 0x6c, 0x6c)
BG_WHITE = RGBColor(0xff, 0xff, 0xff)

prs = Presentation()
prs.slide_width = Inches(13.333)
prs.slide_height = Inches(7.5)
SW = prs.slide_width
SH = prs.slide_height


def add_text(slide, left, top, width, height, text, *, size=18,
             bold=False, color=BLACK, align=PP_ALIGN.LEFT, anchor=MSO_ANCHOR.TOP,
             font="Calibri"):
    box = slide.shapes.add_textbox(left, top, width, height)
    tf = box.text_frame
    tf.word_wrap = True
    tf.vertical_anchor = anchor
    paragraphs = text.split("\n")
    for i, line in enumerate(paragraphs):
        if i == 0:
            p = tf.paragraphs[0]
        else:
            p = tf.add_paragraph()
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


def add_footer(slide, slide_num, total=12):
    add_text(slide, Inches(0.4), SH - Inches(0.4), Inches(8), Inches(0.3),
             "EFE — Kovant Pitch — 2026-05-21",
             size=9, color=LIGHT_GREY)
    add_text(slide, SW - Inches(1.2), SH - Inches(0.4), Inches(0.8), Inches(0.3),
             f"{slide_num}/{total}",
             size=9, color=LIGHT_GREY, align=PP_ALIGN.RIGHT)


def add_header_bar(slide):
    """Barre de top minimaliste sur les content slides."""
    bar = slide.shapes.add_shape(MSO_SHAPE.RECTANGLE,
                                  Inches(0), Inches(0), SW, Inches(0.08))
    bar.fill.solid()
    bar.fill.fore_color.rgb = DARK_BLUE
    bar.line.fill.background()


# ─────────────────────────────────────────────────────────────────────
#  Slide 1 — Title
# ─────────────────────────────────────────────────────────────────────
slide = prs.slides.add_slide(prs.slide_layouts[6])  # blank

# Bandeau couleur supérieur
top_bar = slide.shapes.add_shape(MSO_SHAPE.RECTANGLE,
                                  Inches(0), Inches(0), SW, Inches(2.0))
top_bar.fill.solid()
top_bar.fill.fore_color.rgb = DARK_BLUE
top_bar.line.fill.background()

add_text(slide, Inches(0.8), Inches(0.4), Inches(11), Inches(0.5),
         "EFE", size=48, bold=True, color=BG_WHITE)
add_text(slide, Inches(0.8), Inches(1.2), Inches(11), Inches(0.6),
         "Audit-grade infrastructure for production agentic AI",
         size=22, color=BG_WHITE)

# Main tagline
add_text(slide, Inches(0.8), Inches(2.7), Inches(11), Inches(0.8),
         "12 mois de platform engineering en plug-and-play.",
         size=32, bold=True, color=BLACK)

# Sub-tagline
add_text(slide, Inches(0.8), Inches(3.6), Inches(11), Inches(0.6),
         "Pour les industries régulées : pharma, finance, healthcare, legal, cybersec.",
         size=18, color=LIGHT_GREY)

# Speaker block
add_box(slide, Inches(0.8), Inches(5.5), Inches(5.5), Inches(1.3),
        BG_WHITE, line_color=DARK_BLUE)
add_text(slide, Inches(1.0), Inches(5.6), Inches(5.3), Inches(0.4),
         "Rodrigue Pekar — Founding Engineer",
         size=14, bold=True, color=DARK_BLUE)
add_text(slide, Inches(1.0), Inches(6.0), Inches(5.3), Inches(0.4),
         "EPFL Lausanne",
         size=12, color=BLACK)
add_text(slide, Inches(1.0), Inches(6.4), Inches(5.3), Inches(0.4),
         "Pitch Kovant — Jeudi 21 mai 2026 — 14:00 CEST",
         size=12, color=BLACK)

add_footer(slide, 1)

# ─────────────────────────────────────────────────────────────────────
#  Slide 2 — Problem
# ─────────────────────────────────────────────────────────────────────
slide = prs.slides.add_slide(prs.slide_layouts[6])
add_header_bar(slide)

add_text(slide, Inches(0.5), Inches(0.5), Inches(12), Inches(0.8),
         "Le problème", size=36, bold=True, color=DARK_BLUE)
add_text(slide, Inches(0.5), Inches(1.3), Inches(12), Inches(0.6),
         "Déployer un agent IA en production dans une industrie régulée demande 6-12 mois.",
         size=18, color=LIGHT_GREY)

# Liste des 9 briques à construire
add_text(slide, Inches(0.7), Inches(2.4), Inches(12), Inches(0.4),
         "Ce qu'une équipe doit construire elle-même :",
         size=14, bold=True, color=BLACK)

items = [
    ("LLM router multi-provider", "2-4 sem."),
    ("Cost cap + circuit breaker", "2-3 sem."),
    ("Cache embeddings + LLM", "2 sem."),
    ("Sandbox isolation (Firecracker)", "4-8 sem."),
    ("Step approval workflow", "2-4 sem."),
    ("Audit trail HMAC chain", "3-4 sem."),
    ("Prompts versioning + A/B", "1-2 sem."),
    ("DPIA + AI Act compliance gating", "4-12 sem."),
    ("Active learning closed-loop", "8-12 sem."),
]

for i, (item, weeks) in enumerate(items):
    y = Inches(2.9 + i * 0.35)
    add_text(slide, Inches(0.9), y, Inches(7.5), Inches(0.3),
             f"• {item}", size=13, color=BLACK)
    add_text(slide, Inches(8.5), y, Inches(2.5), Inches(0.3),
             weeks, size=13, color=LIGHT_GREY, align=PP_ALIGN.RIGHT)

# Total
add_box(slide, Inches(0.7), Inches(6.3), Inches(11.5), Inches(0.7),
        DARK_BLUE)
add_text(slide, Inches(0.9), Inches(6.4), Inches(11), Inches(0.5),
         "TOTAL : 28-51 semaines de platform engineering — refait par chaque entreprise.",
         size=16, bold=True, color=BG_WHITE)

add_footer(slide, 2)

# ─────────────────────────────────────────────────────────────────────
#  Slide 3 — Solution
# ─────────────────────────────────────────────────────────────────────
slide = prs.slides.add_slide(prs.slide_layouts[6])
add_header_bar(slide)

add_text(slide, Inches(0.5), Inches(0.5), Inches(12), Inches(0.8),
         "La solution", size=36, bold=True, color=DARK_BLUE)
add_text(slide, Inches(0.5), Inches(1.3), Inches(12), Inches(0.6),
         "EFE est ces 12 mois en plug-and-play.",
         size=18, color=LIGHT_GREY)

# 2 colonnes : sans / avec EFE
add_text(slide, Inches(0.7), Inches(2.3), Inches(5.8), Inches(0.4),
         "Sans EFE", size=18, bold=True, color=WARNING)
add_text(slide, Inches(7.0), Inches(2.3), Inches(5.8), Inches(0.4),
         "Avec EFE", size=18, bold=True, color=ACCENT)

# Box sans EFE
add_box(slide, Inches(0.7), Inches(2.8), Inches(5.8), Inches(3.8),
        RGBColor(0xfa, 0xea, 0xe6))
sans_efe = """28-51 semaines

Engineering coût : ~ 400-650 k€

• LLM router à coder
• Sandbox à concevoir
• Audit trail HMAC à dériver
• Compliance gating à implémenter
• Bench framework à construire
• MCP server à exposer
• Cost cap à intégrer

Risque : chaque équipe réinvente la roue."""
add_text(slide, Inches(0.9), Inches(2.95), Inches(5.4), Inches(3.6),
         sans_efe, size=13, color=BLACK)

# Box avec EFE
add_box(slide, Inches(7.0), Inches(2.8), Inches(5.8), Inches(3.8),
        RGBColor(0xe6, 0xf2, 0xeb))
avec_efe = """2-4 semaines d'intégration

License EFE plug-and-play

• LLM router : M21 — done
• Sandbox Firecracker : M25 — done
• Audit trail HMAC : M16/M23 — done
• Compliance DPIA + AI Act : M25 — done
• Bench-platform : PR #27 — done
• MCP server : M16 — done
• Cost cap + circuit breaker : M21 — done

Garantie : 174 tests prouvent que EFE est lui-même audit-grade."""
add_text(slide, Inches(7.2), Inches(2.95), Inches(5.4), Inches(3.6),
         avec_efe, size=13, color=BLACK)

# ROI inline
add_text(slide, Inches(0.7), Inches(6.8), Inches(11), Inches(0.4),
         "→ ROI : ~ 50-200x — l'agent customer-spécifique demande 2-4 jours au lieu de 28-51 semaines.",
         size=14, bold=True, color=DARK_BLUE)

add_footer(slide, 3)

# ─────────────────────────────────────────────────────────────────────
#  Slide 4 — How : 7 layers structural pinning
# ─────────────────────────────────────────────────────────────────────
slide = prs.slides.add_slide(prs.slide_layouts[6])
add_header_bar(slide)

add_text(slide, Inches(0.5), Inches(0.5), Inches(12), Inches(0.8),
         "Le moat technique : structurellement non-régressable",
         size=32, bold=True, color=DARK_BLUE)
add_text(slide, Inches(0.5), Inches(1.3), Inches(12), Inches(0.6),
         "Si quelqu'un casse EFE, le build CI rouge immédiatement.",
         size=18, color=LIGHT_GREY)

layers = [
    ("Unit tests (effective_tolerance lib)", "45 tests", "taxonomies + resolver + verdict matrix"),
    ("Pinning tests (g11_round_trip)", "9 tests", "déterminisme byte-identical 5x"),
    ("Tolerance-used populated tests", "7 tests", "épingle G.11.a/b/c/d"),
    ("Freeze tests (STABILITY v1.7)", "51 tests", "contrat gelé : taxonomies + sections"),
    ("Backfill edge cases", "14 tests", "byte-identical contre golden"),
    ("Workflow YAML invariants", "18 tests", "CI workflow pinné"),
    ("Audit chain meta-tests (S3 + S9-S12)", "30 tests", "documentation testée comme du code"),
]

for i, (layer, count, desc) in enumerate(layers):
    y = Inches(2.2 + i * 0.55)
    # Numéro
    add_box(slide, Inches(0.5), y, Inches(0.5), Inches(0.45), DARK_BLUE)
    add_text(slide, Inches(0.5), y + Inches(0.05), Inches(0.5), Inches(0.4),
             str(i + 1), size=14, bold=True, color=BG_WHITE,
             align=PP_ALIGN.CENTER)
    # Layer name
    add_text(slide, Inches(1.2), y, Inches(5.5), Inches(0.4),
             layer, size=14, bold=True, color=BLACK)
    # Count
    add_text(slide, Inches(6.8), y, Inches(1.5), Inches(0.4),
             count, size=14, color=ACCENT, bold=True)
    # Description
    add_text(slide, Inches(8.4), y, Inches(4.5), Inches(0.4),
             desc, size=12, color=LIGHT_GREY)

# Total bar
add_box(slide, Inches(0.5), Inches(6.4), Inches(12.3), Inches(0.6),
        ACCENT)
add_text(slide, Inches(0.7), Inches(6.5), Inches(12), Inches(0.4),
         "TOTAL : 174 tests permanents et non-régressables.  PR #27 clos avec triple sign-off.",
         size=16, bold=True, color=BG_WHITE)

add_footer(slide, 4)

# ─────────────────────────────────────────────────────────────────────
#  Slide 5 — Why Now
# ─────────────────────────────────────────────────────────────────────
slide = prs.slides.add_slide(prs.slide_layouts[6])
add_header_bar(slide)

add_text(slide, Inches(0.5), Inches(0.5), Inches(12), Inches(0.8),
         "Pourquoi maintenant", size=36, bold=True, color=DARK_BLUE)
add_text(slide, Inches(0.5), Inches(1.3), Inches(12), Inches(0.6),
         "La fenêtre d'opportunité est 2026-2027.",
         size=18, color=LIGHT_GREY)

# 3 catalyseurs
catalysts = [
    ("EU AI Act enforcement",
     "2026-08-02",
     "Toute IA high-risk en UE doit être auditable.  Banks, insurers, pharma, gov : obligation."),
    ("Agentic AI explosion",
     "2024 → 2026",
     "Harvey AI : 600 M$. Sierra : 4.5 Md$. Mais bloqués sur compliance pour les régulés."),
    ("Swiss regulatory pull",
     "FINMA + Swissmedic",
     "DORA depuis 17-01-2025. Solvency II + Swiss Solvency Test. GxP. Tous demandent audit-trail."),
]

for i, (title, when, desc) in enumerate(catalysts):
    y = Inches(2.4 + i * 1.4)
    # Card background
    add_box(slide, Inches(0.6), y, Inches(12.2), Inches(1.2),
            RGBColor(0xf5, 0xf7, 0xfa), line_color=DARK_BLUE)
    add_text(slide, Inches(0.9), y + Inches(0.1), Inches(7), Inches(0.5),
             title, size=18, bold=True, color=DARK_BLUE)
    add_text(slide, Inches(9.0), y + Inches(0.1), Inches(3.5), Inches(0.5),
             when, size=16, bold=True, color=ACCENT, align=PP_ALIGN.RIGHT)
    add_text(slide, Inches(0.9), y + Inches(0.65), Inches(11.5), Inches(0.5),
             desc, size=13, color=BLACK)

# Bottom message
add_text(slide, Inches(0.5), Inches(6.7), Inches(12.3), Inches(0.4),
         "→ La fenêtre concurrentielle pour devenir l'infrastructure standard est étroite.",
         size=14, bold=True, color=DARK_BLUE, align=PP_ALIGN.CENTER)

add_footer(slide, 5)

# ─────────────────────────────────────────────────────────────────────
#  Slide 6 — Market sectors (Swiss focus)
# ─────────────────────────────────────────────────────────────────────
slide = prs.slides.add_slide(prs.slide_layouts[6])
add_header_bar(slide)

add_text(slide, Inches(0.5), Inches(0.5), Inches(12), Inches(0.8),
         "Marché cible — focus Suisse + EU régulé",
         size=32, bold=True, color=DARK_BLUE)
add_text(slide, Inches(0.5), Inches(1.3), Inches(12), Inches(0.6),
         "5 secteurs prioritaires.  Tickets 200 k € — 2 M €/an.",
         size=18, color=LIGHT_GREY)

sectors = [
    ("Pharma R&D",         "Roche, Novartis, Lonza, Galderma",            "1-2 M €/an", "★★★★★"),
    ("Hedge funds / quant","Pictet, Lombard Odier, BlueOrchard",          "200 k-1 M €/an", "★★★★★"),
    ("Cybersécurité MSSP", "Kudelski, Exeon, Eviden",                     "200-600 k €/an", "★★★★★"),
    ("Big 4 consulting",   "KPMG, PwC, Deloitte, EY (Suisse)",            "500 k-1.5 M €/an", "★★★★"),
    ("Legal RegTech",      "Apiax, Eraneos, Luminance Suisse",            "200-800 k €/an", "★★★★"),
    ("Insurance claims",   "Zurich, Helvetia, Vaudoise, Mobilière",       "200 k-1 M €/an", "★★★★"),
]

# Header
add_text(slide, Inches(0.6), Inches(2.3), Inches(2.8), Inches(0.3),
         "Secteur", size=11, bold=True, color=LIGHT_GREY)
add_text(slide, Inches(3.5), Inches(2.3), Inches(5.5), Inches(0.3),
         "Prospects Suisses cibles", size=11, bold=True, color=LIGHT_GREY)
add_text(slide, Inches(9.2), Inches(2.3), Inches(2.0), Inches(0.3),
         "Ticket annuel", size=11, bold=True, color=LIGHT_GREY)
add_text(slide, Inches(11.4), Inches(2.3), Inches(1.5), Inches(0.3),
         "Fit", size=11, bold=True, color=LIGHT_GREY)

for i, (sector, prospects, ticket, fit) in enumerate(sectors):
    y = Inches(2.7 + i * 0.55)
    # Alternating row background
    if i % 2 == 0:
        add_box(slide, Inches(0.4), y - Inches(0.05), Inches(12.5), Inches(0.55),
                RGBColor(0xf8, 0xf9, 0xfa))
    add_text(slide, Inches(0.6), y, Inches(2.8), Inches(0.4),
             sector, size=13, bold=True, color=BLACK)
    add_text(slide, Inches(3.5), y, Inches(5.5), Inches(0.4),
             prospects, size=11, color=LIGHT_GREY)
    add_text(slide, Inches(9.2), y, Inches(2.0), Inches(0.4),
             ticket, size=12, color=ACCENT, bold=True)
    add_text(slide, Inches(11.4), y, Inches(1.5), Inches(0.4),
             fit, size=14, color=ACCENT)

# Geographic note
add_text(slide, Inches(0.5), Inches(6.5), Inches(12.3), Inches(0.4),
         "Tous ces prospects sont à < 3h de Lausanne.  Cycle de vente local + culturel + régulatoire.",
         size=12, color=LIGHT_GREY, align=PP_ALIGN.CENTER)

add_footer(slide, 6)

# ─────────────────────────────────────────────────────────────────────
#  Slide 7 — Competition
# ─────────────────────────────────────────────────────────────────────
slide = prs.slides.add_slide(prs.slide_layouts[6])
add_header_bar(slide)

add_text(slide, Inches(0.5), Inches(0.5), Inches(12), Inches(0.8),
         "Concurrence", size=36, bold=True, color=DARK_BLUE)
add_text(slide, Inches(0.5), Inches(1.3), Inches(12), Inches(0.6),
         "Aucun concurrent ne combine les 4 axes critiques.",
         size=18, color=LIGHT_GREY)

# Table en 5 colonnes
headers = ["", "Harvey AI", "LangChain", "AWS Bedrock", "EFE"]
col_widths = [3.4, 2.3, 2.3, 2.3, 2.3]
col_x = [0.5, 3.9, 6.2, 8.5, 10.8]

# Header row
for i, h in enumerate(headers):
    y = Inches(2.3)
    is_efe = (i == 4)
    bg_color = ACCENT if is_efe else DARK_BLUE
    if i > 0:
        add_box(slide, Inches(col_x[i]), y, Inches(col_widths[i]), Inches(0.5),
                bg_color)
        add_text(slide, Inches(col_x[i]), y + Inches(0.07), Inches(col_widths[i]),
                 Inches(0.4), h, size=13, bold=True, color=BG_WHITE,
                 align=PP_ALIGN.CENTER)

axes = [
    ("Audit-grade trail (HMAC + signed)",   "—", "—", "Partiel", "✓"),
    ("Multi-tenant isolation (Firecracker)", "—", "—", "Partiel", "✓"),
    ("DPIA + AI Act compliance gating",      "—", "—", "—",       "✓"),
    ("Cost governance + circuit breakers",   "—", "Partiel", "Partiel", "✓"),
    ("Bench platform + regression detect",   "—", "—", "—",       "✓"),
    ("Closed-loop active learning",          "—", "Partiel", "✓",       "✓"),
    ("Plug-and-play for regulated industries","—","—","—",         "✓"),
]

for i, row in enumerate(axes):
    y = Inches(2.95 + i * 0.45)
    # First column (axis)
    add_text(slide, Inches(col_x[0]), y, Inches(col_widths[0]), Inches(0.4),
             row[0], size=12, color=BLACK)
    # Cells
    for j in range(1, 5):
        val = row[j]
        color = ACCENT if val == "✓" else (LIGHT_GREY if val == "—" else WARNING)
        add_text(slide, Inches(col_x[j]), y, Inches(col_widths[j]), Inches(0.4),
                 val, size=14, bold=True, color=color, align=PP_ALIGN.CENTER)

# Differentiator note
add_text(slide, Inches(0.5), Inches(6.5), Inches(12.3), Inches(0.4),
         "→ EFE est le seul à combiner audit + multi-tenant + compliance + cost + bench dans une seule infra.",
         size=13, bold=True, color=DARK_BLUE, align=PP_ALIGN.CENTER)

add_footer(slide, 7)

# ─────────────────────────────────────────────────────────────────────
#  Slide 8 — Proof : 174 tests live
# ─────────────────────────────────────────────────────────────────────
slide = prs.slides.add_slide(prs.slide_layouts[6])
add_header_bar(slide)

add_text(slide, Inches(0.5), Inches(0.5), Inches(12), Inches(0.8),
         "La preuve : 174 tests live, reproductibles",
         size=32, bold=True, color=DARK_BLUE)
add_text(slide, Inches(0.5), Inches(1.3), Inches(12), Inches(0.6),
         "Eat-our-own-dogfood : EFE est lui-même audit-grade.",
         size=18, color=LIGHT_GREY)

# Code-style box pour montrer le test result
add_box(slide, Inches(0.5), Inches(2.2), Inches(12.3), Inches(3.6),
        RGBColor(0x1e, 0x1e, 0x1e))
code_text = """$ bash scripts/efe-exp4-true-efe-run.sh

  ✓ g11_round_trip                           : 9 passed
  ✓ g11_tolerance_used_populated             : 7 passed
  ✓ stability_contract_v17                   : 51 passed
  ✓ backfill_edge_cases                      : 14 passed
  ✓ pr27_workflow_yaml_invariants            : 18 passed
  ✓ s3_audit_chain_integrity                 : 14 passed
  ✓ s9_s12_monitoring_chain_integrity        : 16 passed
  ✓ + lib::compare::effective_tolerance      : 45 passed
  ─────────────────────────────────────────────
  TOTAL : 174 tests passed, 0 failed → release gate GO"""
add_text(slide, Inches(0.8), Inches(2.4), Inches(12), Inches(3.4),
         code_text, size=12, color=RGBColor(0xa0, 0xff, 0xa0),
         font="Menlo")

# Wow moment cue
add_box(slide, Inches(0.5), Inches(6.1), Inches(12.3), Inches(0.9),
        RGBColor(0xfe, 0xf3, 0xc7))
add_text(slide, Inches(0.7), Inches(6.2), Inches(12), Inches(0.4),
         "Live demo (en option, 90 secondes) : modifier 1 caractère dans un golden file →",
         size=12, bold=True, color=BLACK)
add_text(slide, Inches(0.7), Inches(6.55), Inches(12), Inches(0.4),
         "EFE détecte instantanément (test FAILED).  Restaurer → test PASS à nouveau.",
         size=12, color=BLACK)

add_footer(slide, 8)

# ─────────────────────────────────────────────────────────────────────
#  Slide 9 — Business model
# ─────────────────────────────────────────────────────────────────────
slide = prs.slides.add_slide(prs.slide_layouts[6])
add_header_bar(slide)

add_text(slide, Inches(0.5), Inches(0.5), Inches(12), Inches(0.8),
         "Business model", size=36, bold=True, color=DARK_BLUE)
add_text(slide, Inches(0.5), Inches(1.3), Inches(12), Inches(0.6),
         "Subscription B2B par tenant, 3 tiers.",
         size=18, color=LIGHT_GREY)

tiers = [
    ("Design Partner",
     "100-200 k €/an",
     "Premiers 3-5 clients.  Co-construction de l'agent intégration.  Engagement 12 mois.",
     RGBColor(0xe6, 0xf2, 0xeb)),
    ("Enterprise",
     "500 k-1.5 M €/an",
     "Multi-tenant, dedicated VPC, custom SLA.  Banking + pharma cible.",
     RGBColor(0xe6, 0xee, 0xfa)),
    ("Self-hosted",
     "200-400 k €/an + setup",
     "Air-gapped, full on-prem.  Defense + sovereign customers.",
     RGBColor(0xf5, 0xf5, 0xfa)),
]

for i, (name, price, desc, bg) in enumerate(tiers):
    y = Inches(2.3 + i * 1.4)
    add_box(slide, Inches(0.5), y, Inches(12.3), Inches(1.2),
            bg, line_color=DARK_BLUE)
    add_text(slide, Inches(0.8), y + Inches(0.15), Inches(4), Inches(0.5),
             name, size=20, bold=True, color=DARK_BLUE)
    add_text(slide, Inches(8.5), y + Inches(0.15), Inches(4.0), Inches(0.5),
             price, size=20, bold=True, color=ACCENT, align=PP_ALIGN.RIGHT)
    add_text(slide, Inches(0.8), y + Inches(0.7), Inches(11.7), Inches(0.5),
             desc, size=13, color=BLACK)

# Cible LTV
add_text(slide, Inches(0.5), Inches(6.7), Inches(12.3), Inches(0.4),
         "Cible 18 mois : 3 design partners + 1-2 enterprise = 1.5-3 M € ARR.",
         size=14, bold=True, color=DARK_BLUE, align=PP_ALIGN.CENTER)

add_footer(slide, 9)

# ─────────────────────────────────────────────────────────────────────
#  Slide 10 — Traction + Validation
# ─────────────────────────────────────────────────────────────────────
slide = prs.slides.add_slide(prs.slide_layouts[6])
add_header_bar(slide)

add_text(slide, Inches(0.5), Inches(0.5), Inches(12), Inches(0.8),
         "Traction + validation", size=36, bold=True, color=DARK_BLUE)
add_text(slide, Inches(0.5), Inches(1.3), Inches(12), Inches(0.6),
         "Honest framing : pre-seed, produit audit-grade prêt, design partners en cours.",
         size=18, color=LIGHT_GREY)

# 2 colonnes : ce qu'on a / ce qu'on cherche
add_text(slide, Inches(0.6), Inches(2.3), Inches(6), Inches(0.5),
         "Ce qu'on a aujourd'hui", size=18, bold=True, color=ACCENT)
add_text(slide, Inches(7.0), Inches(2.3), Inches(6), Inches(0.5),
         "Ce qu'on cherche dans 12 mois", size=18, bold=True, color=DARK_BLUE)

# Box gauche
add_box(slide, Inches(0.6), Inches(2.9), Inches(6.0), Inches(3.6),
        RGBColor(0xe6, 0xf2, 0xeb))
got = """• 174 tests release-grade pinned

• 35 CB scenarios validés

• PR #27 closed avec triple sign-off

• 12 mois de platform engineering écrit
  et testé

• Pack démo reproductible
  (cargo test → 174 PASS en < 10 sec)

• Audit chain pinned structurellement
  (30 meta-tests)

• Wow-moment vérifiable :
  modifier 1 caractère → CI rouge"""
add_text(slide, Inches(0.8), Inches(3.0), Inches(5.6), Inches(3.5),
         got, size=12, color=BLACK)

# Box droite
add_box(slide, Inches(7.0), Inches(2.9), Inches(5.7), Inches(3.6),
        RGBColor(0xe6, 0xee, 0xfa))
target = """• 3 design partners signés
  (objectif : 1 pharma, 1 finance, 1 cybersec)

• 1-2 enterprise contracts (500 k-1.5 M)

• 2 senior platform engineers hires

• SOC 2 Type I + ISO 27001 prep

• 1.5-3 M € ARR

• Customer-led roadmap

• Conversation Series A à 18 mois
  (10-15 M € next round)"""
add_text(slide, Inches(7.2), Inches(3.0), Inches(5.3), Inches(3.5),
         target, size=12, color=BLACK)

# Pipeline note
add_text(slide, Inches(0.5), Inches(6.7), Inches(12.3), Inches(0.4),
         "Premier contact en cours : Roche Innovation Center, Kudelski Security, Pictet (à confirmer).",
         size=12, color=LIGHT_GREY, align=PP_ALIGN.CENTER)

add_footer(slide, 10)

# ─────────────────────────────────────────────────────────────────────
#  Slide 11 — Team + Ask
# ─────────────────────────────────────────────────────────────────────
slide = prs.slides.add_slide(prs.slide_layouts[6])
add_header_bar(slide)

add_text(slide, Inches(0.5), Inches(0.5), Inches(12), Inches(0.8),
         "Équipe + ask", size=36, bold=True, color=DARK_BLUE)

# Team
add_text(slide, Inches(0.6), Inches(1.5), Inches(6), Inches(0.4),
         "Équipe fondatrice", size=18, bold=True, color=ACCENT)

add_box(slide, Inches(0.6), Inches(2.0), Inches(5.8), Inches(2.0),
        RGBColor(0xe6, 0xf2, 0xeb), line_color=DARK_BLUE)
add_text(slide, Inches(0.8), Inches(2.15), Inches(5.4), Inches(0.4),
         "Rodrigue Pekar", size=16, bold=True, color=DARK_BLUE)
add_text(slide, Inches(0.8), Inches(2.55), Inches(5.4), Inches(0.4),
         "Founding Engineer — EPFL Lausanne",
         size=12, color=LIGHT_GREY)
add_text(slide, Inches(0.8), Inches(2.95), Inches(5.4), Inches(1.0),
         "Architecte EFE.  PR #27 G.11 closure (174 tests).  6 mois de focus sur audit-grade infrastructure.",
         size=12, color=BLACK)

add_box(slide, Inches(0.6), Inches(4.2), Inches(5.8), Inches(1.0),
        RGBColor(0xf5, 0xf5, 0xfa), line_color=LIGHT_GREY)
add_text(slide, Inches(0.8), Inches(4.3), Inches(5.4), Inches(0.4),
         "Co-fondateur (à confirmer côté business)",
         size=14, bold=True, color=LIGHT_GREY)
add_text(slide, Inches(0.8), Inches(4.7), Inches(5.4), Inches(0.4),
         "Profil business / sales — à compléter.",
         size=11, color=LIGHT_GREY)

# Ask
add_text(slide, Inches(7.0), Inches(1.5), Inches(6), Inches(0.4),
         "Ask — pre-seed round", size=18, bold=True, color=DARK_BLUE)

add_box(slide, Inches(7.0), Inches(2.0), Inches(5.7), Inches(4.5),
        RGBColor(0xe6, 0xee, 0xfa), line_color=DARK_BLUE)

ask_text = """Round visé        :  500 k — 1 M CHF
Valuation post    :  4 — 6 M CHF post-money
Runway visé       :  18 mois

Use of funds :

  50 %  Engineering (2 senior platform hires)
  30 %  Sales + customer development
  20 %  Legal / compliance / SOC 2 prep

Milestones 18 mois :

  •  3 design partners signés
  •  1-2 enterprise contracts (500 k-1.5 M)
  •  1.5-3 M € ARR run-rate
  •  Series A conversation ouverte"""
add_text(slide, Inches(7.2), Inches(2.15), Inches(5.3), Inches(4.3),
         ask_text, size=12, color=BLACK, font="Menlo")

# Lead investor invitation
add_text(slide, Inches(0.5), Inches(6.6), Inches(12.3), Inches(0.5),
         "Kovant comme lead investor ?  Anchor pre-seed Suisse pour audit-grade agentic AI infrastructure.",
         size=13, bold=True, color=DARK_BLUE, align=PP_ALIGN.CENTER)

add_footer(slide, 11)

# ─────────────────────────────────────────────────────────────────────
#  Slide 12 — Vision / close
# ─────────────────────────────────────────────────────────────────────
slide = prs.slides.add_slide(prs.slide_layouts[6])

# Bandeau couleur supérieur (comme slide 1)
top_bar = slide.shapes.add_shape(MSO_SHAPE.RECTANGLE,
                                  Inches(0), Inches(0), SW, Inches(2.0))
top_bar.fill.solid()
top_bar.fill.fore_color.rgb = DARK_BLUE
top_bar.line.fill.background()

add_text(slide, Inches(0.8), Inches(0.6), Inches(11), Inches(0.6),
         "Vision", size=36, bold=True, color=BG_WHITE)
add_text(slide, Inches(0.8), Inches(1.3), Inches(11), Inches(0.6),
         "L'audit-grade devient le moat de l'agentic AI.",
         size=20, color=BG_WHITE)

# Main message
add_text(slide, Inches(0.8), Inches(2.7), Inches(11.7), Inches(2.5),
         '"Dans 5 ans, déployer un agent IA dans une industrie régulée\n'
         'sans audit-grade infrastructure sera impossible.\n\n'
         'EFE est positionné pour être la couche d\'infrastructure\n'
         'standard, européenne, audit-by-construction."',
         size=22, color=BLACK, anchor=MSO_ANCHOR.MIDDLE, align=PP_ALIGN.CENTER)

# Contact
add_box(slide, Inches(2.0), Inches(5.7), Inches(9.3), Inches(1.3),
        RGBColor(0xf5, 0xf7, 0xfa), line_color=DARK_BLUE)
add_text(slide, Inches(2.2), Inches(5.85), Inches(9), Inches(0.4),
         "Rodrigue Pekar — rodrigue.pekar@icloud.com",
         size=14, bold=True, color=DARK_BLUE, align=PP_ALIGN.CENTER)
add_text(slide, Inches(2.2), Inches(6.25), Inches(9), Inches(0.4),
         "Le repo est reproductible : git clone + cargo test → 174 PASS.",
         size=12, color=BLACK, align=PP_ALIGN.CENTER)
add_text(slide, Inches(2.2), Inches(6.6), Inches(9), Inches(0.4),
         "Merci.  Questions ?",
         size=14, bold=True, color=ACCENT, align=PP_ALIGN.CENTER)

add_footer(slide, 12)

# ─────────────────────────────────────────────────────────────────────
#  Save
# ─────────────────────────────────────────────────────────────────────
output = "/Users/rodrigue.pekar/Downloads/jjjj/efe/docs/marketing/EFE-Kovant-Pitch-2026-05-21.pptx"
prs.save(output)
print(f"Saved : {output}")
print(f"Slides : {len(prs.slides)}")
