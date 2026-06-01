"""Build the EFE MVP Roadmap PDF.

Usage:
    python3 scripts/build_roadmap_pdf.py
Output:
    EFE_MVP_Roadmap.pdf at repo root.
"""

from __future__ import annotations

from pathlib import Path

from reportlab.lib import colors
from reportlab.lib.enums import TA_CENTER, TA_JUSTIFY, TA_LEFT
from reportlab.lib.pagesizes import A4
from reportlab.lib.styles import ParagraphStyle, getSampleStyleSheet
from reportlab.lib.units import cm, mm
from reportlab.platypus import (
    KeepTogether,
    PageBreak,
    Paragraph,
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
    "Title",
    parent=styles["Title"],
    fontName="Helvetica-Bold",
    fontSize=26,
    leading=32,
    alignment=TA_CENTER,
    textColor=colors.HexColor("#0B3D91"),
    spaceAfter=12,
)
SUBTITLE = ParagraphStyle(
    "Subtitle",
    parent=styles["Normal"],
    fontName="Helvetica",
    fontSize=14,
    leading=18,
    alignment=TA_CENTER,
    textColor=colors.HexColor("#333333"),
    spaceAfter=6,
)
META = ParagraphStyle(
    "Meta",
    parent=styles["Normal"],
    fontName="Helvetica-Oblique",
    fontSize=10,
    leading=14,
    alignment=TA_CENTER,
    textColor=colors.HexColor("#666666"),
)
H1 = ParagraphStyle(
    "H1",
    parent=styles["Heading1"],
    fontName="Helvetica-Bold",
    fontSize=18,
    leading=22,
    textColor=colors.HexColor("#0B3D91"),
    spaceBefore=12,
    spaceAfter=8,
)
H2 = ParagraphStyle(
    "H2",
    parent=styles["Heading2"],
    fontName="Helvetica-Bold",
    fontSize=14,
    leading=18,
    textColor=colors.HexColor("#1F4E79"),
    spaceBefore=10,
    spaceAfter=6,
)
H3 = ParagraphStyle(
    "H3",
    parent=styles["Heading3"],
    fontName="Helvetica-Bold",
    fontSize=11.5,
    leading=14,
    textColor=colors.HexColor("#2F5496"),
    spaceBefore=6,
    spaceAfter=4,
)
BODY = ParagraphStyle(
    "Body",
    parent=styles["Normal"],
    fontName="Helvetica",
    fontSize=10.5,
    leading=14,
    alignment=TA_JUSTIFY,
    spaceAfter=6,
)
BULLET = ParagraphStyle(
    "Bullet",
    parent=BODY,
    leftIndent=14,
    bulletIndent=4,
    spaceAfter=3,
)
CODE = ParagraphStyle(
    "Code",
    parent=styles["Normal"],
    fontName="Courier",
    fontSize=9,
    leading=11,
    leftIndent=8,
    backColor=colors.HexColor("#F2F4F7"),
    borderPadding=4,
    spaceAfter=6,
)
TOC_ITEM = ParagraphStyle(
    "TocItem",
    parent=BODY,
    alignment=TA_LEFT,
    spaceAfter=3,
    leftIndent=6,
)


# --------------------------------------------------------------------------
#  Small helpers
# --------------------------------------------------------------------------

def p(text: str, style=BODY) -> Paragraph:
    return Paragraph(text, style)


def bullets(items: list[str]) -> list[Paragraph]:
    return [Paragraph(f"• {it}", BULLET) for it in items]


def table(data: list[list[str]], col_widths: list[float], header=True) -> Table:
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
    t.setStyle(TableStyle(base))
    return t


def gantt_row(label: str, span: tuple[int, int], total: int = 14) -> list:
    """Build one row of a simple ASCII-like Gantt inside a Table."""
    cells = [label]
    for i in range(1, total + 1):
        cells.append("█" if span[0] <= i <= span[1] else "")
    return cells


# --------------------------------------------------------------------------
#  Content blocks
# --------------------------------------------------------------------------

def build_cover() -> list:
    return [
        Spacer(1, 4 * cm),
        p("EFE — Essaim Fluide Éphémère", TITLE),
        Spacer(1, 4 * mm),
        p("Roadmap du MVP", SUBTITLE),
        Spacer(1, 2 * mm),
        p("Plan opérationnel · 14 semaines · Rust + Python", SUBTITLE),
        Spacer(1, 3 * cm),
        p("Auteur : rodrigue.pekar@icloud.com", META),
        p("Date : 19 avril 2026", META),
        p("Version : 1.0.0", META),
        Spacer(1, 8 * cm),
        p("Document interne — cycle MVP", META),
        PageBreak(),
    ]


def build_toc() -> list:
    items = [
        "1. Vision et objectifs du MVP",
        "2. Principes directeurs",
        "3. Vue d'ensemble temporelle (Gantt 14 semaines)",
        "4. Phase 0 — Fondations (S0)",
        "5. Phase 1 — Socle d'embeddings (S1–S2)",
        "6. Phase 2 — Cône et fabrique d'agents (S3–S4)",
        "7. Phase 3 — Essaim et fusion (S5–S6)",
        "8. Phase 4 — Consensus et classification (S7–S8)",
        "9. Phase 5 — Services gRPC (S9–S10)",
        "10. Phase 6 — Client Python et bridges (S10–S11)",
        "11. Phase 7 — Observabilité (S12)",
        "12. Phase 8 — Hardening et démo (S13–S14)",
        "13. Jalons majeurs (J0 → J8)",
        "14. Matrice des livrables par sprint",
        "15. Dépendances inter-modules",
        "16. Risques et mitigations",
        "17. Critères Go / No-Go du MVP",
        "18. Post-MVP — horizon V1.1 et au-delà",
    ]
    return [
        p("Table des matières", H1),
        *[p(it, TOC_ITEM) for it in items],
        PageBreak(),
    ]


def section_vision() -> list:
    return [
        p("1. Vision et objectifs du MVP", H1),
        p(
            "L'Essaim Fluide Éphémère (EFE) est un système multi-agents qui, pour chaque requête, "
            "instancie une nuée de <b>1024 nano-agents</b> dans l'espace des embeddings, les laisse "
            "converger par fusion attentionnelle jusqu'au consensus, puis <b>se dissout</b>. Le MVP "
            "a pour vocation de valider industriellement le <b>Mode 1</b> — classification zero-shot "
            "appelée comme outil par un LLM hôte (Claude, GPT, …).",
            BODY,
        ),
        p("Objectifs mesurables du MVP", H2),
        *bullets([
            "<b>Précision</b> : ≥ 75 % sur le jeu de calibration <i>support_triage</i> "
            "(4 classes, FR + EN).",
            "<b>Latence</b> : p95 &lt; 2 s par classification, p99 &lt; 3 s, pour "
            "<i>paraphrase-multilingual-MiniLM-L12-v2</i> (dim 384).",
            "<b>Résilience</b> : 0 fuite d'agents après dissolution ; budget "
            "<code>budget_wall_clock_ms</code> strictement respecté.",
            "<b>Expérience développeur</b> : <code>pip install efe-tool</code> + "
            "<code>EfeClient.classify()</code> fonctionnels en moins de 10 lignes.",
            "<b>Observabilité</b> : golden signals (latence, erreurs, saturation, trafic) exportés "
            "sur <code>/metrics</code>.",
            "<b>Démo LLM-hôte</b> : <code>demo_agent_claude.py</code> obtient "
            "≥ 3 / 4 classifications correctes sur le scénario support bilingue.",
        ]),
        p("Hors périmètre MVP", H2),
        *bullets([
            "Mode 2 (génération coopérative d'opinions longues).",
            "Mode 3 (planification multi-étapes).",
            "Persistance longue durée d'un essaim (le MVP est par construction éphémère).",
            "Admin multi-tenant, RBAC, quotas.",
            "Embedders propriétaires (OpenAI, Voyage, Cohere) — reportés en V1.1.",
        ]),
        PageBreak(),
    ]


def section_principles() -> list:
    return [
        p("2. Principes directeurs", H1),
        p(
            "Sept principes invariants structurent les décisions d'ingénierie sur l'ensemble des "
            "14 semaines. Ils servent d'arbitrage en cas de conflit entre deux options d'implémentation.",
            BODY,
        ),
        *bullets([
            "<b>Éphémérité d'abord.</b> Aucun essaim ne survit à sa requête. Toute structure "
            "de données doit pouvoir être libérée synchroniquement à la fin d'un "
            "<code>SwarmController::run</code>.",
            "<b>Fingerprint partout.</b> Catalog, embedder, cône : chacun expose un "
            "<code>fingerprint()</code> déterministe. Un changement de fingerprint invalide le cache.",
            "<b>Budget strict.</b> Le client impose <code>budget_wall_clock_ms</code> ; le serveur "
            "coupe net. Pas de dépassement “best-effort”.",
            "<b>Zéro surprise côté LLM.</b> Le schéma JSON retourné à Claude est stable : "
            "<code>{label, confidence, reject, alignment_delta, rounds}</code>. "
            "Les seuils de décision restent <b>dans le catalog</b>, jamais codés en dur.",
            "<b>Séparation transport / logique.</b> Le client Python ne connaît que gRPC ; les "
            "bridges (Claude, etc.) ne connaissent que le client.",
            "<b>Tests avant vitesse.</b> Chaque sprint livre ses tests ; sans tests verts, la "
            "phase ne passe pas le jalon.",
            "<b>Docs en parallèle.</b> Un M<i>k</i>_ARCHITECTURE.md est livré en même temps "
            "que le module M<i>k</i>.",
        ]),
        PageBreak(),
    ]


def section_gantt() -> list:
    header = ["Phase / Module"] + [f"S{i}" for i in range(1, 15)]
    rows = [
        header,
        gantt_row("P0 — Bootstrap", (1, 1)),
        gantt_row("M1 Catalog", (1, 2)),
        gantt_row("M2 Embedder", (1, 2)),
        gantt_row("M3 Cone policy", (3, 4)),
        gantt_row("M4 Nano-agents", (3, 4)),
        gantt_row("M5 Swarm controller", (5, 6)),
        gantt_row("M6 Fusion", (5, 6)),
        gantt_row("M7 Consensus", (7, 8)),
        gantt_row("M8 Classification", (7, 8)),
        gantt_row("M9 Admin service", (9, 9)),
        gantt_row("M10 Mode 1 gRPC", (9, 10)),
        gantt_row("M11 Python / bridges", (10, 11)),
        gantt_row("M12 Observabilité", (12, 12)),
        gantt_row("P8 — Hardening / demo", (13, 14)),
    ]
    col_widths = [4.2 * cm] + [0.85 * cm] * 14
    return [
        p("3. Vue d'ensemble temporelle — Gantt 14 semaines", H1),
        p(
            "Chaque cellule pleine représente une semaine de travail actif sur le module. "
            "Les sprints de 2 semaines (S1–S2, S3–S4, …) sont délibérément alignés sur "
            "les jalons J1 → J8 décrits en section 13.",
            BODY,
        ),
        Spacer(1, 2 * mm),
        table(rows, col_widths, header=True),
        Spacer(1, 3 * mm),
        p(
            "<b>Lecture.</b> Les paires de modules travaillées en parallèle (M1+M2, M3+M4, "
            "M5+M6, M7+M8) correspondent à des duos fonctionnels : un producteur + un consommateur, "
            "développés ensemble pour éviter les interfaces <i>paper-only</i>.",
            BODY,
        ),
        PageBreak(),
    ]


# --------------------------------------------------------------------------
#  Phases
# --------------------------------------------------------------------------

def phase_block(
    number: int,
    title: str,
    weeks: str,
    goal: str,
    deliverables: list[str],
    exit_criteria: list[str],
    risks: list[str],
) -> list:
    story: list = [
        p(f"{number}. Phase {number - 3} — {title} ({weeks})", H1),
        p("Objectif de phase", H2),
        p(goal, BODY),
        p("Livrables", H2),
        *bullets(deliverables),
        p("Critères de sortie de phase", H2),
        *bullets(exit_criteria),
        p("Risques spécifiques", H2),
        *bullets(risks),
        PageBreak(),
    ]
    return story


def section_phase_0() -> list:
    return [
        p("4. Phase 0 — Fondations (S0)", H1),
        p("Objectif", H2),
        p(
            "Démarrer le cycle sur des bases sûres : workspace Rust compilable, "
            "environnement Python isolé, choix du modèle d'embedding gelé, "
            "<code>protoc</code> disponible, pipeline CI minimal actif.",
            BODY,
        ),
        p("Livrables", H2),
        *bullets([
            "<code>cargo build --release</code> vert sur <code>efe-core</code>, "
            "<code>efe-kernels</code>, <code>efe-server</code>.",
            "<code>.venv-efe</code> créé, <code>pip install -e clients/python</code> OK.",
            "Stubs protobuf générés via <code>grpcio-tools</code>.",
            "<code>ANCHOR_MODEL</code> figé : <code>paraphrase-multilingual-MiniLM-L12-v2</code> (dim 384).",
            "CI (GitHub Actions) exécute <code>cargo test</code> + <code>pytest</code> sur chaque PR.",
        ]),
        p("Critères de sortie (J0)", H2),
        *bullets([
            "<code>cargo test --workspace</code> : vert.",
            "<code>pytest clients/python/tests</code> : vert (même si vide au début).",
            "<code>python -c \"from efe_tool import EfeClient\"</code> : pas d'ImportError.",
        ]),
        p("Risques spécifiques", H2),
        *bullets([
            "Conflit <b>conda / venv</b> sur la machine dev (mitigation : "
            "<code>.venv-efe</code> strict, <code>conda deactivate</code> en préambule).",
            "Téléchargement lent du modèle multilingue au premier run "
            "(mitigation : preload dans <code>make dev-setup</code>).",
        ]),
        PageBreak(),
    ]


def section_phase_1() -> list:
    return phase_block(
        5, "Socle d'embeddings — M1 + M2", "S1–S2",
        goal=(
            "Livrer la capacité d'<b>embedder un texte</b> et de <b>charger un catalog YAML</b>. "
            "Sans cette capacité, aucune autre phase n'a de sens : M3, M4, M8 reposent toutes "
            "sur la sortie normalisée de M2 et les centroïdes de classes issus de M1."
        ),
        deliverables=[
            "<code>Embedder</code> (Protocol Python) + <code>LocalSTEmbedder</code> opérationnel "
            "avec normalisation L2 et <code>fingerprint()</code> stable.",
            "<code>AnchorCatalog.load(path)</code> → <code>.compile(embedder)</code> → "
            "<code>.save(.compiled.json)</code>.",
            "Cache compilé invalidé automatiquement sur changement de fingerprint "
            "(catalog ou embedder).",
            "Tests unitaires : idempotence <code>embed(x) == embed(x)</code>, "
            "<code>‖v‖₂ = 1.0 ± 1e-6</code>, roundtrip YAML ↔ compiled.",
        ],
        exit_criteria=[
            "J1 : sur le catalog <code>support_triage.yaml</code>, compilation &lt; 100 ms pour "
            "les ~30 canonical sentences, tests verts.",
            "Fingerprint de l'embedder reproductible entre runs et machines.",
        ],
        risks=[
            "Double préfixe <code>sentence-transformers/</code> dans le fingerprint "
            "(mitigation : normalisation stricte dans <code>LocalSTEmbedder.fingerprint()</code> — "
            "fix déjà appliqué).",
            "Coût mémoire du modèle multilingue (~500 MB) : mitigation = singleton lazy.",
        ],
    )


def section_phase_2() -> list:
    return phase_block(
        6, "Cône et fabrique d'agents — M3 + M4", "S3–S4",
        goal=(
            "Introduire la <b>géométrie sphérique</b> (politique de cône) et la <b>cellule élémentaire</b> "
            "de l'essaim (nano-agent). À la fin de la phase, on doit pouvoir instancier 1024 "
            "agents, chacun avec un état initial cohérent dans l'espace des embeddings."
        ),
        deliverables=[
            "Struct <code>Cone</code> Rust : projection, distance, inclusion.",
            "Politique de cône configurable via <code>cone_policy.max_projection_distance</code>.",
            "Struct <code>NanoAgent</code> : position, opinion courante, décroissance.",
            "Fabrique <code>AgentFabric::spawn(1024, seed)</code> déterministe sur <code>seed</code>.",
            "Benchmark : spawn 1024 agents &lt; 10 ms, empreinte &lt; 50 MB.",
        ],
        exit_criteria=[
            "J2 : un nano-agent isolé produit une <code>Opinion</code> cohérente à partir "
            "d'un centroid ; test de perturbation passe.",
            "Propriété testée : la projection sur le cône est idempotente "
            "(<code>project(project(x)) == project(x)</code>).",
        ],
        risks=[
            "Dérive numérique sur les projections itérées (mitigation : renormalisation "
            "systématique après chaque tick).",
            "Choix implicite du RNG : <code>rand_chacha::ChaCha20Rng</code> pour reproductibilité.",
        ],
    )


def section_phase_3() -> list:
    return phase_block(
        7, "Essaim et fusion — M5 + M6", "S5–S6",
        goal=(
            "Assembler les agents en essaim, définir la boucle de vie complète "
            "(spawn → tick → fusion → dissolution) et fournir la couche d'<b>attention pondérée</b> "
            "qui agrège les opinions en un centroïde par round."
        ),
        deliverables=[
            "<code>SwarmController::run(SwarmSpec) → SwarmResult</code>.",
            "Respect strict de <code>ResourceBudget</code> (budget_wall_clock_ms, max_rounds).",
            "Dissolution garantie : aucun agent résiduel après retour, testé via "
            "compteur global.",
            "Noyau de fusion : softmax à température configurable, moyenne pondérée "
            "optionnellement tronquée (trimmed mean).",
            "Invariances testées : permutation des agents ne change pas le centroïde ; "
            "<code>‖centroid‖₂ = 1</code> après renorm.",
        ],
        exit_criteria=[
            "J3 : essaim de 1024 agents tourne en ≤ 10 ms de temps CPU utile sur la "
            "machine de réf (M3 Pro).",
            "<code>cargo bench</code> dédié verrouillé comme régression-gate.",
        ],
        risks=[
            "Contention sur un <code>Vec&lt;Agent&gt;</code> partagé → passage à "
            "<code>rayon::par_iter</code> si bench &gt; cible.",
            "Saturation mémoire si <code>max_rounds</code> non borné : "
            "mitigation dans M5 via ResourceBudget.",
        ],
    )


def section_phase_4() -> list:
    return phase_block(
        8, "Consensus et classification — M7 + M8", "S7–S8",
        goal=(
            "Décider <b>quand</b> s'arrêter (M7) et <b>quoi répondre</b> (M8). "
            "M7 surveille variance, entropie, drift du centroïde, hystérésis. "
            "M8 mappe le centroïde final à la classe la plus proche, applique les seuils "
            "et émet un verdict <code>{label, confidence, reject}</code>."
        ),
        deliverables=[
            "<code>ConsensusConfig</code> calibré : <code>hysteresis_rounds=2</code>, "
            "<code>variance_threshold=0.05</code>, <code>confidence_threshold=0.50</code>, "
            "<code>entropy_fraction_max=0.98</code>.",
            "<code>ConfidenceScorer</code> déterministe (note : sortie effectivement constante "
            "~0.575 sur MVP — <b>accepté</b> et documenté comme trade-off).",
            "<code>Classifier::classify(centroid, catalog) → Label</code> via cosine argmax.",
            "Politique de seuils appliquée : high / medium / reject.",
        ],
        exit_criteria=[
            "J4 : sur le fixture de calibration, 95 % des requêtes convergent en ≤ 16 rounds.",
            "Précision top-1 ≥ 75 % sur le jeu de test support_triage.",
        ],
        risks=[
            "Confidence figé : non-bloquant pour MVP (Claude compense via clarification) — "
            "backlog V1.1 pour revoir <code>ConfidenceScorer</code>.",
            "Ambiguïté FR sur « carte bancaire » fraud/billing : mitigation produit "
            "(clarification LLM), pas technique.",
        ],
    )


def section_phase_5() -> list:
    return phase_block(
        9, "Services gRPC — M9 + M10", "S9–S10",
        goal=(
            "Exposer le moteur EFE sur le réseau. M9 (AdminService) gère l'enregistrement "
            "et la récupération des catalogs. M10 (Mode 1 service) expose la RPC "
            "<code>Classify(SwarmSpec) → ClassifyResult</code>."
        ),
        deliverables=[
            "Fichiers <code>.proto</code> figés et versionnés (<code>efe.mode1.v1</code>, "
            "<code>efe.admin.v1</code>).",
            "Serveur <code>efe-server</code> binde <code>127.0.0.1:50051</code> avec "
            "<code>EFE_BACKEND=mvp</code>.",
            "Mapping d'erreurs stable (DEADLINE_EXCEEDED vs INTERNAL vs INVALID_ARGUMENT).",
            "Tests d'intégration : round-trip client Python → serveur Rust → response.",
        ],
        exit_criteria=[
            "J5 : <code>grpcurl -plaintext 127.0.0.1:50051 efe.mode1.v1.Mode1/Classify</code> "
            "retourne un résultat valide sur le catalog support_triage.",
            "Registration optionnelle : <code>cone_policy.registered_in_m9</code> "
            "active la branche AdminService.",
        ],
        risks=[
            "Désynchronisation proto entre Rust et Python (mitigation : "
            "<code>make protos</code> unique source de vérité).",
            "Deadline gRPC mal mappée : déjà corrigé via inspection des détails "
            "(« convergence » → DEADLINE_EXCEEDED ; autre → transport).",
        ],
    )


def section_phase_6() -> list:
    return phase_block(
        10, "Client Python et bridges — M11", "S10–S11",
        goal=(
            "Livrer l'API que le monde extérieur va consommer : <code>EfeClient</code>, "
            "<code>AnchorCatalog</code>, <code>LocalSTEmbedder</code>, bridges LLM. "
            "C'est le <i>surface</i> du produit — tout se juge ici."
        ),
        deliverables=[
            "Package <code>efe-tool</code> installable via <code>pip install -e .</code>.",
            "<code>EfeClient.classify(text, catalog, budget_wall_clock_ms=5000)</code> "
            "avec lazy init du stub gRPC et fermeture propre.",
            "Bridge Claude : <code>run_conversation_with_tool</code>, tool-spec "
            "policy-free, gestion correcte de <code>stop_reason=max_tokens</code>.",
            "Helpers d'extraction : <code>extract_tool_result</code>, "
            "<code>extract_final_text</code>.",
            "Exemple E2E : <code>examples/demo_agent_claude.py</code>.",
            "37 tests unitaires / intégration verts (dont 16 dédiés bridges/claude.py).",
        ],
        exit_criteria=[
            "J6 : <code>python examples/demo_agent_claude.py</code> produit une "
            "classification + ACK Claude cohérent sans configuration manuelle "
            "au-delà de <code>ANTHROPIC_API_KEY</code> et <code>EFE_ENDPOINT</code>.",
        ],
        risks=[
            "Import cycle entre <code>client</code>, <code>spec_builder</code> et "
            "<code>bridges</code> (mitigation : règle d'import hiérarchique stricte).",
            "Incompatibilité de modèles Anthropic : <code>claude-sonnet-4-6</code> "
            "figé en <code>EFE_CLAUDE_MODEL</code>, override possible mais non-testé.",
        ],
    )


def section_phase_7() -> list:
    return phase_block(
        11, "Observabilité — M12", "S12",
        goal=(
            "Rendre le système <b>opérable</b>. Un MVP sans métriques est un MVP "
            "ingérable en production interne. On vise les <i>golden signals</i> "
            "et rien de plus."
        ),
        deliverables=[
            "Endpoint Prometheus <code>/metrics</code> sur <code>efe-server</code> : "
            "histogramme de latence, compteur d'erreurs par code gRPC, gauge "
            "d'essaims actifs.",
            "Logs structurés JSON via <code>tracing</code> + <code>tracing-subscriber</code>.",
            "Tracing OpenTelemetry optionnel (feature-flag <code>otel</code>).",
            "Dashboard Grafana minimal livré en <code>deploy/grafana/</code>.",
        ],
        exit_criteria=[
            "J7 : <code>curl :50051/metrics</code> retourne les 4 golden signals, "
            "exploités par un dashboard fonctionnel.",
        ],
        risks=[
            "Sur-coût de tracing sur chemin chaud (mitigation : "
            "<code>#[instrument(skip_all)]</code> sur les boucles d'agents).",
        ],
    )


def section_phase_8() -> list:
    return [
        p("12. Phase 8 — Hardening et démo (S13–S14)", H1),
        p("Objectif", H2),
        p(
            "Stabiliser, documenter, démontrer. Aucune nouvelle feature après S12. "
            "L'ensemble des deux semaines est consacré à la réduction de dette "
            "technique, à la complétion documentaire et à la livraison du <b>scénario "
            "de démonstration</b>.",
            BODY,
        ),
        p("Charges planifiées", H2),
        *bullets([
            "<b>S13</b> : finalisation <code>MODE1_ARCHITECTURE.md</code> et des 12 "
            "<code>Mk_ARCHITECTURE.md</code> ; passes de revue croisée.",
            "<b>S13</b> : bench-gates CI (latence p95 &lt; 2 s, mémoire &lt; 500 MB).",
            "<b>S14</b> : scénario démo support bilingue FR + EN, 4 cas "
            "(fraud / bug / question / billing) + 1 cas ambigu.",
            "<b>S14</b> : packaging <code>efe-tool</code> pour PyPI interne, tag "
            "<code>v1.0.0</code>.",
            "<b>S14</b> : revue sécurité minimale (secrets API, "
            "<code>ANTHROPIC_API_KEY</code> hors repo, <code>.env.example</code> fourni).",
        ]),
        p("Critères de sortie (J8 — Go MVP)", H2),
        *bullets([
            "Démo complète passe en live, ≥ 3 / 4 correct sur cas non-ambigus.",
            "CI au vert sur 5 runs consécutifs.",
            "Documentation complète : README + 12 architectures + roadmap (ce document).",
            "Aucun <code>TODO</code> bloquant dans le code livré.",
        ]),
        PageBreak(),
    ]


# --------------------------------------------------------------------------
#  Milestones / deliverables matrix / dependencies / risks / go-nogo
# --------------------------------------------------------------------------

def section_milestones() -> list:
    rows = [
        ["ID", "Nom", "Semaine", "Gate"],
        ["J0", "Bootstrap vert",                        "S0",  "cargo build + pytest OK"],
        ["J1", "Embedder + catalog prêts",              "S2",  "fingerprint stable, roundtrip OK"],
        ["J2", "Nano-agent fonctionnel",                "S4",  "opinion cohérente, idempotence cône"],
        ["J3", "Essaim 1024 en < 10 ms",                "S6",  "bench régression-gate"],
        ["J4", "Consensus convergent",                  "S8",  "≤ 16 rounds sur 95 % fixture"],
        ["J5", "gRPC Mode1/Classify",                   "S10", "grpcurl OK sur support_triage"],
        ["J6", "Client Python + bridge Claude",         "S11", "demo_agent_claude.py vert"],
        ["J7", "/metrics + dashboard",                  "S12", "4 golden signals visibles"],
        ["J8", "MVP livrable — démo de sortie",         "S14", "≥ 3/4 démo + CI ×5 vertes"],
    ]
    col_widths = [1.0 * cm, 5.5 * cm, 1.6 * cm, 7.8 * cm]
    return [
        p("13. Jalons majeurs (J0 → J8)", H1),
        p(
            "Les jalons fonctionnent en <b>garde</b> : aucun travail de la phase suivante "
            "ne démarre tant que le jalon courant n'est pas officiellement validé.",
            BODY,
        ),
        Spacer(1, 2 * mm),
        table(rows, col_widths, header=True),
        PageBreak(),
    ]


def section_deliverables_matrix() -> list:
    rows = [
        ["Module", "S1-2", "S3-4", "S5-6", "S7-8", "S9", "S10-11", "S12", "S13-14"],
        ["M1 Catalog",          "●", "○", "·", "·", "·", "·", "·", "◉"],
        ["M2 Embedder",         "●", "·", "·", "·", "·", "·", "·", "◉"],
        ["M3 Cone policy",      "·", "●", "○", "·", "·", "·", "·", "◉"],
        ["M4 Nano-agents",      "·", "●", "○", "·", "·", "·", "·", "◉"],
        ["M5 Swarm ctrl",       "·", "·", "●", "○", "·", "·", "·", "◉"],
        ["M6 Fusion",           "·", "·", "●", "○", "·", "·", "·", "◉"],
        ["M7 Consensus",        "·", "·", "·", "●", "·", "·", "·", "◉"],
        ["M8 Classif dispatch", "·", "·", "·", "●", "·", "○", "·", "◉"],
        ["M9 Admin",            "·", "·", "·", "·", "●", "·", "·", "◉"],
        ["M10 Mode1 gRPC",      "·", "·", "·", "·", "○", "●", "·", "◉"],
        ["M11 Client Python",   "·", "·", "·", "·", "·", "●", "·", "◉"],
        ["M12 Observabilité",   "·", "·", "·", "·", "·", "·", "●", "◉"],
    ]
    col_widths = [4.2 * cm] + [1.5 * cm] * 8
    return [
        p("14. Matrice des livrables par sprint", H1),
        p(
            "Légende : <b>●</b> développement principal · <b>○</b> intégration partielle · "
            "<b>◉</b> hardening / bench / doc · <b>·</b> pas d'activité.",
            BODY,
        ),
        Spacer(1, 2 * mm),
        table(rows, col_widths, header=True),
        Spacer(1, 3 * mm),
        p(
            "La colonne <b>S13-14</b> est intentionnellement pleine : chaque module subit "
            "une passe finale de tests de charge, de doc et de revue croisée pendant le "
            "hardening.",
            BODY,
        ),
        PageBreak(),
    ]


def section_dependencies() -> list:
    rows = [
        ["Module", "Dépend de", "Consommé par"],
        ["M1 Catalog",          "M2",                       "M8, M10, M11"],
        ["M2 Embedder",         "—",                        "M1, M6, M8, M11"],
        ["M3 Cone policy",      "M2",                       "M4, M5, M6"],
        ["M4 Nano-agents",      "M2, M3",                   "M5"],
        ["M5 Swarm ctrl",       "M3, M4, M6",               "M10"],
        ["M6 Fusion",           "M2, M3",                   "M5, M7"],
        ["M7 Consensus",        "M6",                       "M5, M10"],
        ["M8 Classif dispatch", "M1, M2",                   "M10"],
        ["M9 Admin",            "M1",                       "M10, M11"],
        ["M10 Mode1 gRPC",      "M1, M5, M7, M8, M9",       "M11"],
        ["M11 Client Python",   "M10 (via proto)",          "Utilisateurs / bridges LLM"],
        ["M12 Observabilité",   "M5, M10",                  "Ops"],
    ]
    col_widths = [4.5 * cm, 5.5 * cm, 5.5 * cm]
    return [
        p("15. Dépendances inter-modules", H1),
        p(
            "Le graphe est volontairement peu profond (5 niveaux max) pour préserver "
            "une compilation incrémentale rapide et limiter le coût d'un refactor "
            "en aval.",
            BODY,
        ),
        Spacer(1, 2 * mm),
        table(rows, col_widths, header=True),
        Spacer(1, 3 * mm),
        p("Chemins critiques (plus long chemin de bout en bout)", H3),
        *bullets([
            "<b>CP-A</b> : M2 → M3 → M4 → M5 → M10 → M11 (6 maillons, dicte la durée du MVP).",
            "<b>CP-B</b> : M2 → M1 → M8 → M10 → M11 (5 maillons, validation de la précision).",
        ]),
        PageBreak(),
    ]


def section_risks() -> list:
    rows = [
        ["ID", "Risque", "Probabilité", "Impact", "Mitigation"],
        ["R1",
         "Confidence scorer effectivement constant (~0.575)",
         "Confirmée",
         "Moyen",
         "Accepté en MVP ; LLM-hôte compense par clarification ; reprise en V1.1."],
        ["R2",
         "Ambiguïté FR « carte bancaire » fraud/billing",
         "Moyenne",
         "Faible",
         "Enrichir canonical_sentences FR en V1.1 ; doc claire côté produit."],
        ["R3",
         "Latence gRPC &gt; 2 s sous charge",
         "Faible",
         "Élevé",
         "<code>budget_wall_clock_ms</code>, warm start du modèle, bench-gates CI."],
        ["R4",
         "Téléchargement modèle multilingue (~500 MB)",
         "Certaine",
         "Faible",
         "Preload via <code>make dev-setup</code> ; cache <code>~/.cache/huggingface</code>."],
        ["R5",
         "Drift catalog ↔ cache compilé",
         "Moyenne",
         "Moyen",
         "Fingerprint strict ; invalidation auto au moindre écart."],
        ["R6",
         "Incompatibilité Anthropic SDK / modèle",
         "Faible",
         "Moyen",
         "Modèle figé <code>claude-sonnet-4-6</code>, override explicite."],
        ["R7",
         "Fuite d'agents en cas de panic Rust",
         "Faible",
         "Élevé",
         "<code>Drop</code> explicite + test de dissolution ; <code>#[test]</code> dédié."],
        ["R8",
         "Dépendance implicite au conda de la machine dev",
         "Moyenne",
         "Faible",
         ".venv-efe isolé ; README spécifie <code>conda deactivate</code>."],
    ]
    col_widths = [1.1 * cm, 5.0 * cm, 2.2 * cm, 1.8 * cm, 5.8 * cm]
    return [
        p("16. Risques et mitigations", H1),
        Spacer(1, 2 * mm),
        table(rows, col_widths, header=True),
        Spacer(1, 3 * mm),
        p(
            "R1 et R2 sont <b>connus et acceptés</b> au moment de l'écriture de cette roadmap : "
            "ils ont été observés en intégration Claude mais ne bloquent pas le MVP. "
            "R3 et R7 sont les deux risques à surveillance active en phase de hardening.",
            BODY,
        ),
        PageBreak(),
    ]


def section_go_nogo() -> list:
    return [
        p("17. Critères Go / No-Go du MVP", H1),
        p(
            "Le passage en Go se prononce en fin de S14, sur la base stricte de la grille "
            "suivante. Tout No-Go sur un critère « bloquant » reporte la livraison d'un sprint.",
            BODY,
        ),
        p("Critères bloquants", H2),
        *bullets([
            "<b>Build</b> : <code>cargo build --release -p efe-server</code> vert, zéro warning.",
            "<b>Tests</b> : <code>cargo test --workspace</code> &amp; <code>pytest</code> verts "
            "sur 5 runs CI consécutifs.",
            "<b>Latence</b> : p95 &lt; 2 s sur fixture support_triage, p99 &lt; 3 s.",
            "<b>Précision</b> : ≥ 75 % top-1 sur fixture.",
            "<b>Démo</b> : <code>demo_agent_claude.py</code> produit ≥ 3 / 4 correct live.",
            "<b>Observabilité</b> : dashboard Grafana affiche les 4 golden signals.",
            "<b>Dissolution</b> : 0 agent résiduel sur 10 000 requêtes synthétiques.",
        ]),
        p("Critères non-bloquants (nice-to-have)", H2),
        *bullets([
            "Bridge OpenAI en plus de Claude.",
            "Embedder <i>OpenAI text-embedding-3-small</i> branché.",
            "AdminService exposé publiquement (MVP : interne seulement).",
            "Mode benchmark reproductible sur cloud.",
        ]),
        PageBreak(),
    ]


def section_post_mvp() -> list:
    return [
        p("18. Post-MVP — horizon V1.1 et au-delà", H1),
        p(
            "Une fois le MVP livré (J8), quatre chantiers sont pré-qualifiés pour "
            "la V1.1 (horizon ~Q3 2026) :",
            BODY,
        ),
        *bullets([
            "<b>C1 — Scorer de confiance dynamique.</b> Retravailler "
            "<code>ConfidenceScorer</code> pour qu'il reflète la dispersion réelle du "
            "centroïde plutôt qu'une constante structurelle. Sujet d'ingénierie + R&amp;D.",
            "<b>C2 — Embedders multiples.</b> Brancher OpenAI, Voyage, Cohere sous "
            "l'interface <code>Embedder</code> existante. Catalogs multi-embedder avec "
            "fingerprint composite.",
            "<b>C3 — Mode 2.</b> Génération coopérative : l'essaim produit un texte "
            "plutôt qu'une classification. Nécessite M6 étendu (fusion lexicale) + M8bis.",
            "<b>C4 — Déploiement multi-tenant.</b> AdminService avec RBAC, quotas "
            "par projet, isolation des catalogs.",
        ]),
        p("Horizon V1.2 (indicatif)", H2),
        *bullets([
            "Sharding de l'essaim sur plusieurs workers.",
            "Persistance opt-in d'un essaim pour conversation longue.",
            "Mode 3 — planification multi-étapes (decompose &amp; plan).",
        ]),
        Spacer(1, 1 * cm),
        p(
            "<b>Fin du document.</b>",
            ParagraphStyle("End", parent=BODY, alignment=TA_CENTER, fontName="Helvetica-Oblique"),
        ),
    ]


# --------------------------------------------------------------------------
#  Page decoration — footer with page number
# --------------------------------------------------------------------------

def _footer(canvas, doc):
    canvas.saveState()
    canvas.setFont("Helvetica", 8)
    canvas.setFillColor(colors.HexColor("#888888"))
    canvas.drawString(2 * cm, 1.2 * cm, "EFE — Roadmap MVP · v1.0.0")
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
        leftMargin=2 * cm,
        rightMargin=2 * cm,
        topMargin=2 * cm,
        bottomMargin=2 * cm,
        title="EFE — Roadmap MVP",
        author="rodrigue.pekar@icloud.com",
    )

    story: list = []
    story += build_cover()
    story += build_toc()
    story += section_vision()
    story += section_principles()
    story += section_gantt()
    story += section_phase_0()
    story += section_phase_1()
    story += section_phase_2()
    story += section_phase_3()
    story += section_phase_4()
    story += section_phase_5()
    story += section_phase_6()
    story += section_phase_7()
    story += section_phase_8()
    story += section_milestones()
    story += section_deliverables_matrix()
    story += section_dependencies()
    story += section_risks()
    story += section_go_nogo()
    story += section_post_mvp()

    doc.build(story, onFirstPage=_footer, onLaterPages=_footer)


if __name__ == "__main__":
    here = Path(__file__).resolve().parent.parent
    out = here / "EFE_MVP_Roadmap.pdf"
    build(out)
    print(f"Wrote: {out}")
