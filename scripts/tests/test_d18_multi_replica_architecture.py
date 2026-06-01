"""Sprint **D18** — Multi-réplica + bench D12 architecture pin tests.

Architecture-doc-only sprint pin tests. The implementation will be
added in subsequent commits ; these tests pin the document
existence + structure so a future refactor that drops sections
surfaces in CI.

Cf. `docs/bench/d18_multi_replica_bench_d12_architecture.md`.
"""

from __future__ import annotations

from pathlib import Path

_REPO = Path(__file__).resolve().parents[2]
_DOC = _REPO / "docs" / "bench" / "d18_multi_replica_bench_d12_architecture.md"


# ─────────────────────────────────────────────────────────────────────
# Document existence + sanity
# ─────────────────────────────────────────────────────────────────────


def test_d18_architecture_doc_exists():
    assert _DOC.exists(), (
        "D18 Multi-réplica + bench D12 architecture doc missing — "
        "sprint cannot proceed to code phase"
    )


def test_d18_architecture_doc_min_length():
    """Architecture docs in this project are dense (~1500-2200 lignes).
    A doc < 1200 lignes is a sign of insufficient depth."""
    n = sum(1 for _ in _DOC.read_text(encoding="utf-8").splitlines())
    assert n >= 1200, (
        f"D18 architecture doc is too short ({n} lignes ; expected ≥ 1200)"
    )


# ─────────────────────────────────────────────────────────────────────
# Required sections (template alignment with previous sprints)
# ─────────────────────────────────────────────────────────────────────


_REQUIRED_SECTIONS = [
    "## 0. TL;DR",
    "## 1. Pourquoi ce sprint maintenant",
    "## 2. Threat model",
    "## 3. Requirements",
    "## 4. Inventaire des state per-replica vs shared",
    "## 5. State management",
    "## 6. Idempotency cache",
    "## 7. Rate-limit buckets",
    "## 8. Leader election",
    "## 9. Audit chain integrity sous N>1",
    "## 10. Bundle memory amplification",
    "## 11. Graceful shutdown",
    "## 12. Load balancing",
    "## 13. Health probes",
    "## 14. Helm chart D18",
    "## 15. Bench harness",
    "## 16. Bench métriques",
    "## 17. Métriques Prometheus",
    "## 18. Alertes",
    "## 19. Test plan",
    "## 20. Cardinality envelope",
    "## 21. Migration plan",
    "## 22. Critères d'acceptation D18",
    "## 23. Anti-corruption",
    "## 24. Non-objectifs",
    "## 26. Conclusion",
]


def test_d18_architecture_has_all_required_sections():
    content = _DOC.read_text(encoding="utf-8")
    missing = [s for s in _REQUIRED_SECTIONS if s not in content]
    assert not missing, (
        f"D18 architecture doc missing sections: {missing}"
    )


# ─────────────────────────────────────────────────────────────────────
# Threat model coverage (12 threats)
# ─────────────────────────────────────────────────────────────────────


def test_d18_threat_model_covers_12_threats():
    content = _DOC.read_text(encoding="utf-8")
    for tid in [f"T-D18-{n}" for n in range(1, 13)]:
        assert tid in content, (
            f"Threat {tid} must be enumerated in § 2"
        )


def test_d18_threat_model_includes_canonical_classes():
    """The 12 threats must cover at minimum these canonical
    multi-replica classes."""
    content = _DOC.read_text(encoding="utf-8").lower()
    for marker in [
        "split-brain",
        "idempotency",
        "rate-limit",
        "audit chain",
        "graceful shutdown",
        "leader election",
        "cone gc",
    ]:
        assert marker in content, (
            f"D18 threat model missing canonical class: {marker!r}"
        )


# ─────────────────────────────────────────────────────────────────────
# Requirements
# ─────────────────────────────────────────────────────────────────────


def test_d18_requirements_enumerated():
    content = _DOC.read_text(encoding="utf-8")
    # Functional R-D18-1 to R-D18-10
    for rid in [f"R-D18-{n}" for n in range(1, 11)]:
        assert rid in content, f"Requirement {rid} missing"
    # Non-functional N-D18-1 to N-D18-6
    for nid in [f"N-D18-{n}" for n in range(1, 7)]:
        assert nid in content, f"Non-functional {nid} missing"


def test_d18_compliance_requirements_present():
    content = _DOC.read_text(encoding="utf-8")
    for cid in ["C-D18-1", "C-D18-2"]:
        assert cid in content, f"Compliance requirement {cid} missing"


# ─────────────────────────────────────────────────────────────────────
# SLOs (D18-bench-1 to D18-bench-7)
# ─────────────────────────────────────────────────────────────────────


def test_d18_slos_enumerated():
    content = _DOC.read_text(encoding="utf-8")
    for slo in [f"D18-bench-{n}" for n in range(1, 8)]:
        assert slo in content, f"SLO {slo} not declared"


def test_d18_slos_are_quantitative():
    """Each SLO must have a measurable target — at minimum the
    pattern `≤ N ms` or `≥ N RPS` somewhere in any of its
    occurrences. The same SLO id appears multiple times (TL;DR
    list + § 3.3 detailed definitions + acceptance criteria) ;
    we require at least ONE occurrence to be quantitative."""
    import re as _re
    lines = _DOC.read_text(encoding="utf-8").splitlines()
    for n in range(1, 8):
        slo = f"D18-bench-{n}"
        # Find ALL occurrences of this SLO id.
        idxs = [i for i, l in enumerate(lines) if slo in l]
        assert idxs, f"no line found for {slo}"
        # For each occurrence, check the 3-line block. At least
        # one block must be quantitative.
        any_quant = False
        for i in idxs:
            block = "\n".join(lines[i:i + 3])
            if _re.search(
                r"\d+\s*(ms|RPS|%|ops/s|/s|GiB|MiB)",
                block,
                _re.IGNORECASE,
            ):
                any_quant = True
                break
        assert any_quant, (
            f"SLO {slo} doesn't appear to have a quantitative target "
            f"in any of its {len(idxs)} occurrences"
        )


# ─────────────────────────────────────────────────────────────────────
# Metrics inventory
# ─────────────────────────────────────────────────────────────────────


def test_d18_metrics_inventory():
    content = _DOC.read_text(encoding="utf-8")
    expected_metrics = [
        "efe_replica_id",
        "efe_idempotency_hits_total",
        "efe_ratelimit_decisions_total",
        "efe_leader_is_leader",
        "efe_shutdown_phase_seconds",
        "efe_bench_runs_total",
    ]
    for m in expected_metrics:
        assert m in content, f"Metric {m} must appear in § 17"


# ─────────────────────────────────────────────────────────────────────
# Alerts inventory
# ─────────────────────────────────────────────────────────────────────


def test_d18_alerts_inventory():
    content = _DOC.read_text(encoding="utf-8")
    for aid in [f"A-D18-{n}" for n in range(1, 8)]:
        assert aid in content, f"Alert {aid} must be in § 18"


# ─────────────────────────────────────────────────────────────────────
# Cross-sprint integration
# ─────────────────────────────────────────────────────────────────────


def test_d18_integrates_with_existing_sprints():
    content = _DOC.read_text(encoding="utf-8")
    # D18 builds on top of all previous major sprints.
    for sprint in [
        "D11", "D12", "D14", "D14-bis", "D15", "D16", "D17",
    ]:
        assert sprint in content, (
            f"D18 architecture must reference {sprint} integration"
        )


# ─────────────────────────────────────────────────────────────────────
# Acceptance criteria
# ─────────────────────────────────────────────────────────────────────


def test_d18_acceptance_criteria_present():
    """§ 22 must list checkable criteria for sprint closure."""
    content = _DOC.read_text(encoding="utf-8")
    assert "Critères d'acceptation D18" in content
    assert "No regression" in content
    # Must mention the audit-report deliverable (institutional pattern).
    assert "d18_multi_replica_audit_report.md" in content
    # Must mention DR drill (D17) re-run on multi-replica.
    assert "DR drill" in content


# ─────────────────────────────────────────────────────────────────────
# Strategic decisions documented
# ─────────────────────────────────────────────────────────────────────


def test_d18_documents_pg_vs_redis_decision():
    """The choice of PG over Redis for idempotency + rate-limit
    must be explicit so a future maintainer understands why."""
    content = _DOC.read_text(encoding="utf-8")
    assert "Redis" in content
    assert "PG" in content or "Postgres" in content


def test_d18_documents_k8s_lease_for_leader_election():
    """K8s Lease object (coordination.k8s.io/v1) must be the
    chosen leader election mechanism, NOT pg_advisory_lock."""
    content = _DOC.read_text(encoding="utf-8")
    assert "Lease" in content
    assert "coordination.k8s.io" in content


def test_d18_documents_headless_service_for_grpc_lb():
    """gRPC over HTTP/2 multiplexing requires headless service
    + client-side LB for fair distribution."""
    content = _DOC.read_text(encoding="utf-8")
    assert "headless" in content.lower()
    assert "client-side" in content.lower() or "client side" in content.lower()


def test_d18_documents_graceful_shutdown_3_phase():
    """The shutdown sequence must be explicitly 3-phase :
    drain RPC → audit flush → pg close."""
    content = _DOC.read_text(encoding="utf-8")
    for phase in ["Phase 1", "Phase 2", "Phase 3"]:
        assert phase in content


# ─────────────────────────────────────────────────────────────────────
# Non-objectives explicit
# ─────────────────────────────────────────────────────────────────────


def test_d18_non_objectives_explicit():
    """§ 24 must list at least 6 explicit non-objectives so future
    maintainers don't misunderstand scope."""
    content = _DOC.read_text(encoding="utf-8")
    section_24 = (
        content.split("## 24.")[1].split("## 25.")[0]
        if "## 24." in content and "## 25." in content
        else ""
    )
    import re
    items = re.findall(r"^\d+\.\s", section_24, re.MULTILINE)
    assert len(items) >= 6, (
        f"§ 24 non-objectifs must list ≥ 6 items, found {len(items)}"
    )


# ─────────────────────────────────────────────────────────────────────
# Bench-specific pins
# ─────────────────────────────────────────────────────────────────────


def test_d18_bench_documents_6_workload_patterns():
    """The bench harness must support 6 workload patterns."""
    content = _DOC.read_text(encoding="utf-8")
    for w in [
        "read-heavy",
        "write-heavy",
        "balanced",
        "burst",
        "multi-tenant",
        "soak",
    ]:
        assert w in content, f"bench workload {w!r} not documented"


def test_d18_bench_refuses_prod_target():
    """T-D18-12 mitigation : bench must refuse to run against prod."""
    content = _DOC.read_text(encoding="utf-8")
    assert "RefusedProdTarget" in content or "refuse" in content.lower()
