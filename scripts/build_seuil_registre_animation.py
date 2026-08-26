#!/usr/bin/env python3
"""Génère l'animation SVG « Le Seuil & Le Registre ».

Pourquoi un GÉNÉRATEUR et non un SVG écrit à la main : l'audit du 2026-08-26 a dû rattraper
quinze binaires livrés sans source (13 PDF, 2 decks). Un artefact d'exposition doit se
régénérer. Ici la source est ce fichier ; le SVG est la sortie, et il s'ouvre dans n'importe
quel navigateur sans dépendance ni lecteur.

    python3 scripts/build_seuil_registre_animation.py [chemin_de_sortie.svg]

Chaque élément dessiné correspond à un mécanisme VÉRIFIÉ dans le dépôt :
  · le refus au démarrage ................ efe-swarm-gateway/src/main.rs:411 (4 invariants)
  · le chemin local sans clé ............. efe-swarm-gateway/src/lib.rs:1881 (selfhost_e2e_real)
  · la chaîne SUP-2024 .................. reg-evidence/examples/csddd_seed.rs
  · l'ancre opt-in ...................... main.rs:286 (« ancrage INACTIF » sinon)
  · les quatre cadres ................... reg-evidence/src/framework.rs:18
  · le vérifieur zéro dépendance ........ verify-mini/src/main.rs (exit 0 / 1)
"""
import sys

W, H, DUR = 1280, 772, "28s"
NAVY, GREY, GREEN, RED, AMBER, PLUM = "#1F3A5F", "#54606E", "#2C6E49", "#A6322E", "#9C6A16", "#7A4EA8"
EDGE, FILL, PAPER = "#D9E0EA", "#F3F5F8", "#FFFFFF"

_entries: list[float] = []


def cls(t: float) -> str:
    """Enregistre un instant d'apparition (en % du cycle) et rend la classe correspondante."""
    if t not in _entries:
        _entries.append(t)
    return f"e{str(t).replace('.', '_')}"


def keyframes() -> str:
    out = [".el{opacity:0}"]
    for t in sorted(_entries):
        n = f"e{str(t).replace('.', '_')}"
        # invisible avant t, monte en 1,6 %, tient jusqu'à 97 %, s'efface pour la boucle
        out.append(
            f"@keyframes k{n}{{0%,{max(t - 0.1, 0):.2f}%{{opacity:0}}"
            f"{t + 1.6:.2f}%,97%{{opacity:1}}100%{{opacity:0}}}}"
        )
        out.append(f".{n}{{animation:k{n} {DUR} linear infinite}}")
    return "\n    ".join(out)


P: list[str] = []


def add(s: str) -> None:
    P.append(s)


def box(x, y, w, h, t, fill=PAPER, stroke=EDGE, rx=7, sw=1.2):
    add(f'<rect class="el {t}" x="{x}" y="{y}" width="{w}" height="{h}" rx="{rx}" '
        f'fill="{fill}" stroke="{stroke}" stroke-width="{sw}"/>')


def txt(x, y, s, t, size=14, fill=NAVY, weight="normal", anchor="start", style="normal", mono=False):
    fam = "ui-monospace,SFMono-Regular,Menlo,monospace" if mono else \
          "Palatino,'Palatino Linotype','Book Antiqua',Georgia,serif"
    add(f'<text class="el {t}" x="{x}" y="{y}" font-family="{fam}" font-size="{size}" '
        f'fill="{fill}" font-weight="{weight}" text-anchor="{anchor}" font-style="{style}">{s}</text>')


def arrow(x1, y1, x2, y2, t, color=GREY, w=1.6):
    add(f'<line class="el {t}" x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" stroke="{color}" '
        f'stroke-width="{w}" marker-end="url(#ar)"/>')


def band(y, n, title, t):
    txt(40, y, f'<tspan fill="{PLUM}" font-weight="bold">{n}</tspan>  {title}', t, 15, GREY, "bold")


# ── 0 · titre ────────────────────────────────────────────────────────────────────────────
t = cls(1)
txt(W // 2, 40, "Scella — Le Seuil &amp; Le Registre", t, 27, NAVY, "bold", "middle")
txt(W // 2, 62, "ce qui franchit le périmètre est scellé — et n'importe qui peut le re-vérifier",
    cls(2), 14.5, GREY, "normal", "middle", "italic")

# ── 1 · LE SEUIL ─────────────────────────────────────────────────────────────────────────
band(103, "1", "LE SEUIL — le périmètre que vous déployez chez vous", cls(3))
box(40, 118, 168, 62, cls(5), FILL)
txt(124, 145, "votre application", cls(5), 14, NAVY, "bold", "middle")
txt(124, 165, "un appel de modèle", cls(5), 12, GREY, "normal", "middle")
arrow(214, 149, 268, 149, cls(8))
box(274, 112, 176, 74, cls(10), NAVY, NAVY)
txt(362, 143, "LE SEUIL", cls(10), 17, PAPER, "bold", "middle")
txt(362, 165, "passerelle souveraine", cls(10), 11.5, "#B9C7DA", "normal", "middle")
arrow(456, 136, 520, 122, cls(14), GREEN)
box(526, 100, 300, 46, cls(14), "#F1F7F3", GREEN)
txt(542, 120, "modèle local — sans aucune clé d'API", cls(14), 13.5, GREEN, "bold")
txt(542, 137, "le chemin souverain, exercé de bout en bout", cls(14), 11.5, GREY)
arrow(456, 162, 520, 178, cls(17), AMBER)
box(526, 156, 300, 46, cls(17), "#FDF8F0", AMBER)
txt(542, 176, "modèle tiers — quatre invariants exigés", cls(17), 13.5, AMBER, "bold")
txt(542, 193, "clé · authentification · durabilité · caviardage", cls(17), 11.5, GREY)
arrow(832, 149, 890, 149, cls(20))
box(896, 118, 190, 62, cls(20), FILL, NAVY, 7, 1.6)
txt(991, 142, "un reçu", cls(20), 15, NAVY, "bold", "middle")
txt(991, 161, "rendu avec la réponse", cls(20), 11.5, GREY, "normal", "middle")
txt(1100, 137, "à défaut d'un", cls(17), 11.5, AMBER)
txt(1100, 153, "seul invariant :", cls(17), 11.5, AMBER)
txt(1100, 172, "il ne démarre pas", cls(17), 12, AMBER, "bold")

# ── 2 · LA CHAÎNE SCELLÉE ────────────────────────────────────────────────────────────────
band(238, "2", "CE QUI FRANCHIT EST SCELLÉ — le cas SUP-2024, une diligence fournisseur", cls(24))
NODES = [
    ("supplier_intake", "extraction du dossier", NAVY, 27),
    ("évidence figée", "sanctions / PEP au jour J", GREY, 30),
    ("sanction_screen", "PASS", GREEN, 33),
    ("esg_risk_assess", "REVIEW — sous-tier", AMBER, 36),
    ("diligence_aggregate", "CONDITIONNEL", NAVY, 39),
    ("oversight humain", "escalade, puis approbation", PLUM, 42),
]
x, w, gap, y = 44, 190, 9, 268
for i, (a, b, c, tt) in enumerate(NODES):
    k = cls(tt)
    box(x, y, w, 62, k, PAPER, c, 6, 1.5)
    add(f'<rect class="el {k}" x="{x}" y="{y}" width="4" height="62" rx="2" fill="{c}"/>')
    txt(x + 15, y + 26, a, k, 12.5, c, "bold", "start", "normal", True)
    txt(x + 15, y + 46, b, k, 11.5, GREY)
    if i:
        arrow(x - gap - 1, y + 31, x - 2, y + 31, k, EDGE, 2)
    x += w + gap
txt(44, 356, "chaque acte scellé au fil de l'exécution — l'ordre, les entrées, la sortie, et qui a tranché",
    cls(44), 12, GREY, "normal", "start", "italic")
box(940, 352, 296, 40, cls(47), "#F7F3FB", PLUM)
txt(956, 370, "ancre publiée (au choix)", cls(47), 12.5, PLUM, "bold")
txt(956, 385, "sans elle, une troncature resterait invisible", cls(47), 11, GREY)

# ── 3 · LE REGISTRE ──────────────────────────────────────────────────────────────────────
band(432, "3", "LE REGISTRE — une seule capture, relue sous trois cadres", cls(52))
CADRES = [("CSDDD", "Directive (UE) 2024/1760", 55), ("LkSG", "devoir de vigilance · §10", 58),
          ("AI Act", "Règlement (UE) 2024/1689 · art. 12", 61)]
y = 448
for a, b, tt in CADRES:
    k = cls(tt)
    box(44, y, 560, 48, k, FILL)
    txt(60, y + 21, a, k, 14, NAVY, "bold")
    txt(60, y + 39, b, k, 11.5, GREY)
    add(f'<circle class="el {k}" cx="392" cy="{y + 25}" r="5" fill="{GREEN}"/>')
    txt(404, y + 29, "couvert", k, 11.5, GREEN)
    add(f'<circle class="el {k}" cx="470" cy="{y + 25}" r="5" fill="{AMBER}"/>')
    txt(482, y + 29, "partiel", k, 11.5, AMBER)
    add(f'<circle class="el {k}" cx="540" cy="{y + 25}" r="5" fill="{GREY}"/>')
    txt(552, y + 29, "hors", k, 11.5, GREY)
    y += 56
txt(44, 628, "les trois nombres sont publiés ensemble : la seule couverture ferait passer",
    cls(64), 12, GREY, "normal", "start", "italic")
txt(44, 645, "un dossier vide pour une bonne nouvelle", cls(64), 12, GREY, "normal", "start", "italic")

# ── 4 · LA RE-PREUVE ─────────────────────────────────────────────────────────────────────
band(432, "4", "LA RE-PREUVE — par un tiers, sans rien installer", cls(66))
box(660, 448, 576, 56, cls(69), FILL, NAVY, 7, 1.5)
txt(676, 470, "verify-mini", cls(69), 13.5, NAVY, "bold", "start", "normal", True)
txt(676, 490, "arbre de dépendances VIDE — votre auditeur le recompile lui-même",
    cls(69), 11.5, GREY)
arrow(760, 510, 760, 528, cls(73), GREEN)
box(660, 532, 576, 44, cls(73), "#F1F7F3", GREEN)
txt(676, 559, "exit 0", cls(73), 14, GREEN, "bold", "start", "normal", True)
txt(748, 559, "— le dossier est intègre", cls(73), 12.5, GREEN)
box(660, 596, 576, 44, cls(79), "#FCF2F1", RED)
txt(676, 616, "un seul champ scellé retourné", cls(79), 12.5, RED, "bold")
txt(676, 633, "le verdict de criblage passe de FAIL à PASS", cls(79), 11.5, GREY)
arrow(760, 646, 760, 662, cls(84), RED)
box(660, 666, 576, 44, cls(84), "#FCF2F1", RED, 7, 1.6)
txt(676, 693, "exit 1", cls(84), 14, RED, "bold", "start", "normal", True)
txt(748, 693, "— et le record altéré est désigné", cls(84), 12.5, RED)

# ── pied ─────────────────────────────────────────────────────────────────────────────────
add(f'<line class="el {cls(90)}" x1="44" y1="730" x2="{W - 44}" y2="730" stroke="{EDGE}" stroke-width="1"/>')
txt(44, 750, "Ce que le reçu ne prouve pas est écrit DANS le dossier remis : sans ancrage externe il "
             "atteste l'intégrité — jamais la correction, jamais la non-fabrication.",
    cls(90), 12, GREY, "normal", "start", "italic")

SVG = f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {W} {H}" width="100%" height="100%"
     preserveAspectRatio="xMidYMid meet"
     role="img" aria-label="Le Seuil et Le Registre — du périmètre au dossier re-vérifiable">
  <title>Scella — Le Seuil &amp; Le Registre</title>
  <defs>
    <marker id="ar" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="6" markerHeight="6" orient="auto">
      <path d="M0,0 L10,5 L0,10 z" fill="{GREY}"/>
    </marker>
  </defs>
  <style>
    {keyframes()}
    @media (prefers-reduced-motion: reduce){{ .el{{animation:none!important;opacity:1}} }}
  </style>
  <rect width="{W}" height="{H}" fill="{PAPER}"/>
  {chr(10).join("  " + p for p in P)}
</svg>
'''

out = sys.argv[1] if len(sys.argv) > 1 else "docs/reports/Scella_Animation_Seuil_Registre_2026-08-26.svg"
with open(out, "w", encoding="utf-8") as f:
    f.write(SVG)
print(f"écrit : {out}  ({len(SVG)} octets, {len(_entries)} instants d'apparition, cycle {DUR})")
