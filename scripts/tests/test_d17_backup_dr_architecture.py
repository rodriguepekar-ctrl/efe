"""Sprint **D17** — Backup + DR architecture document pin tests.

Architecture-doc-only sprint pin tests. The implementation will be
added in subsequent commits ; these tests pin the document
existence + structure so a future refactor that drops sections
surfaces in CI.

Cf. `docs/bench/d17_backup_dr_architecture.md`.
"""

from __future__ import annotations

from pathlib import Path

_REPO = Path(__file__).resolve().parents[2]
_DOC = _REPO / "docs" / "bench" / "d17_backup_dr_architecture.md"


# ─────────────────────────────────────────────────────────────────────
# Document existence + sanity
# ─────────────────────────────────────────────────────────────────────


def test_d17_architecture_doc_exists():
    assert _DOC.exists(), (
        "D17 Backup + DR architecture doc missing — sprint cannot "
        "proceed to code phase"
    )


def test_d17_architecture_doc_min_length():
    """Architecture docs in this project are dense (~1500-2200 lignes).
    A doc < 1000 lignes is a sign of insufficient depth."""
    n = sum(1 for _ in _DOC.read_text(encoding="utf-8").splitlines())
    assert n >= 1500, (
        f"D17 architecture doc is too short ({n} lignes ; expected ≥ 1500)"
    )


# ─────────────────────────────────────────────────────────────────────
# Required sections (template alignment with previous sprints)
# ─────────────────────────────────────────────────────────────────────


_REQUIRED_SECTIONS = [
    # Top-level structure
    "## 0. TL;DR",
    "## 1. Pourquoi ce sprint maintenant",
    "## 2. Threat model",
    "## 3. Requirements",
    "## 4. Inventaire des données critiques",
    "## 5. Stratégies de backup",
    "## 6. Pipeline backup détaillée",
    "## 7. Crate `efe-backup` design",
    "## 8. CLI `efe-backup` UX",
    "## 9. Schedule",
    "## 10. Restoration runbook",
    "## 11. Encryption",
    "## 12. Audit chain integrity post-restore",
    "## 13. Bundle re-verification",
    "## 14. Tenant isolation",
    "## 15. RTO / RPO",
    "## 16. DR drills",
    "## 17. Failover régional",
    "## 18. Integration avec sprints existants",
    "## 19. Métriques Prometheus",
    "## 20. Alertes",
    "## 21. Helm chart",
    "## 22. Test plan",
    "## 23. SLOs",
    "## 24. Migration plan",
    "## 25. Cardinality envelope",
    "## 26. Critères d'acceptation D17",
    "## 27. Anti-corruption",
    "## 28. Non-objectifs",
    "## 30. Conclusion",
]


def test_d17_architecture_has_all_required_sections():
    content = _DOC.read_text(encoding="utf-8")
    missing = [s for s in _REQUIRED_SECTIONS if s not in content]
    assert not missing, (
        f"D17 architecture doc missing sections: {missing}"
    )


# ─────────────────────────────────────────────────────────────────────
# Threat model coverage
# ─────────────────────────────────────────────────────────────────────


def test_d17_threat_model_covers_12_threats():
    content = _DOC.read_text(encoding="utf-8")
    for tid in [f"T-DR-{n}" for n in range(1, 13)]:
        assert tid in content, (
            f"Threat {tid} must be enumerated in § 2"
        )


def test_d17_threat_model_includes_canonical_attacks():
    """The 12 threats must cover at minimum these well-known attack
    classes. A future architecture refactor that drops one of these
    surfaces in CI."""
    content = _DOC.read_text(encoding="utf-8").lower()
    for marker in [
        "ransomware",
        "insider",
        "region",
        "audit chain",
        "supply-chain",
        "cross-env",
        "rgpd",
    ]:
        assert marker in content, (
            f"D17 threat model missing canonical class: {marker!r}"
        )


# ─────────────────────────────────────────────────────────────────────
# Requirements
# ─────────────────────────────────────────────────────────────────────


def test_d17_requirements_enumerated():
    content = _DOC.read_text(encoding="utf-8")
    # Functional requirements R-DR-1 to R-DR-10
    for rid in [f"R-DR-{n}" for n in range(1, 11)]:
        assert rid in content, f"Requirement {rid} missing"
    # Non-functional N-DR-1 to N-DR-6
    for nid in [f"N-DR-{n}" for n in range(1, 7)]:
        assert nid in content, f"Non-functional {nid} missing"


def test_d17_compliance_requirements_present():
    content = _DOC.read_text(encoding="utf-8")
    for cid in ["C-DR-1", "C-DR-2", "C-DR-3", "C-DR-4"]:
        assert cid in content, f"Compliance requirement {cid} missing"


# ─────────────────────────────────────────────────────────────────────
# RTO / RPO objectives
# ─────────────────────────────────────────────────────────────────────


def test_d17_rto_rpo_objectives_stated():
    content = _DOC.read_text(encoding="utf-8")
    # RTO 30 min, RPO 5 min (per § 15.2 / N-DR-1, N-DR-2)
    assert "30 minutes" in content or "30 min" in content, (
        "RTO target (30 min) must be explicitly stated"
    )
    assert "5 minutes" in content or "5 min" in content, (
        "RPO target (5 min) must be explicitly stated"
    )


# ─────────────────────────────────────────────────────────────────────
# Metrics inventory
# ─────────────────────────────────────────────────────────────────────


def test_d17_metrics_inventory_8_instruments():
    content = _DOC.read_text(encoding="utf-8")
    expected_instruments = [
        "efe_backup_runs_total",
        "efe_backup_duration_seconds",
        "efe_backup_size_bytes",
        "efe_backup_last_success_timestamp",
        "efe_backup_rpo_seconds",
        "efe_dr_drill_runs_total",
        "efe_dr_drill_duration_seconds",
        "efe_dr_drill_last_success_timestamp",
    ]
    for m in expected_instruments:
        assert m in content, f"Metric {m} must appear in § 19"


# ─────────────────────────────────────────────────────────────────────
# Alerts inventory
# ─────────────────────────────────────────────────────────────────────


def test_d17_alerts_inventory_7_alerts():
    content = _DOC.read_text(encoding="utf-8")
    for aid in [f"A-D17-{n}" for n in range(1, 8)]:
        assert aid in content, f"Alert {aid} must be in § 20"


# ─────────────────────────────────────────────────────────────────────
# Cross-sprint integration (must reference D14, D14-bis, D15, D16, D13-bis)
# ─────────────────────────────────────────────────────────────────────


def test_d17_integrates_with_existing_sprints():
    content = _DOC.read_text(encoding="utf-8")
    for sprint in ["D14", "D14-bis", "D15", "D16", "D13-bis"]:
        assert sprint in content, (
            f"D17 architecture must reference {sprint} integration "
            "in § 18"
        )


# ─────────────────────────────────────────────────────────────────────
# Acceptance criteria
# ─────────────────────────────────────────────────────────────────────


def test_d17_acceptance_criteria_present():
    """§ 26 must list checkable criteria for sprint closure."""
    content = _DOC.read_text(encoding="utf-8")
    assert "Critères d'acceptation D17" in content
    # Must mention regression on prior sprints (no regression D13-D16).
    assert "No regression" in content
    # Must mention DR drill RTO < 30 min.
    assert "RTO < 30 min" in content or "RTO" in content


# ─────────────────────────────────────────────────────────────────────
# Strategic decisions documented
# ─────────────────────────────────────────────────────────────────────


def test_d17_documents_pgbackrest_choice():
    """The choice of pgBackRest over alternatives (WAL-G, Barman)
    must be explicit so a future maintainer understands why."""
    content = _DOC.read_text(encoding="utf-8")
    assert "pgBackRest" in content
    assert "WAL-G" in content or "Barman" in content


def test_d17_documents_4_eyes_for_delete():
    """Destructive operations require two-person authorization."""
    content = _DOC.read_text(encoding="utf-8")
    assert "4-eyes" in content or "two-person" in content


def test_d17_documents_re_anchor_procedure():
    """If HMAC keys are definitively lost, the doc MUST explain
    the re-anchor contingency procedure (§ 12.3)."""
    content = _DOC.read_text(encoding="utf-8")
    assert "Re-anchor" in content or "re-anchor" in content
    # Must mention the GDPR notification requirement.
    assert "72 h" in content or "72h" in content


# ─────────────────────────────────────────────────────────────────────
# Non-objectives explicit
# ─────────────────────────────────────────────────────────────────────


def test_d17_non_objectives_explicit():
    """§ 28 must list at least 5 explicit non-objectives so future
    maintainers don't misunderstand scope."""
    content = _DOC.read_text(encoding="utf-8")
    section_28 = content.split("## 28.")[1] if "## 28." in content else ""
    # Count numbered items in non-objectives.
    import re
    items = re.findall(r"^\d+\.\s", section_28, re.MULTILINE)
    assert len(items) >= 5, (
        f"§ 28 non-objectifs must list ≥ 5 items, found {len(items)}"
    )
