"""Build the EFE adoption roadmap by letters (A → E).

Five progressive integration forms for EFE Mode 1:
    A — CLI one-shot
    B — REPL interactif
    C — Script / démo applicatif
    D — Intégration dans projet Python existant
    E — Service backend (sidecar)

Output: EFE_MVP_Roadmap_Lettres.pdf (repo root).
"""

from __future__ import annotations

from pathlib import Path

from reportlab.lib import colors
from reportlab.lib.enums import TA_CENTER, TA_JUSTIFY, TA_LEFT
from reportlab.lib.pagesizes import A4
from reportlab.lib.styles import ParagraphStyle, getSampleStyleSheet
from reportlab.lib.units import cm, mm
from reportlab.platypus import (
    PageBreak,
    Paragraph,
    Preformatted,
    SimpleDocTemplate,
    Spacer,
    Table,
    TableStyle,
)


# --------------------------------------------------------------------------
#  Styles
# --------------------------------------------------------------------------

styles = getSampleStyleSheet()

TITLE = ParagraphStyle(
    "Title", parent=styles["Title"], fontName="Helvetica-Bold",
    fontSize=26, leading=32, alignment=TA_CENTER,
    textColor=colors.HexColor("#0B3D91"), spaceAfter=12,
)
SUBTITLE = ParagraphStyle(
    "Subtitle", parent=styles["Normal"], fontName="Helvetica",
    fontSize=14, leading=18, alignment=TA_CENTER,
    textColor=colors.HexColor("#333333"), spaceAfter=6,
)
META = ParagraphStyle(
    "Meta", parent=styles["Normal"], fontName="Helvetica-Oblique",
    fontSize=10, leading=14, alignment=TA_CENTER,
    textColor=colors.HexColor("#666666"),
)
H1 = ParagraphStyle(
    "H1", parent=styles["Heading1"], fontName="Helvetica-Bold",
    fontSize=18, leading=22, textColor=colors.HexColor("#0B3D91"),
    spaceBefore=12, spaceAfter=8,
)
H2 = ParagraphStyle(
    "H2", parent=styles["Heading2"], fontName="Helvetica-Bold",
    fontSize=14, leading=18, textColor=colors.HexColor("#1F4E79"),
    spaceBefore=10, spaceAfter=6,
)
H3 = ParagraphStyle(
    "H3", parent=styles["Heading3"], fontName="Helvetica-Bold",
    fontSize=11.5, leading=14, textColor=colors.HexColor("#2F5496"),
    spaceBefore=6, spaceAfter=4,
)
BODY = ParagraphStyle(
    "Body", parent=styles["Normal"], fontName="Helvetica",
    fontSize=10.5, leading=14, alignment=TA_JUSTIFY, spaceAfter=6,
)
BULLET = ParagraphStyle(
    "Bullet", parent=BODY, leftIndent=14, bulletIndent=4, spaceAfter=3,
)
CODE = ParagraphStyle(
    "Code", parent=styles["Normal"], fontName="Courier",
    fontSize=8.5, leading=11, leftIndent=8,
    backColor=colors.HexColor("#F2F4F7"),
    borderPadding=4, spaceAfter=6,
)
TOC_ITEM = ParagraphStyle(
    "TocItem", parent=BODY, alignment=TA_LEFT, spaceAfter=3, leftIndent=6,
)

LETTER_COLORS = {
    "A": colors.HexColor("#2E7D32"),  # vert — simple
    "B": colors.HexColor("#1565C0"),  # bleu — interactif
    "C": colors.HexColor("#6A1B9A"),  # violet — scénario
    "D": colors.HexColor("#E65100"),  # orange — intégration
    "E": colors.HexColor("#C62828"),  # rouge — infra
}


# --------------------------------------------------------------------------
#  Helpers
# --------------------------------------------------------------------------

def p(text: str, style=BODY) -> Paragraph:
    return Paragraph(text, style)


def bullets(items: list[str]) -> list[Paragraph]:
    return [Paragraph(f"• {it}", BULLET) for it in items]


def code(text: str) -> Preformatted:
    return Preformatted(text, CODE)


def table(data: list[list], col_widths: list[float], header=True,
          extra_style: list | None = None) -> Table:
    t = Table(data, colWidths=col_widths, repeatRows=1 if header else 0)
    base = [
        ("FONT", (0, 0), (-1, -1), "Helvetica", 9),
        ("VALIGN", (0, 0), (-1, -1), "TOP"),
        ("GRID", (0, 0), (-1, -1), 0.3, colors.HexColor("#AAAAAA")),
        ("LEFTPADDING", (0, 0), (-1, -1), 4),
        ("RIGHTPADDING", (0, 0), (-1, -1), 4),
        ("TOPPADDING", (0, 0), (-1, -1), 3),
        ("BOTTOMPADDING", (0, 0), (-1, -1), 3),
    ]
    if header:
        base += [
            ("BACKGROUND", (0, 0), (-1, 0), colors.HexColor("#0B3D91")),
            ("TEXTCOLOR", (0, 0), (-1, 0), colors.white),
            ("FONT", (0, 0), (-1, 0), "Helvetica-Bold", 9),
            ("ALIGN", (0, 0), (-1, 0), "CENTER"),
        ]
    if extra_style:
        base += extra_style
    t.setStyle(TableStyle(base))
    return t


def letter_banner(letter: str, title: str, tagline: str) -> list:
    color = LETTER_COLORS[letter]
    style_letter = ParagraphStyle(
        f"Letter{letter}", parent=TITLE, fontSize=48, leading=56,
        textColor=color, alignment=TA_LEFT, spaceAfter=0,
    )
    style_title = ParagraphStyle(
        f"LetterTitle{letter}", parent=H1,
        textColor=color, fontSize=20, leading=24,
        spaceBefore=0, spaceAfter=2,
    )
    style_tag = ParagraphStyle(
        f"LetterTag{letter}", parent=BODY, fontName="Helvetica-Oblique",
        textColor=colors.HexColor("#555555"), alignment=TA_LEFT,
    )
    banner_table = Table(
        [
            [Paragraph(letter, style_letter),
             [Paragraph(title, style_title), Paragraph(tagline, style_tag)]],
        ],
        colWidths=[2.8 * cm, 14.2 * cm],
    )
    banner_table.setStyle(TableStyle([
        ("VALIGN", (0, 0), (-1, -1), "MIDDLE"),
        ("LEFTPADDING", (0, 0), (-1, -1), 0),
        ("RIGHTPADDING", (0, 0), (-1, -1), 0),
        ("TOPPADDING", (0, 0), (-1, -1), 0),
        ("BOTTOMPADDING", (0, 0), (-1, -1), 6),
        ("LINEBELOW", (0, 0), (-1, -1), 1.5, color),
    ]))
    return [banner_table, Spacer(1, 4 * mm)]


# --------------------------------------------------------------------------
#  Content
# --------------------------------------------------------------------------

def build_cover() -> list:
    return [
        Spacer(1, 4 * cm),
        p("EFE — Essaim Fluide Éphémère", TITLE),
        Spacer(1, 4 * mm),
        p("Roadmap d'adoption par lettres", SUBTITLE),
        Spacer(1, 2 * mm),
        p("A · B · C · D · E", ParagraphStyle(
            "Letters", parent=TITLE, fontSize=36, leading=40,
            alignment=TA_CENTER, textColor=colors.HexColor("#333333"),
        )),
        Spacer(1, 2.5 * cm),
        p(
            "Cinq formes progressives d'intégration du Mode 1,<br/>"
            "de l'essai en une ligne au déploiement production.",
            SUBTITLE,
        ),
        Spacer(1, 3 * cm),
        p("Auteur : rodrigue.pekar@icloud.com", META),
        p("Date : 19 avril 2026", META),
        p("Version : 1.0.0", META),
        PageBreak(),
    ]


def build_toc() -> list:
    items = [
        "1. Vue d'ensemble — pourquoi cinq lettres ?",
        "2. Tableau comparatif express",
        "3. Matrice effort × valeur",
        "4. Lettre A — CLI one-shot",
        "5. Lettre B — REPL interactif",
        "6. Lettre C — Script / démo applicatif",
        "7. Lettre D — Intégration dans projet Python existant",
        "8. Lettre E — Service backend (sidecar)",
        "9. Chemin d'adoption recommandé",
        "10. Arbre de décision — quelle lettre choisir ?",
        "11. Jalons par lettre",
        "12. Critères Go / No-Go par lettre",
        "13. Dépendances entre lettres",
        "14. Annexe — snippets minimaux par lettre",
    ]
    return [
        p("Table des matières", H1),
        *[p(it, TOC_ITEM) for it in items],
        PageBreak(),
    ]


def section_overview() -> list:
    return [
        p("1. Vue d'ensemble — pourquoi cinq lettres ?", H1),
        p(
            "Adopter EFE dans la vraie vie n'est jamais une décision binaire. "
            "Entre le <i>« j'essaie en 30 secondes »</i> et le <i>« je déploie en "
            "production multi-tenant »</i>, il existe cinq paliers bien distincts. "
            "Les nommer <b>A, B, C, D, E</b> permet à une équipe de dire sans ambiguïté "
            "« on est en C, on cible D pour T3 ».",
            BODY,
        ),
        p("Les cinq lettres en une phrase", H2),
        *bullets([
            "<b><font color='#2E7D32'>A · CLI one-shot</font></b> — une commande, une classification, "
            "zéro code utilisateur.",
            "<b><font color='#1565C0'>B · REPL interactif</font></b> — boucle question / réponse "
            "dans un terminal, pour explorer un catalog.",
            "<b><font color='#6A1B9A'>C · Script / démo applicatif</font></b> — un fichier Python "
            "isolé qui orchestre EFE + un LLM hôte sur un scénario métier.",
            "<b><font color='#E65100'>D · Intégration projet Python</font></b> — EFE devient un "
            "module embarqué dans un code base existant (Django, FastAPI, Celery…).",
            "<b><font color='#C62828'>E · Service backend</font></b> — EFE tourne en sidecar ou "
            "microservice, consommé par n'importe quel runtime via gRPC / HTTP.",
        ]),
        p("Propriété clé — monotonie des prérequis", H2),
        p(
            "Chaque lettre ajoute strictement aux prérequis de la précédente. On ne saute "
            "jamais un palier : si l'on vise D directement, on reproduit de facto A → B → C "
            "dans le processus de validation, simplement en interne. Les sauter officiellement "
            "cache de la dette technique.",
            BODY,
        ),
        PageBreak(),
    ]


def section_compare_table() -> list:
    rows = [
        ["Lettre", "Forme", "Effort", "Prérequis", "Cible typique", "ROI immédiat"],
        ["A", "CLI one-shot",          "0.5 j",  "serveur up, pip install",     "PoC technique", "Validation  << 1h"],
        ["B", "REPL",                  "2 j",    "A + config env",              "Tuning catalog", "Iteration rapide"],
        ["C", "Script démo",           "1-2 sem","B + ANTHROPIC_API_KEY",       "Démo métier",   "Convaincre un PM"],
        ["D", "Lib dans projet",       "2-4 sem","C + archi cible",             "App production",  "Valeur client directe"],
        ["E", "Service backend",       "3-4 sem","D + infra",                   "Équipe hétérogène", "Scalabilité"],
    ]
    col_widths = [1.1 * cm, 3.3 * cm, 1.7 * cm, 3.6 * cm, 3.5 * cm, 3.8 * cm]
    # Colorier la première colonne par lettre
    letter_style = []
    for i, letter in enumerate("ABCDE"):
        row = i + 1
        letter_style.append(
            ("BACKGROUND", (0, row), (0, row), LETTER_COLORS[letter])
        )
        letter_style.append(
            ("TEXTCOLOR", (0, row), (0, row), colors.white)
        )
        letter_style.append(
            ("FONT", (0, row), (0, row), "Helvetica-Bold", 11)
        )
        letter_style.append(
            ("ALIGN", (0, row), (0, row), "CENTER")
        )
    return [
        p("2. Tableau comparatif express", H1),
        p(
            "Un coup d'œil suffit : plus la lettre monte, plus l'effort augmente, "
            "mais plus la valeur captée est durable. Les trois premières colonnes "
            "répondent à <i>« combien ça coûte ? »</i>, les trois dernières à "
            "<i>« pour qui et pourquoi ? »</i>.",
            BODY,
        ),
        Spacer(1, 2 * mm),
        table(rows, col_widths, header=True, extra_style=letter_style),
        PageBreak(),
    ]


def section_matrix() -> list:
    rows = [
        ["", "Effort → ", "Faible", "Moyen", "Élevé"],
        ["Valeur ↓", "Éphémère",    "A",    "",     ""],
        ["",         "Récurrente",  "B",    "C",    ""],
        ["",         "Structurelle", "",    "D",   "E"],
    ]
    col_widths = [2.5 * cm, 3 * cm, 3 * cm, 3 * cm, 3 * cm]
    letter_style = [
        ("BACKGROUND", (2, 1), (2, 1), LETTER_COLORS["A"]),
        ("BACKGROUND", (2, 2), (2, 2), LETTER_COLORS["B"]),
        ("BACKGROUND", (3, 2), (3, 2), LETTER_COLORS["C"]),
        ("BACKGROUND", (3, 3), (3, 3), LETTER_COLORS["D"]),
        ("BACKGROUND", (4, 3), (4, 3), LETTER_COLORS["E"]),
        ("TEXTCOLOR", (2, 1), (4, 3), colors.white),
        ("FONT", (2, 1), (4, 3), "Helvetica-Bold", 14),
        ("ALIGN", (0, 0), (-1, -1), "CENTER"),
        ("VALIGN", (0, 0), (-1, -1), "MIDDLE"),
    ]
    return [
        p("3. Matrice effort × valeur", H1),
        p(
            "Placer les lettres sur un plan <b>effort</b> × <b>valeur captée</b> "
            "éclaire pourquoi aucune n'est interchangeable. La diagonale "
            "A → B → C → D → E suit un gradient qui n'est <b>ni linéaire ni "
            "substituable</b>.",
            BODY,
        ),
        Spacer(1, 3 * mm),
        table(rows, col_widths, header=True, extra_style=letter_style),
        Spacer(1, 4 * mm),
        p(
            "<b>Lecture.</b> Une PoC en <b>A</b> ne capte qu'une valeur <i>éphémère</i> "
            "(la démo du vendredi soir), mais elle coûte presque rien. Une "
            "intégration en <b>D</b> capte une valeur <i>structurelle</i> (features "
            "livrées aux clients) au prix d'un effort moyen ; seule <b>E</b> justifie "
            "un effort élevé, typiquement quand plusieurs équipes consomment EFE "
            "depuis des runtimes différents.",
            BODY,
        ),
        PageBreak(),
    ]


# --------------------------------------------------------------------------
#  Per-letter sections
# --------------------------------------------------------------------------

def section_letter_A() -> list:
    return [
        *letter_banner(
            "A", "CLI one-shot",
            "Une commande, une classification, zéro code utilisateur.",
        ),
        p("Objectif", H2),
        p(
            "Prouver en moins d'une heure, sur la machine d'un dev, que le moteur EFE "
            "classifie correctement une phrase donnée contre un catalog donné. "
            "Aucun code n'est écrit côté utilisateur — la commande est le produit.",
            BODY,
        ),
        p("Livrable", H2),
        code(
            "$ efe classify \\\n"
            "      --catalog catalogs/support_triage.yaml \\\n"
            "      --text    \"Ma carte a été débitée sans mon consentement\" \\\n"
            "      --budget-ms 5000\n"
            "\n"
            "{\"label\":\"fraud\",\"confidence\":0.575,\"reject\":false,\n"
            " \"alignment_delta\":0.12,\"rounds\":4}"
        ),
        p("Prérequis", H2),
        *bullets([
            "<code>efe-server</code> lancé sur <code>127.0.0.1:50051</code> "
            "(<code>EFE_BACKEND=mvp</code>).",
            "<code>pip install efe-tool</code> (expose la commande <code>efe</code>).",
            "Un catalog YAML compilable (un <code>.compiled.json</code> sera généré).",
        ]),
        p("Critères de sortie — Go A", H2),
        *bullets([
            "<code>efe classify</code> retourne JSON valide en &lt; 2 s.",
            "<code>efe classify --help</code> liste les flags "
            "(<code>--budget-ms</code>, <code>--timeout-s</code>, <code>--max-rounds</code>).",
            "Un test d'intégration shell couvre au moins un cas par classe du catalog.",
        ]),
        p("Effort et planning", H2),
        p(
            "<b>Effort</b> : 0.5 jour. "
            "<b>Risques</b> : aucun structurel — c'est l'outil le plus simple. "
            "Seul piège observable : oublier de lancer le serveur avant d'appeler "
            "la CLI, d'où un message d'erreur explicite prévu en retour.",
            BODY,
        ),
        p("Quand <i>s'arrêter</i> à A ?", H2),
        p(
            "Jamais en production. A est un <b>outil d'enquête</b> (support, debug, démo "
            "technique). Si un cas d'usage semble vouloir vivre en A durablement, "
            "c'est qu'il veut probablement devenir B ou C.",
            BODY,
        ),
        PageBreak(),
    ]


def section_letter_B() -> list:
    return [
        *letter_banner(
            "B", "REPL interactif",
            "Boucle question / réponse, parfaite pour tuner un catalog.",
        ),
        p("Objectif", H2),
        p(
            "Donner à un product owner ou à un ingénieur une <b>session vivante</b> "
            "avec le moteur : taper une phrase, voir le label, ajuster les canonical "
            "sentences du catalog, recharger, retaper. C'est l'outil d'itération "
            "le plus rapide pour calibrer un catalog.",
            BODY,
        ),
        p("Livrable", H2),
        code(
            "$ python -m efe_tool.repl --catalog catalogs/support_triage.yaml\n"
            "EFE> J'ai été débité de 149€ ce matin.\n"
            "  → fraud   (confidence=0.575, rounds=4)\n"
            "EFE> :reload\n"
            "catalog recompiled (fingerprint changed)\n"
            "EFE> J'ai été débité de 149€ ce matin.\n"
            "  → billing (confidence=0.575, rounds=5)"
        ),
        p("Prérequis", H2),
        *bullets([
            "Lettre A validée.",
            "Variables d'env : <code>EFE_ENDPOINT</code>, <code>EFE_CATALOG_PATH</code>.",
            "Commande <code>:reload</code>, <code>:catalog &lt;path&gt;</code>, "
            "<code>:thresholds</code>, <code>:help</code>.",
        ]),
        p("Critères de sortie — Go B", H2),
        *bullets([
            "Une session REPL tient au minimum 30 requêtes consécutives sans redémarrage.",
            "Rechargement à chaud d'un catalog modifié en &lt; 500 ms.",
            "Historique conservé via <code>readline</code> entre sessions.",
        ]),
        p("Effort et planning", H2),
        p(
            "<b>Effort</b> : 2 jours. "
            "<b>Dépendance</b> : module <code>efe_tool.repl</code> à créer "
            "(il n'existe pas au MVP initial). "
            "<b>Piège</b> : maintenir un singleton d'embedder pour éviter de recharger "
            "le modèle à chaque <code>:reload</code>.",
            BODY,
        ),
        p("Quand <i>s'arrêter</i> à B ?", H2),
        p(
            "Pour le travail de tuning d'un catalog, B est l'<b>outil définitif</b> — "
            "il n'y a rien de plus productif. En revanche, B ne sert à rien en "
            "production : aucun système de l'entreprise ne parle à un REPL.",
            BODY,
        ),
        PageBreak(),
    ]


def section_letter_C() -> list:
    return [
        *letter_banner(
            "C", "Script / démo applicatif",
            "Un fichier Python qui orchestre EFE + LLM sur un scénario métier.",
        ),
        p("Objectif", H2),
        p(
            "Convaincre un décideur non-technique qu'EFE résout un vrai problème. "
            "On ne teste plus le moteur — on teste le <b>flux de bout en bout</b> : "
            "message utilisateur → Claude invoque <code>efe_classify</code> → "
            "Claude rédige une réponse adaptée à la classe prédite.",
            BODY,
        ),
        p("Livrable", H2),
        p(
            "<code>examples/demo_agent_claude.py</code>, exécutable en une commande, "
            "reproduit le scénario support bilingue (4 classes, 4 phrases FR + EN). "
            "C'est le livrable de référence du MVP EFE.",
            BODY,
        ),
        code(
            "$ python examples/demo_agent_claude.py\n"
            "[user] Ma carte a été utilisée sans mon consentement\n"
            "[assistant] tool_use efe_classify → {catalog:'support_triage', text:'…'}\n"
            "[tool]      {\"label\":\"fraud\",\"confidence\":0.575, …}\n"
            "[assistant] Je suis désolé d'apprendre cela. Pouvez-vous me confirmer\n"
            "            la date et le montant précis afin qu'on bloque la carte ?"
        ),
        p("Prérequis", H2),
        *bullets([
            "Lettre B validée (pour avoir calibré le catalog).",
            "<code>ANTHROPIC_API_KEY</code> exporté.",
            "Modèle <code>claude-sonnet-4-6</code> disponible sur l'API.",
            "Bridge <code>efe_tool.bridges.claude</code> livré (M11).",
        ]),
        p("Critères de sortie — Go C", H2),
        *bullets([
            "Démo live passe sur ≥ 3 cas / 4.",
            "Le script gère <code>reject=True</code> en posant une question de clarification.",
            "Le script gère <code>stop_reason=\"max_tokens\"</code> avec un warning.",
            "Exécution reproductible en &lt; 30 s.",
        ]),
        p("Effort et planning", H2),
        p(
            "<b>Effort</b> : 1 à 2 semaines pour un scénario métier correctement "
            "scripté. <b>Piège</b> : ne pas coder en dur les seuils dans le prompt "
            "système — le <code>SYSTEM_PROMPT</code> doit référencer les champs "
            "retournés, pas des valeurs numériques.",
            BODY,
        ),
        PageBreak(),
    ]


def section_letter_D() -> list:
    return [
        *letter_banner(
            "D", "Intégration dans un projet Python existant",
            "EFE devient un module embarqué dans un code base applicatif.",
        ),
        p("Objectif", H2),
        p(
            "Intégrer EFE comme dépendance <b>first-class</b> d'un projet Python "
            "existant (Django, FastAPI, Flask, Celery, Dagster, notebooks) sans "
            "casser les invariants du projet hôte : logs, config, injection de "
            "dépendances, cycle de vie, tests.",
            BODY,
        ),
        p("Livrable", H2),
        *bullets([
            "Fichier <code>efe_agent.py</code> dans l'arborescence hôte, avec "
            "pattern singleton lazy (<code>_get_catalog()</code>, "
            "<code>_get_efe_client()</code>, <code>_get_anthropic()</code>).",
            "Configuration entièrement par variables d'environnement "
            "(<code>EFE_ENDPOINT</code>, <code>EFE_TIMEOUT_S</code>, "
            "<code>EFE_CATALOG_PATH</code>, <code>EFE_CLAUDE_MODEL</code>, "
            "<code>EFE_BUDGET_MS</code>, <code>EFE_MAX_ROUNDS</code>).",
            "Hooks framework-spécifiques :",
        ]),
        *bullets([
            "<b>FastAPI</b> : <code>@app.on_event(\"startup\")</code> pour preload ; "
            "<code>@app.on_event(\"shutdown\")</code> pour <code>client.close()</code>.",
            "<b>Django</b> : <code>AppConfig.ready()</code> pour preload ; "
            "signal de shutdown WSGI.",
            "<b>Flask</b> : pattern <i>application factory</i> + <code>teardown_appcontext</code>.",
            "<b>Celery</b> : <code>@worker_ready.connect</code> pour preload du modèle "
            "au démarrage du worker.",
            "<b>Notebooks</b> : singleton module-level, fermeture via <code>atexit</code>.",
        ]),
        p("Patterns obligatoires", H2),
        *bullets([
            "<b>Lazy init</b> : ne pas charger <code>LocalSTEmbedder</code> au moment "
            "de l'import — uniquement au premier appel.",
            "<b>Thread-safety</b> : <code>threading.Lock</code> autour des singletons.",
            "<b>Idempotence</b> : <code>classify(text)</code> peut être réessayé sans "
            "effet de bord.",
            "<b>Fermeture propre</b> : <code>client.close()</code> au shutdown du "
            "runtime hôte.",
            "<b>Observabilité</b> : logger structuré du host, pas un logger maison.",
        ]),
        p("Critères de sortie — Go D", H2),
        *bullets([
            "<code>pytest</code> passe sur le projet hôte avec et sans EFE disponible.",
            "Temps de démarrage du process hôte n'augmente pas de plus de 100 ms "
            "(grâce au lazy init).",
            "Aucune variable d'environnement codée en dur : tout override possible.",
            "Shutdown gracieux : 0 connexion gRPC résiduelle.",
            "Un <code>README-efe.md</code> interne documente les envs &amp; les hooks.",
        ]),
        p("Effort et planning", H2),
        p(
            "<b>Effort</b> : 2 à 4 semaines selon la taille du code base hôte et la "
            "maturité de son injection de dépendances. "
            "<b>Piège principal</b> : déclencher un import circulaire via "
            "<code>efe_agent.py</code> — le pattern singleton via fonctions "
            "<code>_get_*()</code> le prévient.",
            BODY,
        ),
        PageBreak(),
    ]


def section_letter_E() -> list:
    return [
        *letter_banner(
            "E", "Service backend (sidecar)",
            "EFE tourne en processus séparé, consommable depuis n'importe quel runtime.",
        ),
        p("Objectif", H2),
        p(
            "Exposer EFE comme <b>service réseau</b> à des consommateurs hétérogènes "
            "(Go, TypeScript, Java, …). Le Python n'est plus imposé au client. "
            "C'est la seule lettre qui soutient plusieurs équipes en parallèle sans "
            "duplication de code.",
            BODY,
        ),
        p("Livrable", H2),
        *bullets([
            "Image Docker <code>efe-server:1.0.0</code> basée sur Debian slim, ~300 MB.",
            "Deux endpoints : <code>gRPC</code> natif (port 50051), <code>REST</code> "
            "via gateway (port 8080).",
            "Manifestes Kubernetes dans <code>deploy/k8s/</code> : "
            "Deployment, Service, HorizontalPodAutoscaler, PodDisruptionBudget.",
            "Chart Helm optionnel en <code>deploy/helm/efe/</code>.",
            "Admin endpoint <code>/admin/v1/catalogs</code> pour enregistrement distant.",
            "Authentification par token JWT signé (JWK rotatif).",
        ]),
        p("Infra requise", H2),
        *bullets([
            "Cluster K8s avec 2 répliques minimum, autoscale 2–10.",
            "Volume partagé (ou bucket S3) pour les catalogs compilés.",
            "Ingress TLS, idéalement mTLS pour gRPC inter-services.",
            "Prometheus + Grafana (livrable M12).",
            "Runbook d'opération pour on-call (cold start, drift catalog, timeout).",
        ]),
        p("Critères de sortie — Go E", H2),
        *bullets([
            "SLO : disponibilité ≥ 99.5 %, p95 latence &lt; 2 s sur 7 jours glissants.",
            "Tests de charge : 100 req/s soutenu avec CPU &lt; 70 %.",
            "Zero-downtime deploy validé sur au moins 3 rollouts.",
            "Runbook testé par un tiers (game day).",
        ]),
        p("Effort et planning", H2),
        p(
            "<b>Effort</b> : 3 à 4 semaines. "
            "<b>Coût récurrent</b> : non-négligeable (infra, on-call). "
            "<b>Piège</b> : sous-estimer le coût du preload du modèle au cold start — "
            "prévoir un <code>readinessProbe</code> qui attend le preload, sinon les "
            "premières requêtes après scale-up échouent en timeout.",
            BODY,
        ),
        p("Quand <i>pas</i> faire E ?", H2),
        p(
            "Tant qu'une seule équipe Python consomme EFE, E est une sur-ingénierie. "
            "La lettre D suffit largement jusqu'à l'apparition d'un second consommateur "
            "dans un autre langage.",
            BODY,
        ),
        PageBreak(),
    ]


# --------------------------------------------------------------------------
#  Cross-letter sections
# --------------------------------------------------------------------------

def section_path() -> list:
    return [
        p("9. Chemin d'adoption recommandé", H1),
        p(
            "La trajectoire <b>A → B → C → D → E</b> n'est pas obligatoire, mais chaque "
            "saut non-validé produit de la dette. Trois trajectoires typiques :",
            BODY,
        ),
        p("Trajectoire « équipe produit pilote »", H2),
        *bullets([
            "S0 → S1 : <b>A</b> pour prouver que le moteur marche.",
            "S2 : <b>B</b> pour calibrer le catalog.",
            "S3 → S5 : <b>C</b> pour la démo exécutive.",
            "S6 → S10 : <b>D</b> pour livraison aux utilisateurs finaux.",
            "<i>E reporté</i> tant qu'un seul runtime consomme EFE.",
        ]),
        p("Trajectoire « plateforme interne »", H2),
        *bullets([
            "S0 → S2 : <b>A</b> + <b>C</b> en parallèle (une équipe pour chaque).",
            "S3 → S6 : <b>D</b> dans un projet pilote.",
            "S7 → S12 : <b>E</b> pour exposer au reste de l'organisation.",
        ]),
        p("Trajectoire « notebook researcher »", H2),
        *bullets([
            "S0 : <b>A</b> pour les tests one-off.",
            "S1 → S∞ : <b>B</b> permanent dans l'exploration data.",
            "<b>D</b> uniquement si un papier exige une reproduction scriptée.",
        ]),
        PageBreak(),
    ]


def section_decision_tree() -> list:
    return [
        p("10. Arbre de décision — quelle lettre choisir ?", H1),
        p(
            "Parcours descendant : s'arrêter à la première réponse « oui ».",
            BODY,
        ),
        code(
            "Est-ce que je veux juste vérifier qu'EFE répond bien à\n"
            "  UNE phrase sur UN catalog ?                              → A\n"
            "\n"
            "Est-ce que je veux itérer sur les canonical_sentences\n"
            "  d'un catalog en live ?                                   → B\n"
            "\n"
            "Est-ce que je veux montrer un scénario métier complet\n"
            "  à un décideur non-technique ?                            → C\n"
            "\n"
            "Est-ce que je veux qu'un vrai utilisateur final consomme\n"
            "  EFE depuis mon produit Python ?                          → D\n"
            "\n"
            "Est-ce que PLUSIEURS runtimes (Python + Go, ou Python +\n"
            "  Node, etc.) doivent consommer EFE ?                      → E\n"
            "\n"
            "Est-ce que je veux un SLO public avec on-call ?            → E"
        ),
        PageBreak(),
    ]


def section_milestones_per_letter() -> list:
    rows = [
        ["Lettre", "Jalon", "Critère binaire", "Semaine"],
        ["A", "J-A", "`efe classify` renvoie JSON valide",                    "S1"],
        ["B", "J-B", "30 prompts consécutifs sans crash",                     "S2"],
        ["C", "J-C", "Démo Claude passe live ≥ 3/4",                          "S3-5"],
        ["D", "J-D", "pytest projet hôte vert avec EFE intégré",              "S6-10"],
        ["E", "J-E", "Zero-downtime deploy + SLO tenu sur 7 jours",           "S11-14"],
    ]
    col_widths = [1.2 * cm, 1.5 * cm, 9 * cm, 2 * cm]
    letter_style = []
    for i, letter in enumerate("ABCDE"):
        row = i + 1
        letter_style.append(
            ("BACKGROUND", (0, row), (0, row), LETTER_COLORS[letter])
        )
        letter_style.append(("TEXTCOLOR", (0, row), (0, row), colors.white))
        letter_style.append(("FONT", (0, row), (0, row), "Helvetica-Bold", 10))
        letter_style.append(("ALIGN", (0, row), (0, row), "CENTER"))
    return [
        p("11. Jalons par lettre", H1),
        Spacer(1, 2 * mm),
        table(rows, col_widths, header=True, extra_style=letter_style),
        Spacer(1, 3 * mm),
        p(
            "Chaque jalon J-X est une <b>porte binaire</b> : soit le critère est "
            "observé, soit il ne l'est pas. Pas de semi-validation.",
            BODY,
        ),
        PageBreak(),
    ]


def section_gono_per_letter() -> list:
    rows = [
        ["Lettre", "Go", "No-Go"],
        ["A",
         "JSON valide, latence &lt; 2 s, help documenté",
         "Message d'erreur obscur, crash silencieux"],
        ["B",
         "Reload à chaud OK, historique, 30 req OK",
         "Fuite de connexion gRPC après :reload"],
        ["C",
         "Démo live ≥ 3/4, gère reject=True",
         "Seuils codés en dur dans le prompt"],
        ["D",
         "pytest hôte vert, shutdown propre, lazy init",
         "Import circulaire, +100ms startup, hardcoded env"],
        ["E",
         "SLO ≥ 99.5 %, zero-downtime, runbook testé",
         "Cold start casse les requêtes, pas de runbook"],
    ]
    col_widths = [1.2 * cm, 7 * cm, 7 * cm]
    letter_style = []
    for i, letter in enumerate("ABCDE"):
        row = i + 1
        letter_style.append(
            ("BACKGROUND", (0, row), (0, row), LETTER_COLORS[letter])
        )
        letter_style.append(("TEXTCOLOR", (0, row), (0, row), colors.white))
        letter_style.append(("FONT", (0, row), (0, row), "Helvetica-Bold", 10))
        letter_style.append(("ALIGN", (0, row), (0, row), "CENTER"))
    return [
        p("12. Critères Go / No-Go par lettre", H1),
        Spacer(1, 2 * mm),
        table(rows, col_widths, header=True, extra_style=letter_style),
        PageBreak(),
    ]


def section_dependencies_letters() -> list:
    rows = [
        ["Lettre", "Suppose", "Bloque", "Livrable EFE requis"],
        ["A", "—",              "B, C, D, E", "M9 + M10 + CLI de M11"],
        ["B", "A",              "C",          "REPL addition à M11"],
        ["C", "B",              "D",          "Bridges M11 (Claude)"],
        ["D", "C",              "E",          "efe_agent.py patterns"],
        ["E", "D",              "—",          "Docker + K8s + auth"],
    ]
    col_widths = [1.2 * cm, 2.5 * cm, 2.5 * cm, 9 * cm]
    letter_style = []
    for i, letter in enumerate("ABCDE"):
        row = i + 1
        letter_style.append(
            ("BACKGROUND", (0, row), (0, row), LETTER_COLORS[letter])
        )
        letter_style.append(("TEXTCOLOR", (0, row), (0, row), colors.white))
        letter_style.append(("FONT", (0, row), (0, row), "Helvetica-Bold", 10))
        letter_style.append(("ALIGN", (0, row), (0, row), "CENTER"))
    return [
        p("13. Dépendances entre lettres", H1),
        Spacer(1, 2 * mm),
        table(rows, col_widths, header=True, extra_style=letter_style),
        Spacer(1, 3 * mm),
        p(
            "La colonne « Livrable EFE requis » indique le module MVP qui doit être "
            "prêt côté moteur pour qu'une lettre soit techniquement atteignable. "
            "Tout effort d'adoption plus ambitieux que le niveau réellement livré est "
            "voué à l'échec.",
            BODY,
        ),
        PageBreak(),
    ]


def section_snippets() -> list:
    return [
        p("14. Annexe — snippets minimaux par lettre", H1),
        p(
            "Chaque snippet ci-dessous est <b>suffisant</b> pour atteindre le Go de "
            "sa lettre sur le catalog <code>support_triage.yaml</code>.",
            BODY,
        ),
        p("A — CLI one-shot", H2),
        code(
            "efe classify \\\n"
            "  --catalog catalogs/support_triage.yaml \\\n"
            "  --text \"J'ai été débité sans autorisation\" \\\n"
            "  --budget-ms 5000"
        ),
        p("B — REPL interactif", H2),
        code(
            "python -m efe_tool.repl --catalog catalogs/support_triage.yaml\n"
            "EFE> Ma carte a été piratée\n"
            "EFE> :thresholds\n"
            "EFE> :reload"
        ),
        p("C — Script / démo", H2),
        code(
            "from anthropic import Anthropic\n"
            "from efe_tool import AnchorCatalog, EfeClient, LocalSTEmbedder\n"
            "from efe_tool.bridges.claude import run_conversation_with_tool\n"
            "\n"
            "catalog = AnchorCatalog.load(\n"
            "    'catalogs/support_triage.yaml', embedder=LocalSTEmbedder())\n"
            "client = EfeClient.from_env()\n"
            "transcript = run_conversation_with_tool(\n"
            "    Anthropic(),\n"
            "    efe_client=client,\n"
            "    catalogs={catalog.name: catalog},\n"
            "    model='claude-sonnet-4-6',\n"
            "    system=SYSTEM_PROMPT,\n"
            "    user='Ma carte a été débitée sans mon consentement',\n"
            ")"
        ),
        p("D — FastAPI (extrait)", H2),
        code(
            "# efe_agent.py (dans le projet hôte)\n"
            "from functools import lru_cache\n"
            "import threading\n"
            "\n"
            "_lock = threading.Lock()\n"
            "_client = None\n"
            "\n"
            "def _get_efe_client():\n"
            "    global _client\n"
            "    with _lock:\n"
            "        if _client is None:\n"
            "            from efe_tool import EfeClient\n"
            "            _client = EfeClient.from_env()\n"
            "    return _client\n"
            "\n"
            "# main.py (FastAPI)\n"
            "from fastapi import FastAPI\n"
            "app = FastAPI()\n"
            "\n"
            "@app.on_event('startup')\n"
            "def _preload():\n"
            "    _get_efe_client()  # warm-up\n"
            "\n"
            "@app.on_event('shutdown')\n"
            "def _close():\n"
            "    if _client is not None:\n"
            "        _client.close()"
        ),
        p("E — Kubernetes (extrait)", H2),
        code(
            "apiVersion: apps/v1\n"
            "kind: Deployment\n"
            "metadata: { name: efe-server }\n"
            "spec:\n"
            "  replicas: 2\n"
            "  selector: { matchLabels: { app: efe-server } }\n"
            "  template:\n"
            "    spec:\n"
            "      containers:\n"
            "      - name: efe\n"
            "        image: efe-server:1.0.0\n"
            "        ports: [{ containerPort: 50051 }, { containerPort: 8080 }]\n"
            "        readinessProbe:\n"
            "          exec: { command: ['grpc-health-probe', '-addr=:50051'] }\n"
            "          initialDelaySeconds: 20\n"
            "        resources:\n"
            "          requests: { cpu: 500m, memory: 768Mi }\n"
            "          limits:   { cpu: 2,    memory: 1.5Gi }"
        ),
        Spacer(1, 1 * cm),
        p(
            "<b>Fin du document.</b>",
            ParagraphStyle("End", parent=BODY, alignment=TA_CENTER,
                           fontName="Helvetica-Oblique"),
        ),
    ]


# --------------------------------------------------------------------------
#  Footer with page number
# --------------------------------------------------------------------------

def _footer(canvas, doc):
    canvas.saveState()
    canvas.setFont("Helvetica", 8)
    canvas.setFillColor(colors.HexColor("#888888"))
    canvas.drawString(2 * cm, 1.2 * cm, "EFE — Roadmap par lettres · v1.0.0")
    canvas.drawRightString(
        A4[0] - 2 * cm, 1.2 * cm, f"Page {canvas.getPageNumber()}"
    )
    canvas.restoreState()


# --------------------------------------------------------------------------
#  Entrypoint
# --------------------------------------------------------------------------

def build(output_path: Path) -> None:
    doc = SimpleDocTemplate(
        str(output_path),
        pagesize=A4,
        leftMargin=2 * cm, rightMargin=2 * cm,
        topMargin=2 * cm, bottomMargin=2 * cm,
        title="EFE — Roadmap d'adoption par lettres",
        author="rodrigue.pekar@icloud.com",
    )
    story: list = []
    story += build_cover()
    story += build_toc()
    story += section_overview()
    story += section_compare_table()
    story += section_matrix()
    story += section_letter_A()
    story += section_letter_B()
    story += section_letter_C()
    story += section_letter_D()
    story += section_letter_E()
    story += section_path()
    story += section_decision_tree()
    story += section_milestones_per_letter()
    story += section_gono_per_letter()
    story += section_dependencies_letters()
    story += section_snippets()

    doc.build(story, onFirstPage=_footer, onLaterPages=_footer)


if __name__ == "__main__":
    here = Path(__file__).resolve().parent.parent
    out = here / "EFE_MVP_Roadmap_Lettres.pdf"
    build(out)
    print(f"Wrote: {out}")
