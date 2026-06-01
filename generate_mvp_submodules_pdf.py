"""Générateur PDF — EFE MVP : Sous-Modules Ultra-Détaillés.

Produit :  EFE_MVP_Sous_Modules.pdf  (~30-50 pages)

Contenu :
    - Page de garde
    - Table des matières
    - Introduction & Architecture globale
    - 12 modules (M01 → M12) avec :
        * Vue d'ensemble
        * Scope MVP (In / Out)
        * Sous-modules
        * Sous-sous-modules
        * Interfaces publiques (tables)
        * Algorithmes clés
        * Structures de données
        * Tests
        * Dépendances
        * Critères d'acceptation
    - Plan de sprints (6 sprints de 2 semaines)
    - Critères d'acceptation globaux
    - Annexes (glossaire, matrice RACI, métriques)

Usage :
    python3 generate_mvp_submodules_pdf.py
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
    SimpleDocTemplate,
    Spacer,
    Table,
    TableStyle,
)


# =========================================================================
# Stylesheet
# =========================================================================

_SS = getSampleStyleSheet()


def _style(name: str, parent: str, **kw) -> ParagraphStyle:
    base = _SS[parent]
    return ParagraphStyle(name=name, parent=base, **kw)


STYLES = {
    "Cover": _style("Cover", "Title", fontSize=28, leading=34,
                    alignment=TA_CENTER, textColor=colors.HexColor("#0B3D91")),
    "CoverSub": _style("CoverSub", "Normal", fontSize=14, leading=20,
                       alignment=TA_CENTER, textColor=colors.HexColor("#333333")),
    "H1": _style("H1", "Heading1", fontSize=18, leading=22, spaceBefore=18,
                 spaceAfter=10, textColor=colors.HexColor("#0B3D91")),
    "H2": _style("H2", "Heading2", fontSize=14, leading=18, spaceBefore=12,
                 spaceAfter=6, textColor=colors.HexColor("#1C4E9E")),
    "H3": _style("H3", "Heading3", fontSize=12, leading=15, spaceBefore=8,
                 spaceAfter=4, textColor=colors.HexColor("#2A6DB4")),
    "H4": _style("H4", "Heading4", fontSize=11, leading=14, spaceBefore=6,
                 spaceAfter=3, textColor=colors.HexColor("#444444")),
    "Body": _style("Body", "Normal", fontSize=10, leading=14,
                   alignment=TA_JUSTIFY, spaceAfter=6),
    "Bullet": _style("Bullet", "Normal", fontSize=10, leading=13,
                     leftIndent=14, bulletIndent=4, spaceAfter=2),
    "Code": _style("Code", "Code", fontSize=8.5, leading=11,
                   backColor=colors.HexColor("#F2F4F8"),
                   borderColor=colors.HexColor("#CCCCCC"),
                   borderWidth=0.5, borderPadding=4,
                   leftIndent=8, rightIndent=8),
    "Caption": _style("Caption", "Normal", fontSize=9, leading=11,
                      alignment=TA_CENTER, textColor=colors.HexColor("#555555"),
                      spaceAfter=8),
    "TOC": _style("TOC", "Normal", fontSize=10, leading=16, leftIndent=12),
}


# =========================================================================
# Helpers
# =========================================================================

STORY: list = []


def p(text: str, style: str = "Body") -> None:
    STORY.append(Paragraph(text, STYLES[style]))


def h(level: int, text: str) -> None:
    p(text, f"H{level}")


def bullets(items: list[str]) -> None:
    for it in items:
        STORY.append(Paragraph("• " + it, STYLES["Bullet"]))


def spacer(h_mm: float = 4) -> None:
    STORY.append(Spacer(1, h_mm * mm))


def pagebreak() -> None:
    STORY.append(PageBreak())


def table(header: list[str], rows: list[list[str]],
          col_widths=None, caption: str | None = None) -> None:
    # Wrap cells in Paragraph for automatic word wrap.
    body_style = ParagraphStyle("tbody", parent=_SS["Normal"],
                                fontSize=8.5, leading=11)
    head_style = ParagraphStyle("thead", parent=_SS["Normal"],
                                fontSize=9, leading=11,
                                fontName="Helvetica-Bold",
                                textColor=colors.white)
    wrapped_header = [Paragraph(c, head_style) for c in header]
    wrapped_rows = [[Paragraph(c, body_style) for c in row] for row in rows]
    data = [wrapped_header] + wrapped_rows
    t = Table(data, colWidths=col_widths, repeatRows=1, hAlign="LEFT")
    t.setStyle(TableStyle([
        ("BACKGROUND", (0, 0), (-1, 0), colors.HexColor("#0B3D91")),
        ("VALIGN", (0, 0), (-1, -1), "TOP"),
        ("GRID", (0, 0), (-1, -1), 0.25, colors.HexColor("#999999")),
        ("ROWBACKGROUNDS", (0, 1), (-1, -1),
         [colors.white, colors.HexColor("#F6F8FB")]),
        ("LEFTPADDING", (0, 0), (-1, -1), 4),
        ("RIGHTPADDING", (0, 0), (-1, -1), 4),
        ("TOPPADDING", (0, 0), (-1, -1), 3),
        ("BOTTOMPADDING", (0, 0), (-1, -1), 3),
    ]))
    STORY.append(t)
    if caption:
        p(caption, "Caption")
    else:
        spacer(3)


# =========================================================================
# 1. COVER
# =========================================================================

def section_cover() -> None:
    STORY.append(Spacer(1, 55 * mm))
    p("ESSAIM FLUIDE ÉPHÉMÈRE", "Cover")
    spacer(3)
    p("Sous-Modules du MVP — Spécification Ultra-Détaillée", "CoverSub")
    spacer(30)
    p("Plateforme hybride Python + Rust — 12 modules<br/>"
      "Inférence multi-agents via espace latent partagé",
      "CoverSub")
    spacer(40)
    p("Document technique — Version MVP 1.0", "CoverSub")
    p("Avril 2026", "CoverSub")
    p("Auteur : Équipe EFE Core", "CoverSub")
    pagebreak()


# =========================================================================
# 2. TABLE OF CONTENTS
# =========================================================================

def section_toc() -> None:
    h(1, "Table des matières")
    items = [
        ("1.  Introduction", "3"),
        ("2.  Architecture globale du MVP", "4"),
        ("3.  Nomenclature et conventions", "5"),
        ("4.  M01 — Platform Core (efe_platform)", "6"),
        ("5.  M02 — Worker Runtime (efe_worker)", "9"),
        ("6.  M03 — Registry (efe_registry)", "12"),
        ("7.  M04 — Orchestration (efe_orch)", "15"),
        ("8.  M05 — Fine-Tuning (efe_ft)", "18"),
        ("9.  M06 — Inference (efe_inference)", "21"),
        ("10. M07 — Rollout (efe_rollout)", "24"),
        ("11. M08 — Gate (efe_gate)", "27"),
        ("12. M09 — Drift Detection (efe_drift)", "30"),
        ("13. M10 — Scheduling &amp; Continuous Learning", "33"),
        ("14. M11 — Security, Multi-tenancy, Compliance", "36"),
        ("15. M12 — Integration with EFE Swarm", "39"),
        ("16. Plan de sprints (12 semaines)", "42"),
        ("17. Critères d'acceptation globaux du MVP", "44"),
        ("18. Annexes (glossaire, RACI, métriques)", "46"),
    ]
    for title, page in items:
        STORY.append(Paragraph(
            f'{title}<br/><font color="#AAAAAA">{"." * 60}</font> p. {page}',
            STYLES["TOC"]))
    pagebreak()


# =========================================================================
# 3. INTRODUCTION
# =========================================================================

def section_intro() -> None:
    h(1, "1.  Introduction")
    p("L'<b>Essaim Fluide Éphémère (EFE)</b> est une plateforme d'inférence "
      "multi-agents qui fait coopérer des familles de sous-modèles spécialisés "
      "dans un <b>espace latent partagé</b>. Plutôt qu'un agent monolithique, "
      "EFE instancie à la volée un essaim de <i>contributeurs</i> dont les "
      "trajectoires latentes sont contraintes par un <b>cône d'alignement</b> "
      "et agrégées par un <b>réconciliateur déterministe</b>. Le MVP couvre "
      "12 modules qui, ensemble, livrent la chaîne complète de bout en bout.")
    p("Ce document fixe, pour chaque module, la liste nominale des "
      "<b>sous-modules</b> et <b>sous-sous-modules</b> retenus dans le "
      "périmètre MVP. Chaque section précise les interfaces publiques, "
      "les algorithmes clés, les dépendances inter-modules, la stratégie "
      "de tests et les critères d'acceptation qui débloquent le "
      "passage en production.")
    h(2, "Objectifs du MVP")
    bullets([
        "Faire tourner un essaim de 4 à 32 contributeurs sur une même requête, "
        "avec verdict convergent sous 500 ms (p50).",
        "Garantir la <b>multi-tenant isolation</b> par clé chiffrée et par "
        "domaine HMAC (5 préfixes, 27 tags, 0 collision prouvée).",
        "Offrir un <b>fine-tuning continu</b> sur des adaptateurs LoRA "
        "versionnés dans un registre de confiance (M03).",
        "Détecter et réagir aux <b>dérives statistiques</b> par un pipeline "
        "M09 → M10 → M04 capable de re-entraîner sous 24 h.",
        "Exposer une surface gRPC unique (M12) qui absorbe les 11 modules "
        "amont derrière une façade sécurisée.",
    ])
    h(2, "Hors scope MVP")
    bullets([
        "Support GPU multi-nœuds pour l'inférence (reporté au post-MVP).",
        "Fédération cross-région (le MVP est mono-région, multi-AZ).",
        "Marketplace d'adaptateurs tiers (reporté).",
        "Interface utilisateur graphique (seulement CLI + gRPC au MVP).",
    ])
    pagebreak()


# =========================================================================
# 4. ARCHITECTURE
# =========================================================================

def section_architecture() -> None:
    h(1, "2.  Architecture globale du MVP")
    p("Les 12 modules s'organisent en trois <b>couches horizontales</b> "
      "(Données, Compute, Contrôle) et deux <b>plans verticaux</b> "
      "(Sécurité, Observabilité). La figure logique ci-dessous résume "
      "les dépendances de compile-time.")
    table(
        header=["Couche", "Modules", "Rôle"],
        rows=[
            ["Données",     "M01, M03",                "Types socle, registre versionné"],
            ["Compute",     "M02, M05, M06, M07",      "Exécution, entraînement, rollout"],
            ["Contrôle",    "M04, M08, M09, M10",      "Orchestration, gate, drift, sched"],
            ["Sécurité",    "M11",                     "PDP, PII, audit, multi-tenancy"],
            ["Intégration", "M12",                     "Surface gRPC + absorption M1-M11"],
        ],
        col_widths=[30 * mm, 40 * mm, 95 * mm],
    )
    h(2, "Flux nominal d'une requête de classification")
    p("Un appel <code>SwarmClient.classify(text, …)</code> arrive sur M12. "
      "La chaîne complète est :")
    bullets([
        "<b>M12.SwarmClient</b> — vérifie le PDP M11, sanitize le prompt "
        "(PII linter), attribue un <i>request_id</i>.",
        "<b>M12.ContractAdapter</b> — résout le <i>catalog_key</i> vers un "
        "<i>SwarmSpec</i> complet (population, cône, budget).",
        "<b>M04.Orchestrator</b> — planifie l'essaim, appelle M02 pour "
        "allouer les contributeurs.",
        "<b>M02.Worker</b> — exécute chaque contributeur, diffuse les "
        "latents projetés sur le cône M03.",
        "<b>M06.Inference</b> — agrège et réconcilie ; produit "
        "<code>ClassifyResponse</code>.",
        "<b>M08.Gate</b> — contrôle la sortie (score, seuils, policy).",
        "<b>M11.Audit</b> — scelle un <i>AuditDraft</i> HMAC dans la "
        "chaîne tamper-evident.",
    ])
    pagebreak()


# =========================================================================
# 5. NOMENCLATURE
# =========================================================================

def section_nomenclature() -> None:
    h(1, "3.  Nomenclature et conventions")
    p("Toutes les classes publiques respectent le style <b>CamelCase</b>, "
      "les noms de modules pythons sont en <b>snake_case</b> avec le "
      "préfixe <code>efe_</code>. Les identifiants cryptographiques sont "
      "en <b>kebab-case</b> (<code>efe-sched</code>, <code>efe-sec</code>…).")
    h(2, "Préfixes HMAC et domain-tags")
    table(
        header=["Préfixe", "Utilisation", "Nb. de tags"],
        rows=[
            ["efe-drift",    "Scellés de DriftResult (M09)",         "3"],
            ["efe-rollout",  "Canary hash + stage attestation (M07)", "4"],
            ["efe-sched",    "fire_key + JobSpec canonical (M10)",   "5"],
            ["efe-sec",      "AuditDraft HMAC-SHA256 (M11)",         "7"],
            ["efe-m12",      "Catalog-key hash, bundle hash (M12)",  "8"],
        ],
        col_widths=[30 * mm, 90 * mm, 30 * mm],
    )
    h(2, "Statuts de maturité")
    bullets([
        "<b>MVP-Core</b> : livré, testé, sans dépendance externe non résolue.",
        "<b>MVP-Beta</b> : livré, testé, surveillé par feature-flag.",
        "<b>Post-MVP</b> : dans l'architecture cible, non livré au MVP.",
    ])
    pagebreak()


# =========================================================================
# 6. MODULE TEMPLATE
# =========================================================================

def module_section(
    *,
    mid: str,
    title: str,
    package: str,
    overview: list[str],
    scope_in: list[str],
    scope_out: list[str],
    sub_modules: list[tuple[str, str]],
    sub_sub: list[tuple[str, list[tuple[str, str]]]],
    interfaces: list[tuple[str, str, str]],
    algorithms: list[tuple[str, str]],
    data_structures: list[tuple[str, str]],
    tests: list[tuple[str, str]],
    dependencies_up: list[str],
    dependencies_down: list[str],
    acceptance: list[str],
) -> None:
    h(1, f"{mid} — {title} (<font face='Courier'>{package}</font>)")

    h(2, "Vue d'ensemble")
    for para in overview:
        p(para)

    h(2, "Scope MVP")
    h(3, "Inclus")
    bullets(scope_in)
    h(3, "Exclu (post-MVP)")
    bullets(scope_out)

    h(2, "Sous-modules")
    table(
        header=["Sous-module", "Rôle"],
        rows=[[n, d] for n, d in sub_modules],
        col_widths=[55 * mm, 110 * mm],
    )

    h(2, "Sous-sous-modules détaillés")
    for section, items in sub_sub:
        h(3, section)
        for name, desc in items:
            p(f"<b>{name}</b> — {desc}")

    h(2, "Interfaces publiques")
    table(
        header=["Méthode / Classe", "Signature", "Rôle"],
        rows=[[m, s, r] for m, s, r in interfaces],
        col_widths=[45 * mm, 65 * mm, 55 * mm],
    )

    h(2, "Algorithmes clés")
    for name, desc in algorithms:
        h(3, name)
        p(desc)

    h(2, "Structures de données")
    table(
        header=["Dataclass", "Rôle"],
        rows=[[n, r] for n, r in data_structures],
        col_widths=[55 * mm, 110 * mm],
    )

    h(2, "Stratégie de tests")
    table(
        header=["Fichier", "Couverture"],
        rows=[[f, s] for f, s in tests],
        col_widths=[70 * mm, 95 * mm],
    )

    h(2, "Dépendances")
    h(3, "Amont (consommées)")
    bullets(dependencies_up or ["Aucune — module socle."])
    h(3, "Aval (consommateurs)")
    bullets(dependencies_down or ["Aucun — module terminal."])

    h(2, "Critères d'acceptation")
    bullets(acceptance)

    pagebreak()


# =========================================================================
# MODULE DEFINITIONS
# =========================================================================

def section_m01() -> None:
    module_section(
        mid="M01", title="Platform Core", package="efe_platform",
        overview=[
            "M01 est le <b>module socle</b> : il définit les types de base "
            "(vecteurs latents, configurations de bot, spécifications "
            "d'essaim) et fournit un <code>SwarmClient</code> minimal utilisé "
            "comme référence de compatibilité ascendante.",
            "Historiquement, M01 a précédé M12 ; dans le MVP il conserve "
            "son API publique pour permettre aux callers existants de "
            "basculer progressivement vers <code>efe_swarm_integration</code> "
            "via l'adaptateur <code>M01PlatformAdapter</code>.",
            "Le module est <b>Pure-Python</b>, sans dépendance externe "
            "forte hors <code>numpy</code>. Les invariants cryptographiques "
            "ne sont pas assurés ici (délégués à M11).",
        ],
        scope_in=[
            "Types <code>Bot</code>, <code>SwarmSpec</code>, <code>LatentVector</code>.",
            "Fonctions utilitaires de normalisation L2 et de projection "
            "sur un simplexe borné.",
            "Client minimal <code>SwarmClient</code> (v1) — façade legacy.",
        ],
        scope_out=[
            "Cône d'alignement paramétrique complet (porté par M03).",
            "Schéma versionné de <code>SwarmSpec</code> — au MVP, figé.",
            "Exporteur OpenTelemetry (via M04).",
        ],
        sub_modules=[
            ("efe_platform.types",     "Dataclasses + enums publics."),
            ("efe_platform.spec",      "Construction et validation SwarmSpec."),
            ("efe_platform.embedding", "Norme L2, projection, utilitaires."),
            ("efe_platform.legacy",    "SwarmClient v1 + signatures gelées."),
            ("efe_platform.errors",    "Hiérarchie d'exceptions de base."),
        ],
        sub_sub=[
            ("efe_platform.types", [
                ("Bot",         "Dataclass gelée : identifiant, template_code, "
                                "paramètres. Immuable, hashable."),
                ("SwarmSpec",   "Population + cône (facultatif) + problème "
                                "embeddé ; sérialisable JSON."),
                ("LatentVector","Wrapper <code>np.ndarray</code> typé avec "
                                "dimensions contraintes à dim_latent."),
            ]),
            ("efe_platform.spec", [
                ("validate",        "Vérifie population.weights ≥ 0 et somme ≤ 1.0."),
                ("normalize_embed", "Retourne un embedding L2-normalisé."),
                ("freeze",          "Produit une version immuable pour hash."),
            ]),
            ("efe_platform.legacy", [
                ("SwarmClient.describe_spec", "Renvoie le dict dérivé du spec, "
                                              "identique v1."),
                ("SwarmClient.execute",       "Wrapper autour d'un backend pluggable."),
            ]),
        ],
        interfaces=[
            ("Bot",          "@dataclass(frozen=True)", "Identité d'un contributeur."),
            ("SwarmSpec",    "@dataclass(frozen=True)", "Spécification essaim."),
            ("LatentVector", "NDArray[float32]",        "Vecteur dans l'espace latent."),
            ("validate",     "(spec) -&gt; list[str]",  "Erreurs lisibles."),
            ("SwarmClient",  "class",                   "Client legacy."),
        ],
        algorithms=[
            ("Normalisation L2 stable",
             "Implémentation naïve <code>x / ||x||₂</code> avec epsilon=1e-9 "
             "pour éviter division par zéro ; repli sur vecteur unitaire "
             "canonique si ||x|| &lt; eps."),
            ("Projection simplexe",
             "Algorithme <i>Duchi 2008</i> en O(d log d) pour projeter sur "
             "{x ≥ 0, Σx = 1} ; utilisé par les poids de population."),
        ],
        data_structures=[
            ("Bot",           "Immutable ; attributs : id, template_code, params."),
            ("SwarmSpec",     "Immutable ; dim_latent, problem_embedding, population."),
            ("PopulationMix", "Tuple d'entrées pondérées (<code>PopulationEntry</code>)."),
        ],
        tests=[
            ("tests_platform/test_types.py", "20 cas — construction, égalité, hash, pickle."),
            ("tests_platform/test_spec.py",  "15 cas — validate() sur specs corrompus."),
            ("tests_platform/test_embed.py", "12 cas — normalisation + projection."),
        ],
        dependencies_up=[],
        dependencies_down=[
            "M04 — utilise SwarmSpec pour planifier l'essaim.",
            "M06 — attend un LatentVector typé.",
            "M12 — importe SwarmSpec pour l'adaptateur legacy.",
        ],
        acceptance=[
            "Couverture ≥ 95 % sur <code>efe_platform/types.py</code>.",
            "Aucune dépendance vers M02-M12 (vérifié par <code>grep</code> de CI).",
            "Sérialisation JSON round-trip pour tout <code>SwarmSpec</code> valide.",
            "Temps moyen de <code>validate</code> &lt; 200 µs sur specs typiques.",
        ],
    )


def section_m02() -> None:
    module_section(
        mid="M02", title="Worker Runtime", package="efe_worker",
        overview=[
            "M02 héberge le runtime d'exécution d'un contributeur unique. "
            "Un <i>worker</i> est un processus léger qui charge un modèle "
            "(ou un adaptateur LoRA), traite une slice d'inférence, et "
            "diffuse son latent projeté sur le cône partagé.",
            "Le MVP supporte deux familles : <b>CPU</b> (inférence batch "
            "courte, &lt; 50 ms) et <b>GPU mono-device</b> (latences "
            "exploitables sur L40S / H100, aussi LoRA-hot-swap).",
            "Le worker expose un <b>heartbeat</b> toutes les 500 ms ; un "
            "<code>HeartbeatTimeout</code> au-delà de 3 battements manqués "
            "déclenche l'éviction par M04.",
        ],
        scope_in=[
            "Cycle de vie : BOOT → READY → RUN → DRAIN → STOP.",
            "Chargement LoRA depuis M03 via digest signé.",
            "Pool d'exécution batché avec file prioritaire.",
            "Health endpoint TCP + métriques Prometheus.",
        ],
        scope_out=[
            "Multi-GPU sharding (post-MVP).",
            "Quantisation dynamique (post-MVP, Q8 prévu).",
            "Live-migration entre hôtes.",
        ],
        sub_modules=[
            ("efe_worker.runtime",   "Boucle principale, lifecycle."),
            ("efe_worker.pool",      "BatchPool et scheduler interne."),
            ("efe_worker.adapters",  "LoRA hot-swap, LoRA registry cache."),
            ("efe_worker.heartbeat", "Client heartbeat vers M04."),
            ("efe_worker.errors",    "HeartbeatTimeout, LoadFailure, …"),
            ("efe_worker.metrics",   "Exposition Prometheus (/metrics)."),
        ],
        sub_sub=[
            ("efe_worker.runtime", [
                ("WorkerLoop",  "Boucle asynchrone basée sur asyncio."),
                ("WorkerState", "State-machine BOOT/READY/RUN/DRAIN/STOP."),
                ("drain_hook",  "Attend la complétion avant SIGTERM."),
            ]),
            ("efe_worker.pool", [
                ("BatchPool",     "Agrège les requêtes pour batcher."),
                ("PriorityQueue", "File min-heap par deadline (ms)."),
                ("flush_on_idle", "Déclenche un batch si idle &gt; 8 ms."),
            ]),
            ("efe_worker.adapters", [
                ("LoraCache",  "Cache LRU bornée (taille = RAM/3)."),
                ("LoraLoader", "Télécharge, vérifie digest, hot-swap."),
                ("unload_all", "Libère la VRAM au shutdown."),
            ]),
            ("efe_worker.heartbeat", [
                ("HeartbeatClient", "Envoie un datagramme UDP toutes 500 ms."),
                ("on_timeout",      "Callback invoqué après 3 misses."),
            ]),
        ],
        interfaces=[
            ("WorkerLoop.run",        "async () -&gt; None",          "Boucle principale."),
            ("BatchPool.submit",      "(req) -&gt; Future[Output]",   "Push une requête."),
            ("LoraLoader.load",       "(digest, bytes) -&gt; Adapter","Charge + vérifie."),
            ("HeartbeatClient.pulse", "() -&gt; None",                "Beat UDP ad-hoc."),
        ],
        algorithms=[
            ("Batching adaptatif",
             "Le <code>BatchPool</code> ferme un batch si (a) taille ≥ "
             "batch_max, (b) deadline la plus serrée - now &lt; 8 ms, ou "
             "(c) idle &gt; flush_ms. Compromis throughput / p99 latency."),
            ("LoRA hot-swap",
             "Le chargement est atomique : l'ancien adapter reste pointé "
             "tant que le nouveau n'est pas prêt ; swap via "
             "<code>compare_and_swap</code> sur un <code>contextvars</code> "
             "du WorkerLoop."),
        ],
        data_structures=[
            ("WorkerConfig", "CPU/GPU, batch_max, flush_ms, heartbeat_period."),
            ("WorkerState",  "Enum 5 états."),
            ("InferenceJob", "input_tensor, deadline_ns, tenant_id, request_id."),
            ("WorkerStats",  "Throughput/s, latences p50/p95/p99."),
        ],
        tests=[
            ("tests_worker/test_runtime.py",   "18 cas — lifecycle et drain."),
            ("tests_worker/test_pool.py",      "14 cas — batching adaptatif."),
            ("tests_worker/test_adapters.py",  "11 cas — LoRA cache, digest mismatch."),
            ("tests_worker/test_heartbeat.py", "8 cas  — timeout, recovery."),
        ],
        dependencies_up=[
            "M01 — types <code>LatentVector</code>.",
            "M03 — registre d'adapters pour digest et bytes.",
        ],
        dependencies_down=[
            "M04 — heartbeat consommé par l'orchestrateur.",
            "M06 — produit les latents pour l'inférence.",
        ],
        acceptance=[
            "Heartbeat perdu ⇒ éviction en &lt; 2 s (mesuré).",
            "LoRA hot-swap &lt; 120 ms sur L40S sans invalider les batchs "
            "en cours.",
            "p99 d'une slice batchée ≤ 35 ms sur CPU, ≤ 8 ms sur GPU.",
            "Aucun leak VRAM après 10 000 swaps (testé en CI nightly).",
        ],
    )


def section_m03() -> None:
    module_section(
        mid="M03", title="Registry", package="efe_registry",
        overview=[
            "M03 est le <b>registre versionné</b> de tous les artefacts : "
            "modèles de base, adaptateurs LoRA, cônes d'alignement, "
            "embeddings de problème. Chaque artefact reçoit un "
            "<code>digest</code> SHA-256 immuable et une chaîne de "
            "signatures M11.",
            "Le MVP stocke les blobs sur S3 (ou MinIO en dev), et les "
            "métadonnées dans PostgreSQL. Les lectures sont idempotentes "
            "et le <i>cas chaud</i> est servi depuis un cache local "
            "(LRU, TTL 1 h).",
        ],
        scope_in=[
            "CRUD sur artefacts ; verrouillage optimiste par version.",
            "Vérification de digest à la lecture (refus si mismatch).",
            "Pré-signature S3 pour lectures distantes.",
            "Tags (ex. <code>canary</code>, <code>prod</code>).",
        ],
        scope_out=[
            "Réplication cross-région (post-MVP).",
            "Marketplace public (post-MVP).",
            "Quota par utilisateur — seulement par tenant au MVP.",
        ],
        sub_modules=[
            ("efe_registry.store",  "Couche persistante (Postgres)."),
            ("efe_registry.blobs",  "Abstraction BlobStore (S3, MinIO, FS)."),
            ("efe_registry.api",    "Façade publique + dataclasses."),
            ("efe_registry.cache",  "Cache LRU local + invalidation."),
            ("efe_registry.errors", "ArtifactNotFound, DigestMismatch."),
        ],
        sub_sub=[
            ("efe_registry.store", [
                ("ArtifactRow",     "Mapping Postgres : id, tenant, digest, "
                                    "mime, size, created_at, tags[]."),
                ("VersionLock",     "Verrou optimiste par (artifact_id, version)."),
                ("retention_sweep", "Daemon de purge respectant les tags."),
            ]),
            ("efe_registry.blobs", [
                ("S3BlobStore",   "Implémentation boto3 ; MPU &gt; 32 MiB."),
                ("FileBlobStore", "Utilisé en dev et tests (fs POSIX)."),
                ("presign_get",   "URL signée 15 min."),
            ]),
            ("efe_registry.api", [
                ("Registry.put", "put(bytes, meta) -&gt; Artifact (digest calculé)."),
                ("Registry.get", "get(id, version) -&gt; Artifact."),
                ("Registry.tag", "tag(id, version, t) avec vérification droits."),
            ]),
        ],
        interfaces=[
            ("Registry.put", "(bytes, meta) -&gt; Artifact", "Upload + hash."),
            ("Registry.get", "(id, version) -&gt; Artifact", "Lecture vérifiée."),
            ("Registry.tag", "(id, v, tag) -&gt; None",      "Annote version."),
            ("Artifact",     "@dataclass(frozen=True)",      "Vue immuable."),
        ],
        algorithms=[
            ("Streaming SHA-256",
             "Upload en <i>chunks</i> de 64 KiB, hash calculé à la volée ; "
             "rejet de l'upload si digest annoncé ≠ digest réel."),
            ("Invalidation multi-instance",
             "Pub/Sub Redis sur <code>registry.invalidate</code> ; chaque "
             "instance purge son LRU local pour la clé annoncée."),
        ],
        data_structures=[
            ("Artifact",      "id, version, digest, mime, size, tags, tenant_id."),
            ("BlobRef",       "bucket + key, utilisé par M12 artefact syncer."),
            ("RegistryStats", "total_artifacts, total_bytes, top_tags."),
        ],
        tests=[
            ("tests_registry/test_store.py", "22 cas — CRUD, verrous, transactions."),
            ("tests_registry/test_blobs.py", "14 cas — S3/FS, MPU, presign."),
            ("tests_registry/test_cache.py", "10 cas — LRU, TTL, invalidation."),
            ("tests_registry/test_api.py",   "19 cas — digest, tags, droits."),
        ],
        dependencies_up=["M01 — types de base."],
        dependencies_down=[
            "M02 — charge les LoRAs depuis M03.",
            "M05 — écrit les adaptateurs fraîchement entraînés.",
            "M07 — tag canary ⇒ prod.",
            "M12 — <code>M03ArtifactBlobSource</code>.",
        ],
        acceptance=[
            "PUT d'un artefact de 256 MiB sans erreur (S3 MPU).",
            "GET avec digest falsifié ⇒ <code>DigestMismatch</code>.",
            "Cache hit-rate ≥ 80 % après 1 min de charge stable.",
            "Retention sweep ne supprime <i>jamais</i> un tag "
            "<code>prod</code>.",
        ],
    )


def section_m04() -> None:
    module_section(
        mid="M04", title="Orchestration", package="efe_orch",
        overview=[
            "M04 est le <b>cœur de contrôle</b> : il reçoit une requête "
            "d'essaim, planifie les contributeurs sur les workers M02 "
            "disponibles, supervise le heartbeat, et agrège les résultats "
            "pour le réconciliateur M06.",
            "L'orchestrateur applique un <b>backpressure</b> token-bucket "
            "par tenant, un <b>bulkhead</b> par population, et un "
            "<b>circuit-breaker</b> par worker pool. Les trois "
            "mécanismes sont composés dans un pipeline déterministe.",
        ],
        scope_in=[
            "Planification synchrone (classify) et asynchrone (execute).",
            "Heartbeat monitor + éviction automatique.",
            "Backpressure par tenant (tokens/s configurable).",
            "Bulkhead par <code>TemplateCode</code>.",
            "Circuit-breaker avec état half-open après 30 s.",
        ],
        scope_out=[
            "Auto-scaling horizontal des workers (post-MVP, via K8s HPA).",
            "Planification prédictive ML-based.",
        ],
        sub_modules=[
            ("efe_orch.orchestrator", "Boucle de planification."),
            ("efe_orch.backpressure", "Token-bucket multi-tenant."),
            ("efe_orch.bulkhead",     "Isolation par population."),
            ("efe_orch.circuit",      "Circuit-breaker par pool."),
            ("efe_orch.monitor",      "Collecte heartbeat M02."),
            ("efe_orch.errors",       "OrchError, BackpressureRejected."),
        ],
        sub_sub=[
            ("efe_orch.orchestrator", [
                ("Orchestrator.schedule", "Assigne N contributeurs à N workers."),
                ("Orchestrator.drain",    "Vidange propre avant shutdown."),
                ("LatentCollector",       "Agrège latents jusqu'à seuil."),
            ]),
            ("efe_orch.backpressure", [
                ("TokenBucket", "Refill configurable, burst=2x steady."),
                ("TenantQuota", "Limite tokens/s par tenant."),
                ("on_reject",   "Émet <code>BackpressureRejected</code>."),
            ]),
            ("efe_orch.bulkhead", [
                ("Bulkhead",   "Pool bornée par <code>TemplateCode</code>."),
                ("PoolStats",  "Occupation, attente moyenne."),
            ]),
            ("efe_orch.circuit", [
                ("CircuitBreaker",       "Open/Closed/Half-open ; fenêtres 30 s."),
                ("trip_on_failure_rate", "Seuil 50 % d'échecs sur 10 req."),
            ]),
        ],
        interfaces=[
            ("Orchestrator.schedule", "async (spec, ctx) -&gt; Plan",   "Planifie un essaim."),
            ("Orchestrator.cancel",   "async (request_id) -&gt; None",  "Annule une run."),
            ("TokenBucket.try_take",  "(n=1) -&gt; bool",               "Réserve tokens."),
            ("CircuitBreaker.call",   "(fn, *a) -&gt; Any",             "Wrap protégé."),
        ],
        algorithms=[
            ("Planification déterministe",
             "Tri stable par (deadline, tenant_priority, contributor_weight) ; "
             "assignation round-robin avec affinité si un worker a déjà "
             "l'adapter chargé."),
            ("Circuit-breaker half-open",
             "Après tripping, un seul probe passe toutes les 30 s ; succès "
             "3 fois consécutifs = re-close ; échec ⇒ re-open."),
        ],
        data_structures=[
            ("Plan",         "Liste ordonnée de (worker_id, Bot, deadline_ns)."),
            ("Heartbeat",    "worker_id, timestamp, in_flight, queued."),
            ("CircuitState", "Enum Open/Closed/HalfOpen + timers."),
        ],
        tests=[
            ("tests_orch/test_orchestrator.py", "24 cas — plan, cancel, drain."),
            ("tests_orch/test_backpressure.py", "15 cas — buckets, multi-tenants."),
            ("tests_orch/test_bulkhead.py",     "11 cas — saturation, isolation."),
            ("tests_orch/test_circuit.py",      "13 cas — tripping, probes."),
        ],
        dependencies_up=["M01, M02, M03"],
        dependencies_down=[
            "M06 — reçoit les latents à réconcilier.",
            "M08 — consulte Plan pour la gate policy.",
            "M10 — consomme BackpressureRejected pour replanification.",
        ],
        acceptance=[
            "Backpressure rejette en &lt; 5 ms (mesuré).",
            "Bulkhead n'affame jamais un tenant premium (test adversarial).",
            "Circuit-breaker relâche correctement après 30 s.",
            "Cancel propagé à tous les workers en &lt; 50 ms.",
        ],
    )


def section_m05() -> None:
    module_section(
        mid="M05", title="Fine-Tuning", package="efe_ft",
        overview=[
            "M05 gère le <b>fine-tuning continu</b> des adaptateurs LoRA. "
            "Le MVP supporte uniquement le LoRA rank ≤ 32 sur la base "
            "figée ; le full-finetune est exclu du périmètre.",
            "Le module consomme un dataset versionné (M03), entraîne un "
            "adapter, et dépose le binaire signé dans le registre avec "
            "un tag <code>candidate</code> qui sera promu en "
            "<code>canary</code> par M07 après validation M08.",
        ],
        scope_in=[
            "Entraînement LoRA single-GPU.",
            "Checkpointing toutes 500 steps.",
            "Evaluation hold-out automatique.",
            "Métadonnées complètes (hyper-params, seed, git-sha).",
        ],
        scope_out=[
            "Distributed Data Parallel (post-MVP).",
            "Quantization-Aware Training.",
            "Hyperparam search AutoML.",
        ],
        sub_modules=[
            ("efe_ft.trainer",    "Boucle d'entraînement."),
            ("efe_ft.dataset",    "Chargement des datasets M03."),
            ("efe_ft.loss",       "Loss functions paramétriques."),
            ("efe_ft.checkpoint", "Sauvegarde / reprise."),
            ("efe_ft.eval",       "Hold-out + métriques."),
            ("efe_ft.errors",     "FtError, CheckpointCorrupted."),
        ],
        sub_sub=[
            ("efe_ft.trainer", [
                ("Trainer.fit",    "Entraîne un LoRA sur N epochs."),
                ("Trainer.resume", "Reprend depuis un checkpoint."),
                ("grad_clip",      "Clip L2 max-norm = 1.0."),
            ]),
            ("efe_ft.dataset", [
                ("DatasetSpec",     "dataset_id + version + split."),
                ("StreamingLoader", "Lazy-loading depuis M03."),
                ("TokenizedBatch",  "Batch tokens + masks + labels."),
            ]),
            ("efe_ft.loss", [
                ("CrossEntropy",   "Baseline pour classification."),
                ("LabelSmoothing", "eps=0.1 par défaut."),
            ]),
            ("efe_ft.checkpoint", [
                ("Checkpoint.save",  "State-dict + optim + RNG seeds."),
                ("verify_integrity", "SHA-256 comparé au manifest."),
            ]),
        ],
        interfaces=[
            ("Trainer.fit",     "(dataset, cfg) -&gt; Artifact",  "Entraîne LoRA."),
            ("Trainer.eval",    "(adapter, split) -&gt; Metrics", "Evalue hold-out."),
            ("DatasetSpec",     "@dataclass(frozen=True)",        "Référence dataset."),
            ("Checkpoint.save", "(path, state)",                  "Sérialise."),
        ],
        algorithms=[
            ("LoRA Merging déterministe",
             "Les poids ΔW = B·A sont fusionnés dans W via "
             "<code>W' = W + α·B·A</code> avec α configurable. Opération "
             "reproductible à bit-près via seeds PyTorch fixées."),
            ("Evaluation stratifiée",
             "Hold-out stratifié par label avec <code>StratifiedKFold</code> ; "
             "métriques : accuracy, F1-macro, calibration ECE."),
        ],
        data_structures=[
            ("TrainConfig", "lr, steps, warmup, grad_clip, lora_rank, lora_alpha."),
            ("Checkpoint",  "step, state_dict, optim_state, rng_seeds."),
            ("Metrics",     "loss, accuracy, f1, ece, latency_ms."),
        ],
        tests=[
            ("tests_ft/test_trainer.py",    "20 cas — fit, resume, reproductibilité."),
            ("tests_ft/test_dataset.py",    "12 cas — streaming, splits."),
            ("tests_ft/test_loss.py",       "8 cas — CE, LabelSmoothing."),
            ("tests_ft/test_checkpoint.py", "10 cas — sauvegarde, corruption."),
        ],
        dependencies_up=["M01, M03"],
        dependencies_down=[
            "M07 — récupère le <code>candidate</code> pour promotion.",
            "M09 — consulte Metrics pour détection de dérive.",
        ],
        acceptance=[
            "Reproductibilité à bit-près (seeds fixées) sur 3 runs.",
            "Checkpoint recovery en &lt; 30 s après crash.",
            "Metrics hold-out exposées dans l'Artifact final.",
            "Aucun leak mémoire après 1000 steps (testé en CI).",
        ],
    )


def section_m06() -> None:
    module_section(
        mid="M06", title="Inference", package="efe_inference",
        overview=[
            "M06 réalise la <b>réconciliation des latents</b> et la "
            "<b>décodification finale</b> en un verdict structuré. "
            "Le MVP supporte deux réconciliateurs : moyenne pondérée "
            "par confiance, et <i>Mixture-of-Experts</i> déterministe.",
            "Le module expose une API haut-niveau "
            "<code>InferenceService.classify(text, settings)</code> qui "
            "est également consommée par M12.",
        ],
        scope_in=[
            "Réconciliation moyenne pondérée.",
            "MoE déterministe (top-k=2).",
            "Confiance calibrée (Platt scaling offline).",
            "Support multi-langue (tokenizer sentencepiece).",
        ],
        scope_out=[
            "Mélange bayésien non-paramétrique (post-MVP).",
            "Sampling temperature-scheduled (post-MVP).",
        ],
        sub_modules=[
            ("efe_inference.service",     "Façade publique."),
            ("efe_inference.reconciler",  "Réconciliation de latents."),
            ("efe_inference.decoder",     "Décodage final → ClassifyResponse."),
            ("efe_inference.calibration", "Platt scaling, ECE."),
            ("efe_inference.errors",      "InferenceError, DecoderFailure."),
        ],
        sub_sub=[
            ("efe_inference.service", [
                ("InferenceService.classify", "Point d'entrée synchrone."),
                ("InferenceSettings",         "Config : task_id, budget, rounds."),
            ]),
            ("efe_inference.reconciler", [
                ("MeanReconciler", "Moyenne L2-renormalisée."),
                ("MoEReconciler",  "Top-k softmax pondéré par probas."),
            ]),
            ("efe_inference.decoder", [
                ("ClassifyResponse", "label, confidence, verdict, latency_ms."),
                ("argmax_calibrated","Renvoie top-1 post-calibration."),
            ]),
            ("efe_inference.calibration", [
                ("PlattCalibrator",            "Ajuste a, b sur hold-out."),
                ("expected_calibration_error", "Bucket 10 bins."),
            ]),
        ],
        interfaces=[
            ("InferenceService.classify", "async (text, settings) -&gt; resp", "Classe un prompt."),
            ("MeanReconciler.aggregate",  "(latents, weights) -&gt; latent",   "Agrège."),
            ("ClassifyResponse",          "@dataclass(frozen=True)",            "Verdict final."),
            ("PlattCalibrator.apply",     "(logits) -&gt; probs",               "Calibre."),
        ],
        algorithms=[
            ("MoE déterministe",
             "Top-k sur les poids α_i (k=2) ; softmax normalisé sur les "
             "<i>selected experts</i> ; sortie = <code>Σ α_i·output_i</code> "
             "avec <code>α_i</code> renormalisé à 1."),
            ("Platt scaling",
             "Régression logistique à 1 paramètre (<code>σ(a·logit + b)</code>) "
             "ajustée par L-BFGS sur le hold-out ; mis à jour mensuellement."),
        ],
        data_structures=[
            ("ClassifyResponse",  "label, confidence, verdict, latency_ms, trace_id."),
            ("InferenceSettings", "task_id, swarm_budget_ms, max_rounds."),
            ("ReconcileResult",   "latent agrégé + vector de contributions."),
        ],
        tests=[
            ("tests_inference/test_service.py",     "18 cas — end-to-end classify."),
            ("tests_inference/test_reconciler.py",  "14 cas — Mean, MoE, edge cases."),
            ("tests_inference/test_decoder.py",     "10 cas — argmax, confiance."),
            ("tests_inference/test_calibration.py", "9 cas  — Platt, ECE."),
        ],
        dependencies_up=["M01, M02, M03, M04"],
        dependencies_down=[
            "M08 — reçoit ClassifyResponse pour gate.",
            "M12 — M06InferenceAdapter.",
        ],
        acceptance=[
            "p50 classify &lt; 80 ms, p99 &lt; 400 ms sur corpus bench.",
            "ECE &lt; 3 % post-calibration sur tenant de référence.",
            "MoE et Mean réconciliateurs interchangeables via config.",
            "Aucun non-déterminisme observé sur 10 000 répétitions.",
        ],
    )


def section_m07() -> None:
    module_section(
        mid="M07", title="Rollout", package="efe_rollout",
        overview=[
            "M07 gouverne le <b>déploiement progressif</b> d'un "
            "<code>candidate</code> en <code>canary</code> puis en "
            "<code>prod</code>. Il implémente un rollout en 4 étages "
            "(1 % → 5 % → 25 % → 100 %) avec attestation HMAC à "
            "chaque passage.",
            "Chaque étage embarque un <b>canary hash</b> calculé sur la "
            "concaténation (digest_model, digest_dataset, config) et "
            "scellé par la clé <code>efe-rollout</code>.",
        ],
        scope_in=[
            "4 étages configurables par stratégie.",
            "Auto-rollback si M08 décline 2 stages consécutifs.",
            "Attestation HMAC par stage.",
            "Historique complet (replay-able).",
        ],
        scope_out=[
            "Rollout cross-région (post-MVP).",
            "A/B testing statistique formel (post-MVP).",
        ],
        sub_modules=[
            ("efe_rollout.controller", "Logique état → état."),
            ("efe_rollout.canary",     "Hash canary, attestation."),
            ("efe_rollout.strategy",   "Stratégies de progression."),
            ("efe_rollout.audit",      "Enregistre dans M11."),
            ("efe_rollout.errors",     "ArtifactFetchFailed, RolloutStalled."),
        ],
        sub_sub=[
            ("efe_rollout.controller", [
                ("RolloutController.advance",  "Avance d'un stage si OK."),
                ("RolloutController.rollback", "Revient au stage précédent."),
                ("Stage",                      "Enum : S1, S5, S25, S100."),
            ]),
            ("efe_rollout.canary", [
                ("canary_hash",  "SHA-256 sur les 3 digests."),
                ("attest_stage", "HMAC efe-rollout/canary-hash/stage."),
            ]),
            ("efe_rollout.strategy", [
                ("ProgressiveStrategy", "Bump stage si p95 latency + "
                                        "error_rate stables ≥ 10 min."),
                ("ManualStrategy",      "Passage manuel via CLI."),
            ]),
        ],
        interfaces=[
            ("RolloutController.advance",  "() -&gt; Stage",             "Passe au stage suivant."),
            ("RolloutController.rollback", "() -&gt; Stage",             "Revient arrière."),
            ("canary_hash",                "(d1,d2,cfg) -&gt; h",        "Calcule hash."),
            ("attest_stage",               "(hash, stage) -&gt; bytes",  "Signature HMAC."),
        ],
        algorithms=[
            ("Progression multistage",
             "Checklist par stage : (a) error_rate &lt; seuil_S, "
             "(b) p95 latency &lt; target_S, (c) M08 pass. "
             "Trois validations ⇒ bump ; deux échecs ⇒ rollback."),
            ("Attestation HMAC",
             "tag = HMAC(key, b'efe-rollout:canary-hash:' || hash || "
             "b':' || stage_byte) — prévient les attaques par rejeu "
             "inter-stages."),
        ],
        data_structures=[
            ("RolloutPlan",       "stages[], current_stage, started_at."),
            ("StageAttestation",  "hash, stage, hmac_tag, timestamp."),
            ("RolloutMetrics",    "error_rate, p95_ms, qps."),
        ],
        tests=[
            ("tests_rollout/test_controller.py", "16 cas — advance/rollback."),
            ("tests_rollout/test_canary.py",     "9 cas  — hash + HMAC."),
            ("tests_rollout/test_strategy.py",   "12 cas — progressive, manuel."),
            ("tests_rollout/test_audit.py",      "8 cas  — AuditDraft."),
        ],
        dependencies_up=["M03, M05, M08, M11"],
        dependencies_down=["M12 — RolloutPreflight (describe_spec)."],
        acceptance=[
            "Rollback complet en &lt; 60 s après décision.",
            "Canary hash stable (même config ⇒ même hash).",
            "Attestation replay-proof — impossible de rejouer S25 → S100 "
            "sans stage intermédiaire.",
            "Audit trail complet avec hmac-chain vérifiable.",
        ],
    )


def section_m08() -> None:
    module_section(
        mid="M08", title="Gate", package="efe_gate",
        overview=[
            "M08 est la <b>gate de décision</b> : pour chaque réponse "
            "M06, elle applique un jeu de règles policy-as-code qui "
            "décide <i>admit</i>, <i>hold</i>, ou <i>reject</i>. Le MVP "
            "couvre 12 règles (seuils de confiance, contenus sensibles, "
            "compatibilité cône).",
            "La gate est <b>côté serveur</b> ; une version client "
            "allégée (règles lisibles uniquement) est exposée via M12 "
            "pour du pré-check.",
        ],
        scope_in=[
            "12 règles MVP, toutes testables unitairement.",
            "Décision finale en &lt; 2 ms (mesuré).",
            "Artefact-driven : les règles sont dans un Artifact M03.",
            "Audit de chaque décision (M11).",
        ],
        scope_out=[
            "Règles ML apprises (post-MVP).",
            "Règles par tenant (post-MVP — le MVP est global).",
        ],
        sub_modules=[
            ("efe_gate.engine",   "Exécuteur de règles."),
            ("efe_gate.rules",    "Catalogue des règles MVP."),
            ("efe_gate.decision", "Dataclass Decision + audit."),
            ("efe_gate.fetcher",  "Récupère les règles depuis M03."),
            ("efe_gate.errors",   "DecisionError, ArtifactFetchFailed."),
        ],
        sub_sub=[
            ("efe_gate.engine", [
                ("RuleEngine.evaluate", "Applique les règles in-order."),
                ("DecisionTrace",       "Piste : règle → verdict partiel."),
            ]),
            ("efe_gate.rules", [
                ("ConfidenceThreshold", "reject si confidence &lt; 0.5."),
                ("ConeCompatibility",   "reject si cos &lt; 0.3 avec cône."),
                ("PiiRule",             "hold si M11 détecte PII."),
                ("TenantBlacklist",     "reject sur liste statique."),
            ]),
            ("efe_gate.fetcher", [
                ("RulesFetcher.get", "Télécharge rules.yaml depuis M03."),
                ("signed_by_m11",    "Vérifie signature M11."),
            ]),
        ],
        interfaces=[
            ("RuleEngine.evaluate", "(resp, ctx) -&gt; Decision", "Applique règles."),
            ("Decision",            "@dataclass(frozen=True)",    "Verdict + trace."),
            ("RulesFetcher.get",    "(tag) -&gt; Rules",          "Charge signé."),
        ],
        algorithms=[
            ("Short-circuit evaluation",
             "Les règles sont évaluées dans l'ordre priority ; la "
             "première règle qui retourne <code>reject</code> ou "
             "<code>hold</code> stoppe l'évaluation."),
            ("Cône compatibility check",
             "Produit scalaire <code>latent · cone_normal</code> ; "
             "projection sur le cône si &gt; max_projection_distance "
             "⇒ <code>reject</code>."),
        ],
        data_structures=[
            ("Decision",   "verdict ∈ {admit, hold, reject}, trace, reasons."),
            ("Rules",      "Liste ordonnée de RuleSpec."),
            ("RuleSpec",   "name, priority, params, version."),
        ],
        tests=[
            ("tests_gate/test_engine.py",  "20 cas — ordre, short-circuit."),
            ("tests_gate/test_rules.py",   "36 cas — 3×12 règles × cas."),
            ("tests_gate/test_fetcher.py", "8 cas  — signature, cache."),
        ],
        dependencies_up=["M03, M06, M11"],
        dependencies_down=["M07, M12"],
        acceptance=[
            "12/12 règles couvertes par 3 cas chacune (nominal, bord, attaque).",
            "p99 evaluate &lt; 2 ms.",
            "Signatures rules.yaml refusées si clé ≠ M11 RootKey.",
            "Decision trace reproductible.",
        ],
    )


def section_m09() -> None:
    module_section(
        mid="M09", title="Drift Detection", package="efe_drift",
        overview=[
            "M09 surveille la <b>dérive statistique</b> des évaluations. "
            "Il agrège les traces M06 et compare la distribution courante "
            "à une baseline figée (fenêtre 7 jours). Trois statistiques "
            "sont calculées : KL divergence, PSI, et score KS.",
            "Les résultats sont publiés en <code>DriftResult</code>, "
            "scellés HMAC et consommés par M10 pour déclencher un "
            "ré-entraînement.",
        ],
        scope_in=[
            "KL, PSI, KS en streaming.",
            "Baseline automatique rolling 7 j.",
            "Severity ∈ {info, warn, major, critical}.",
            "Scellé HMAC avec <code>efe-drift</code>.",
        ],
        scope_out=[
            "Détection multi-variée (copules, MMD) — post-MVP.",
            "Explicabilité par feature (post-MVP).",
        ],
        sub_modules=[
            ("efe_drift.detector",   "Moteur de détection."),
            ("efe_drift.statistics", "Implémentations KL, PSI, KS."),
            ("efe_drift.baseline",   "Gestion rolling baseline."),
            ("efe_drift.result",     "DriftResult + scellement."),
            ("efe_drift.errors",     "DriftError, BaselineUnavailable."),
        ],
        sub_sub=[
            ("efe_drift.detector", [
                ("DriftDetector.evaluate", "Calcule les 3 stats + severity."),
                ("Window",                 "Buffer circulaire de N échantillons."),
            ]),
            ("efe_drift.statistics", [
                ("kl_divergence", "KL(p || q) avec Laplace smoothing."),
                ("psi_score",     "Bins auto par quantiles."),
                ("ks_statistic",  "Two-sample Kolmogorov-Smirnov."),
            ]),
            ("efe_drift.baseline", [
                ("RollingBaseline", "Reconstruit sur 7 j tous les jours."),
                ("BaselineStore",   "Persistance en M03 (tag=baseline)."),
            ]),
            ("efe_drift.result", [
                ("DriftResult", "tenant_id, task_id, score, severity, hmac_tag."),
                ("seal_result", "HMAC efe-drift/canonical_json."),
            ]),
        ],
        interfaces=[
            ("DriftDetector.evaluate", "(samples) -&gt; DriftResult", "Calcule dérive."),
            ("DriftResult",            "@dataclass(frozen=True)",     "Verdict signé."),
            ("RollingBaseline.refresh","() -&gt; None",               "Rafraîchit."),
        ],
        algorithms=[
            ("KL Divergence smoothed",
             "<code>KL(p||q) = Σ p_i·log((p_i+ε)/(q_i+ε))</code> avec "
             "ε=1e-6 pour éviter les log(0) ; bornée à 50 pour stabilité."),
            ("Population Stability Index",
             "<code>PSI = Σ (a_i - e_i)·log(a_i/e_i)</code> sur 10 bins "
             "équifréquents ; seuils 0.1 (info), 0.25 (warn), 0.5 (major)."),
            ("Severity mapping",
             "max(KL_sev, PSI_sev, KS_sev) ⇒ severity global ; "
             "critical déclenche un re-train M10 immédiat."),
        ],
        data_structures=[
            ("DriftResult",   "evaluation_id, tenant_id, task_id, feature_name, "
                              "score, severity, timestamp, hmac_tag."),
            ("BaselineStats", "mean, std, quantiles[], bucket_edges[]."),
            ("DriftWindow",   "Buffer circulaire thread-safe."),
        ],
        tests=[
            ("tests_drift/test_detector.py",   "18 cas — nominal, no-drift, heavy-drift."),
            ("tests_drift/test_statistics.py", "22 cas — KL, PSI, KS."),
            ("tests_drift/test_baseline.py",   "11 cas — rolling, persistence."),
            ("tests_drift/test_result.py",     "8 cas  — scellement HMAC."),
        ],
        dependencies_up=["M03 (baseline), M06 (traces), M11 (HMAC)"],
        dependencies_down=["M10 — consomme DriftResult.critical."],
        acceptance=[
            "Détection fiable d'une dérive simulée de 10 % (PSI &gt; 0.25).",
            "Scellement HMAC vérifiable à 100 % des DriftResult.",
            "Baseline mise à jour tous les jours sans perte de données.",
            "CPU overhead &lt; 2 % par worker.",
        ],
    )


def section_m10() -> None:
    module_section(
        mid="M10", title="Scheduling & Continuous Learning",
        package="efe_scheduling",
        overview=[
            "M10 orchestre la <b>boucle d'apprentissage continu</b> : "
            "DriftResult critique ⇒ JobSpec d'entraînement M05 ⇒ "
            "Artifact M03 ⇒ RolloutPlan M07. Le scheduler est "
            "idempotent par <code>fire_key</code> (HMAC canonique).",
            "La planification utilise un <b>cron-like syntax</b> plus "
            "des triggers événementiels (drift, manual). La concurrence "
            "est limitée par tenant via un semaphore distribué Redis.",
        ],
        scope_in=[
            "ContinuousLearningPipeline (drift → retrain → rollout).",
            "JobSpec canonical-JSON-based, idempotent.",
            "fire_key HMAC pour dédoublonnage.",
            "Triggers : cron, drift, manual.",
        ],
        scope_out=[
            "Scheduling multi-cluster (post-MVP).",
            "Stratégies budgetaires avancées (cost-aware).",
        ],
        sub_modules=[
            ("efe_scheduling.pipeline",  "ContinuousLearningPipeline."),
            ("efe_scheduling.scheduler", "Cron + event triggers."),
            ("efe_scheduling.jobs",      "JobSpec + canonical hash."),
            ("efe_scheduling.dispatch",  "M10JobDispatcher — appelle M05."),
            ("efe_scheduling.errors",    "SchedulingError, DuplicateFire."),
        ],
        sub_sub=[
            ("efe_scheduling.pipeline", [
                ("ContinuousLearningPipeline.handle_drift",
                 "Point d'entrée sur DriftEvent critique."),
                ("on_retrain_done",
                 "Callback déclenchant M07."),
            ]),
            ("efe_scheduling.scheduler", [
                ("CronParser",     "Supporte les syntaxes standard UNIX."),
                ("EventRouter",    "Router drift/manual/cron → JobSpec."),
                ("TenantSemaphore","Bornes concurrency par tenant."),
            ]),
            ("efe_scheduling.jobs", [
                ("JobSpec",  "dataset, config, tenant_id, fire_key."),
                ("fire_key", "HMAC efe-sched/canonical_json(spec)."),
            ]),
            ("efe_scheduling.dispatch", [
                ("M10JobDispatcher.submit", "Appelle M05.Trainer.fit."),
                ("retry_with_backoff",      "Exponentiel 1s→30s→5min."),
            ]),
        ],
        interfaces=[
            ("ContinuousLearningPipeline.handle_drift",
             "(DriftResult) -&gt; Job", "Déclenche le pipeline."),
            ("JobSpec",            "@dataclass(frozen=True)", "Spec de job."),
            ("fire_key",           "(spec) -&gt; bytes",      "Hash HMAC."),
            ("M10JobDispatcher.submit",
             "(spec) -&gt; Future",  "Envoi à M05."),
        ],
        algorithms=[
            ("Dédoublonnage idempotent",
             "fire_key = HMAC-SHA256(k_sched, canonical_json(spec)). Si "
             "un job avec la même fire_key existe dans les 24 h, il est "
             "réutilisé — garantit qu'un pic de drift ne lance pas 50 "
             "re-trainings."),
            ("Backoff exponentiel plafonné",
             "delay = min(cap, base · 2^attempt) + jitter(0, 20 %) ; "
             "base=1 s, cap=5 min, abandon après 7 tentatives."),
        ],
        data_structures=[
            ("JobSpec",       "id, tenant_id, dataset_id, config, fire_key."),
            ("DriftEvent",    "drift_result + timestamp + retries."),
            ("ScheduleEntry", "cron_expr + job_template."),
        ],
        tests=[
            ("tests_scheduling/test_pipeline.py",  "20 cas — flow drift→rollout."),
            ("tests_scheduling/test_scheduler.py", "14 cas — cron + events."),
            ("tests_scheduling/test_jobs.py",      "12 cas — fire_key stabilité."),
            ("tests_scheduling/test_dispatch.py",  "10 cas — backoff, retries."),
        ],
        dependencies_up=["M03, M05, M07, M09, M11"],
        dependencies_down=["M12 — consommateur M10JobDispatcher."],
        acceptance=[
            "Idempotence prouvée : 100 drifts identiques ⇒ 1 seul job.",
            "p50 drift→job &lt; 500 ms.",
            "Crash recovery sans perte de jobs pending.",
            "Tous les jobs terminaux (OK/KO) audités dans M11.",
        ],
    )


def section_m11() -> None:
    module_section(
        mid="M11", title="Security, Multi-tenancy, Compliance",
        package="efe_security",
        overview=[
            "M11 est le <b>plan de sécurité transverse</b>. Il couvre : "
            "authentification (SPIFFE + JWT), autorisation "
            "(PolicyDecisionPoint + RBAC), PII linter, audit "
            "tamper-evident, isolation multi-tenant par clé chiffrée.",
            "Toutes les primitives cryptographiques sont domain-tagged "
            "(préfixe <code>efe-sec</code>) pour garantir "
            "l'indépendance des usages. M11 est l'<b>unique émetteur</b> "
            "de scellés HMAC pour les autres modules.",
        ],
        scope_in=[
            "PolicyDecisionPoint avec cache L1 (5 min TTL).",
            "PiiClassifier NFKC + regex + depth-limit.",
            "Audit chain HMAC-SHA256 append-only.",
            "Tenant provisioner + key-rotation 90 j.",
        ],
        scope_out=[
            "FIPS 140-3 certification (post-MVP — on suit les guidelines).",
            "HSM integration (post-MVP — MVP utilise KMS cloud).",
        ],
        sub_modules=[
            ("efe_security.authz",   "PDP + RBAC + ScopeMissing."),
            ("efe_security.pii",     "Classifier + Policy + Kinds."),
            ("efe_security.audit",   "Draft + Emitter + Store + Chain."),
            ("efe_security.context", "SecurityContext + Principal + Actor."),
            ("efe_security.tenancy", "Provisioner + KeyVault + Rotation."),
            ("efe_security.crypto",  "Primitives : HMAC, canonical_json, SecureBytes."),
            ("efe_security.errors",  "Famille d'erreurs M11."),
        ],
        sub_sub=[
            ("efe_security.authz", [
                ("PolicyDecisionPoint.decide", "Évalue (principal, action, resource)."),
                ("RbacRegistry",               "Role → scopes (admin, operator, reader, security_officer)."),
                ("ACTION_SCOPES",              "Table action → scopes requis."),
            ]),
            ("efe_security.pii", [
                ("PiiClassifier.find", "Regex + ancrage NFKC."),
                ("PiiPolicy",          "block / redact / allow selon kind."),
                ("PiiKind",            "email, phone, ssn, card, iban, …"),
            ]),
            ("efe_security.audit", [
                ("AuditDraft",        "Immutable ; canonicalisable."),
                ("AuditEmitter.sink", "Append dans un AuditStore."),
                ("AuditChain.verify", "Re-calcule chain_hash."),
            ]),
            ("efe_security.tenancy", [
                ("TenantProvisioner", "Crée tenant + clés + policy bundle."),
                ("KeyVault.rotate",   "Rotation 90 j sans downtime."),
            ]),
        ],
        interfaces=[
            ("PolicyDecisionPoint.decide", "async (req) -&gt; Decision",    "Autorise action."),
            ("PiiClassifier.find",         "(text) -&gt; list[Hit]",        "Détecte PII."),
            ("AuditEmitter.emit",          "async (draft) -&gt; None",      "Scelle + append."),
            ("hmac_seal",                  "(key, tag, payload) -&gt; bytes","Primitive."),
        ],
        algorithms=[
            ("Domain-tagged HMAC",
             "Tous les HMACs sont préfixés par un <i>tag</i> ASCII "
             "(<code>efe-sched:fire_key:</code>, <code>efe-drift:result:</code>, …). "
             "27 tags distincts, prouvés sans collision par un test "
             "générateur pair-wise."),
            ("Canonical JSON serialization",
             "Clés triées, séparateurs fixes, UTF-8 NFC, ints canonique, "
             "floats bannis. Reproductible à l'octet sur toutes les "
             "plateformes."),
            ("Audit chain",
             "chain_hash_n = HMAC(k_chain, chain_hash_(n-1) || "
             "canonical(draft_n)). Rupture détectée en O(1) par entrée."),
        ],
        data_structures=[
            ("SecurityContext", "Principal + request_id + timestamps."),
            ("Principal",       "Actor + tenant_id + scopes."),
            ("AuditDraft",      "occurred_at, tenant_id, actor_kind, actor_id, "
                                "action, outcome, severity, reasons, payload."),
            ("TenantKeys",      "k_master, k_chain, k_sched, k_drift, …"),
        ],
        tests=[
            ("tests_security/test_authz.py",   "30 cas — PDP, cache, RBAC."),
            ("tests_security/test_pii.py",     "42 cas — 12 PiiKind × 3.5 cas."),
            ("tests_security/test_audit.py",   "24 cas — chain integrity."),
            ("tests_security/test_tenancy.py", "15 cas — provisioning, rotation."),
            ("tests_security/test_crypto.py",  "20 cas — HMAC, canonical JSON."),
        ],
        dependencies_up=["M01 (types)"],
        dependencies_down=[
            "Tous les autres modules utilisent <code>efe_security</code> "
            "via <code>efe_swarm_integration.integration.adapters_m11</code>.",
        ],
        acceptance=[
            "Aucune collision de domain-tag (prouvé, 27 tags).",
            "PDP decision &lt; 3 ms en hit cache, &lt; 15 ms en miss.",
            "AuditChain.verify O(n) sur 1 M entries en &lt; 2 s.",
            "Tenant key rotation 0-downtime (validé par scénario).",
            "PII detection recall ≥ 0.97 sur jeu de test FR + EN.",
        ],
    )


def section_m12() -> None:
    module_section(
        mid="M12", title="Integration with EFE Swarm",
        package="efe_swarm_integration",
        overview=[
            "M12 est la <b>façade unique d'intégration</b> vers l'essaim. "
            "Il expose <code>SwarmClient</code>, <code>AdminClient</code>, "
            "et un bundle <code>PlatformWiring</code> via "
            "<code>wire_up_platform()</code>. Il absorbe les 11 modules "
            "amont via 9 adaptateurs dédiés.",
            "La surface publique est gRPC ; les méthodes "
            "<i>classify/execute/describe_spec/cone.register</i> sont "
            "documentées par contrat protobuf. Un interceptor d'audit "
            "enregistre chaque appel dans M11.",
        ],
        scope_in=[
            "SwarmClient.classify / execute / describe_spec / register_cone.",
            "9 adaptateurs (M01, M02, M03, M04, M05, M06, M07, M09, M11).",
            "Catalog-key routing + contract adapter.",
            "Bulkhead, CircuitBreaker, BackpressureInterceptor.",
            "PDP guard + PII prompt guard à la borne.",
        ],
        scope_out=[
            "Support WebSocket (post-MVP, gRPC suffit).",
            "Versionning multi-schéma protobuf (V1 uniquement au MVP).",
        ],
        sub_modules=[
            ("efe_swarm_integration.client",      "SwarmClient + AdminClient."),
            ("efe_swarm_integration.integration", "Absorption layer — 9 adapters."),
            ("efe_swarm_integration.types",       "SwarmSpec V2, Anchor, ConeRef."),
            ("efe_swarm_integration.reconciler",  "Decoder + ClassifyResponse."),
            ("efe_swarm_integration.artefact",    "Syncer + BlobSource."),
            ("efe_swarm_integration.flow",        "Backpressure + Bulkhead + Circuit."),
            ("efe_swarm_integration.audit",       "AuditInterceptor."),
            ("efe_swarm_integration.errors",      "Taxonomie 12 erreurs M12."),
        ],
        sub_sub=[
            ("efe_swarm_integration.integration (adaptateurs)", [
                ("adapters_m01.M01PlatformAdapter",     "Bridge legacy SwarmSpec v1."),
                ("adapters_m02.M02WorkerAdapter",       "Abstrait le heartbeat."),
                ("adapters_m03.M03ArtifactBlobSource",  "Fournit BlobSource."),
                ("adapters_m04.M04OrchAdapter",         "Backpressure unifié."),
                ("adapters_m05.M05FtTrigger",           "Lance un train M05 via M10."),
                ("adapters_m06.M06InferenceAdapter",    "classify_text pour M06."),
                ("adapters_m07.M07RolloutPreflight",    "describe_spec pour stage."),
                ("adapters_m09.M09DriftAdapter",        "Réagit aux drifts critical."),
                ("adapters_m11.M11PdpAdapter",          "PDP/Pii/Audit ports concrets."),
            ]),
            ("efe_swarm_integration.client", [
                ("SwarmClient.classify",      "async (req) -&gt; ClassifyResponse."),
                ("SwarmClient.execute",       "async (req) -&gt; ExecuteResponse."),
                ("SwarmClient.describe_spec", "async (spec) -&gt; SpecReport."),
                ("AdminClient.register_cone", "async (cone) -&gt; ConeRef."),
            ]),
            ("efe_swarm_integration.flow", [
                ("BulkheadRegistry",        "Bornée par TemplateCode."),
                ("CircuitBreakerRegistry",  "Instance par pool worker."),
                ("BackpressureInterceptor", "Enveloppe les gRPC calls."),
            ]),
            ("efe_swarm_integration.artefact", [
                ("ArtefactSyncer", "Replique un artefact vers worker."),
                ("BlobSource",     "Protocol iter_chunks async."),
            ]),
        ],
        interfaces=[
            ("SwarmClient.classify",      "async (ClassifyRequest) -&gt; ClassifyResponse", "Inférence."),
            ("SwarmClient.execute",       "async (ExecuteRequest) -&gt; ExecuteResponse",   "Exécution."),
            ("SwarmClient.describe_spec", "async (SwarmSpec) -&gt; SpecReport",             "Inspection."),
            ("wire_up_platform",          "(driver, embedder, audit, pdp, pii) -&gt; Wiring","Fabrique."),
        ],
        algorithms=[
            ("Absorption par adaptation",
             "M12 est le <b>seul</b> module dont le code importe des "
             "symboles depuis M01-M11 (excepté M11 qui est socle). "
             "Chaque adaptateur dépend de ≤ 2 modules amont, et expose "
             "une API Protocol-stable ; tous les autres modules M12 "
             "manipulent uniquement des Protocols."),
            ("Canonical catalog key",
             "Catalog-key = HMAC-SHA256(k_m12, canonical_json({feature, "
             "task}))  — stable sur évolution de schéma."),
        ],
        data_structures=[
            ("SwarmSpec (V2)",  "dim_latent, problem_embedding, population, "
                                "alignment_cone (ou ref)."),
            ("ClassifyRequest", "text, catalog_key, tenant_id, actor_id, "
                                "budget_ms, max_rounds."),
            ("ExecuteRequest",  "swarm_id, input, budget_ms."),
            ("PlatformWiring",  "swarm, admin, m10_dispatch, m09_drift, "
                                "catalog, m11 adapters."),
        ],
        tests=[
            ("tests_swarm_integration/test_client.py",            "36 cas — SwarmClient, AdminClient."),
            ("tests_swarm_integration/test_flow.py",              "22 cas — bulkhead, circuit."),
            ("tests_swarm_integration/test_audit.py",             "14 cas — interceptor."),
            ("tests_swarm_integration/test_absorption.py",        "19 cas — 9 adapters, vrais M1-M11."),
            ("tests_swarm_integration/test_absorption_closure.py","3 cas  — taxonomie erreurs."),
        ],
        dependencies_up=["M01 → M11 (via adaptateurs uniquement)."],
        dependencies_down=[
            "Clients externes (SDK Python, gRPC direct).",
            "CLI <code>efe</code> (post-MVP).",
        ],
        acceptance=[
            "100 % des M1-M11 absorbés par un adaptateur dédié.",
            "E2E classify (text → verdict) &lt; 500 ms p50 en tenant premium.",
            "Interceptor d'audit actif sur toutes les RPC (0 trou).",
            "wire_up_platform() idempotent — appel double ⇒ même objet.",
            "Taxonomie d'erreur couvre &gt; 95 % des exceptions M1-M11.",
        ],
    )


# =========================================================================
# 16. SPRINTS
# =========================================================================

def section_sprints() -> None:
    h(1, "16.  Plan de sprints (12 semaines)")
    p("Le MVP se livre en <b>6 sprints de 2 semaines</b> avec un "
      "hard-freeze en S11 (test d'acceptation + sécurité) et un go-live "
      "staged en S12. La table ci-dessous fixe les livrables.")
    table(
        header=["Sprint", "Durée", "Modules", "Livrables majeurs"],
        rows=[
            ["S1 — Socle",       "S1-S2",   "M01, M02 squelette",
             "Types gelés, worker runtime qui boote et drain proprement."],
            ["S2 — Données",     "S3-S4",   "M03, M01 final",
             "Registry CRUD, blob S3, digest streaming."],
            ["S3 — Contrôle",    "S5-S6",   "M04, M06",
             "Orchestration, réconcilateur Mean + MoE, Classify e2e CPU."],
            ["S4 — Qualité",     "S7-S8",   "M05, M08, M09",
             "Fine-tuning LoRA, 12 règles de gate, KL/PSI/KS."],
            ["S5 — Sécurité",    "S9-S10",  "M11 complet, M07",
             "PDP/PII/Audit chain, rollout 4-stages."],
            ["S6 — Intégration", "S11-S12", "M10, M12 final",
             "ContinuousLearningPipeline, SwarmClient gRPC, "
             "PlatformWiring, go-live canary 5 %."],
        ],
        col_widths=[30 * mm, 22 * mm, 35 * mm, 88 * mm],
    )
    h(2, "Jalons globaux")
    bullets([
        "<b>J1 (fin S2)</b> : premier bot répond en local avec worker + "
        "registry opérationnels.",
        "<b>J2 (fin S4)</b> : classify e2e avec 4 contributeurs, "
        "verdict convergent &lt; 1 s.",
        "<b>J3 (fin S6)</b> : pipeline drift→retrain prouvé sur "
        "jeu synthétique.",
        "<b>J4 (fin S8)</b> : audit chain vérifiable sur 100 k entrées.",
        "<b>J5 (fin S10)</b> : rollout canary 5 % en staging.",
        "<b>J6 (fin S12)</b> : go-live prod en tenant pilote, SLO tenus.",
    ])
    pagebreak()


# =========================================================================
# 17. ACCEPTANCE
# =========================================================================

def section_acceptance_global() -> None:
    h(1, "17.  Critères d'acceptation globaux du MVP")
    p("Le MVP est considéré <b>livré</b> quand l'ensemble des "
      "critères suivants sont satisfaits. Chaque critère est validé "
      "par un test automatique (CI) ou un test manuel signé.")
    h(2, "17.1  Fonctionnels")
    bullets([
        "E2E classify <code>text → ClassifyResponse</code> en &lt; 500 ms "
        "p50, &lt; 2 s p99.",
        "Execute d'un essaim (≤ 32 contributeurs) sous 10 s p95.",
        "Describe_spec retourne un rapport complet avec coût estimé.",
        "Cone register/evolve/delete respectent l'ACL M11.",
        "Pipeline drift → retrain → rollout déclenché automatiquement "
        "sous 24 h sur dérive critique.",
    ])
    h(2, "17.2  Non-fonctionnels")
    bullets([
        "SLO disponibilité : 99,5 % (MVP), 99,9 % (post-MVP).",
        "SLO latence : p50 &lt; 500 ms, p99 &lt; 2 s pour classify.",
        "Débit : ≥ 200 classify/s sur 3 nœuds L40S.",
        "Empreinte : ≤ 12 GiB RAM par worker, ≤ 24 GiB VRAM par "
        "contributeur.",
    ])
    h(2, "17.3  Sécurité &amp; conformité")
    bullets([
        "Aucune PII clear-text n'atteint M02/M06 (audité par fuzzing).",
        "Chaque requête reçoit un audit avec chain_hash vérifiable.",
        "Rotation de clé tenant sans downtime (test signé).",
        "Zéro collision HMAC domain-tag (27 tags × 5 préfixes).",
        "Tous les artefacts M03 signés par la RootKey M11.",
    ])
    h(2, "17.4  Qualité du code")
    bullets([
        "Couverture ≥ 90 % sur tous les modules.",
        "Aucun <code>TODO</code>, <code>FIXME</code>, <code>XXX</code> "
        "dans les modules M1-M12 (grep CI).",
        "Type-check mypy --strict vert.",
        "Lint ruff + black vert.",
        "Temps de build Docker &lt; 8 min (image 900 MiB).",
    ])
    pagebreak()


# =========================================================================
# 18. ANNEXES
# =========================================================================

def section_annexes() -> None:
    h(1, "18.  Annexes")
    h(2, "18.1  Glossaire")
    table(
        header=["Terme", "Définition"],
        rows=[
            ["Cône d'alignement", "Ensemble convexe de vecteurs latents "
                                  "autorisés pour un essaim."],
            ["Contributeur",      "Instance élémentaire (bot ou modèle) "
                                  "participant à l'essaim."],
            ["Canary",            "Tag de déploiement limité à 1-5 % du "
                                  "trafic pour validation."],
            ["Fire-key",          "Hash HMAC identifiant un job, "
                                  "garantissant idempotence."],
            ["HBM budget",        "Budget de mémoire haut-débit (HBM) "
                                  "par requête."],
            ["Réconciliateur",    "Agrégateur déterministe de latents "
                                  "produisant le verdict."],
            ["Scopes",            "Permissions granulaires attachées à "
                                  "un principal M11."],
            ["SwarmSpec",         "Dataclass immuable décrivant la "
                                  "forme d'un essaim."],
        ],
        col_widths=[40 * mm, 125 * mm],
    )
    h(2, "18.2  Matrice RACI")
    table(
        header=["Activité", "Core", "Infra", "Sec", "Prod"],
        rows=[
            ["Définition API publique",       "R", "C", "C", "I"],
            ["Implémentation M1-M10",         "R", "C", "I", "I"],
            ["Implémentation M11 (sécurité)", "A", "C", "R", "I"],
            ["Absorption M12",                "R", "A", "C", "I"],
            ["Rollout prod",                  "C", "R", "A", "A"],
            ["Audit de sécurité",             "C", "C", "R", "A"],
            ["SLO monitoring",                "I", "R", "C", "A"],
        ],
        col_widths=[70 * mm, 22 * mm, 22 * mm, 22 * mm, 22 * mm],
        caption="R = Responsible, A = Accountable, C = Consulted, I = Informed",
    )
    h(2, "18.3  Métriques clés")
    bullets([
        "<b>Latence</b> : classify p50/p95/p99, execute p50/p95/p99.",
        "<b>Débit</b> : req/s par module, par tenant.",
        "<b>Qualité</b> : F1-macro, ECE, AUROC sur hold-out.",
        "<b>Sécurité</b> : taux de deny PDP, PII hits/jour, audit gaps.",
        "<b>Résilience</b> : MTTR, heartbeat miss rate, rollbacks.",
        "<b>Dérive</b> : KL/PSI/KS par tenant × task, alertes/semaine.",
    ])
    h(2, "18.4  Référence croisée module × invariant")
    table(
        header=["Invariant", "Modules concernés"],
        rows=[
            ["Canonical JSON",             "M09, M10, M11, M12"],
            ["HMAC domain-tagged",         "M07, M09, M10, M11, M12"],
            ["Immutable dataclasses",      "M01, M06, M09, M10, M11, M12"],
            ["Audit append-only",          "M11 — émis par M04, M07, M08, M10, M12"],
            ["No external clear-text PII", "M11 (émetteur) × M06, M12 (bornes)"],
            ["Idempotence par fire-key",   "M10 (émis) × M05 (consommé)"],
        ],
        col_widths=[60 * mm, 105 * mm],
    )
    spacer(6)
    p("<i>Fin du document — EFE MVP Sous-Modules v1.0</i>", "Caption")


# =========================================================================
# PAGE DECORATION (footer page numbers)
# =========================================================================

def _on_page(canvas, doc):
    canvas.saveState()
    canvas.setFont("Helvetica", 8)
    canvas.setFillColor(colors.HexColor("#666666"))
    canvas.drawRightString(
        A4[0] - 15 * mm, 12 * mm,
        f"EFE MVP — Sous-Modules   |   p. {doc.page}",
    )
    canvas.drawString(
        15 * mm, 12 * mm,
        "Essaim Fluide Éphémère — Spécification technique",
    )
    canvas.restoreState()


# =========================================================================
# MAIN
# =========================================================================

def main() -> None:
    out_path = Path(
        "/Users/rodrigue.pekar/Downloads/jjjj/efe/EFE_MVP_Sous_Modules.pdf"
    )
    doc = SimpleDocTemplate(
        str(out_path),
        pagesize=A4,
        leftMargin=18 * mm,
        rightMargin=18 * mm,
        topMargin=18 * mm,
        bottomMargin=18 * mm,
        title="EFE MVP — Sous-Modules Ultra-Détaillés",
        author="Équipe EFE Core",
        subject="Spécification technique des sous-modules MVP",
    )

    # Build story
    section_cover()
    section_toc()
    section_intro()
    section_architecture()
    section_nomenclature()
    section_m01()
    section_m02()
    section_m03()
    section_m04()
    section_m05()
    section_m06()
    section_m07()
    section_m08()
    section_m09()
    section_m10()
    section_m11()
    section_m12()
    section_sprints()
    section_acceptance_global()
    section_annexes()

    doc.build(STORY, onFirstPage=_on_page, onLaterPages=_on_page)
    size_kb = out_path.stat().st_size / 1024
    print(f"OK  -> {out_path}  ({size_kb:.1f} KiB)")


if __name__ == "__main__":
    main()
