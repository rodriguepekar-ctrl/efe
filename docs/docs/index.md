# EFE — Ephemeral Fluid Swarm

> Multi-agent inference via shared latent space.
> A zero-shot multilingual text classifier you can ship in 10 minutes.

---

## Get started

<div class="grid cards" markdown>

-   :material-rocket-launch: **[Quickstart](tutorial/quickstart.md)** — 10 minutes from zero to first classification.
-   :material-book-open: **[Concepts](tutorial/concepts.md)** — anchors, swarm fusion, hybrid decoder explained.
-   :material-school: **[Your first classifier](tutorial/your-first-classifier.md)** — author your own intent catalog.
-   :material-api: **[API Reference](reference/api.md)** — full SDK API auto-generated from docstrings.

</div>

---

## Pourquoi EFE ?

Les systèmes multi-agents actuels (CrewAI, LangGraph, AutoGen) orchestrent
des agents LLM qui se parlent en texte. Cela impose une **séquentialisation
forcée** du raisonnement et une latence quasi-linéaire avec le nombre
d'agents impliqués.

EFE propose un paradigme différent : des **nano-agents éphémères** (durée
de vie 1–50 ms) qui communiquent directement via un **espace latent
partagé** (tenseur dense résidant en HBM) et convergent vers un consensus
géométrique.

Gain attendu : **10–100× en latence** pour des tâches de raisonnement
structuré, à qualité égale ou supérieure à un LLM distillé de taille
équivalente.

## Navigation

- [Quickstart](quickstart.md) : faire tourner un essaim en 5 minutes.
- [Architecture](concepts/architecture.md) : comprendre le design.
- [Glossaire](concepts/glossary.md) : vocabulaire technique.
- [ADRs](adr/index.md) : décisions architecturales documentées.
- [Contributing](contributing.md) : participer au projet.

## Statut

**Version 0.0.1 — Sprint 0 (bootstrap complet)**

Ce repo est en phase d'amorçage. Les composants fonctionnels arrivent
progressivement :

| Sprint | Contenu |
|--------|---------|
| S0 (maintenant) | Structure du repo, CI, Docker, ADR-001 |
| S1 | Types, erreurs, memory pool |
| S2 | Genesis + ELS + premier kernel CUDA |
| S3 | Fusion tensorielle (Swarm Attention Triton) |
| S4 | Consensus + templates initiaux |
| S5 | Dissolution + décodage |
| S6 | Alignment + API gRPC |
| S7 | Telemetry + release 0.1.0 MVP |
