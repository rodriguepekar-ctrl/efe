"""Tests for consent, DSAR, tenancy, quota, lifecycle, incident, compliance."""

from __future__ import annotations

import asyncio
from datetime import datetime, timedelta, timezone

import pytest

from efe_security.consent import (
    ConsentManager,
    DsarKind,
    DsarService,
    DsarState,
    LegalBasis,
    default_purpose_catalog,
)
from efe_security.errors import (
    ConsentMissing,
    ConsentWithdrawn,
    LegalHoldInForce,
    PurposeNotDeclared,
    QuotaExceeded,
    TenantDisabled,
    TenantNotFound,
)
from efe_security.incident import (
    IncidentService,
    NotificationCenter,
    Runbook,
    RunbookRegistry,
    SimpleCounterDetector,
    run_playbook,
)
from efe_security.incident.detector import IncidentDraft
from efe_security.incident.triage import IncidentCategory, IncidentState
from efe_security.lifecycle import (
    Category,
    ErasureService,
    GcDaemon,
    RetentionPolicy,
    anonymize_text,
    default_retention_policies,
)
from efe_security.secrets import (
    EnvelopeManager,
    InMemoryKmsClient,
    KeyRegistry,
    RotationPlan,
    RotationScheduler,
)
from efe_security.secrets.key_registry import KeyEntry, KeyPurpose, KeyState
from efe_security.tenancy import (
    OffboardMode,
    QuotaKind,
    QuotaManager,
    TenantProvisioner,
    TenantTier,
)


# ---------------------------------------------------------------------------
# Consent + purposes
# ---------------------------------------------------------------------------


def test_consent_record_and_require() -> None:
    async def scenario() -> None:
        cat = await default_purpose_catalog()
        mgr = ConsentManager(purposes=cat)
        now = datetime(2026, 4, 21, tzinfo=timezone.utc)
        rec = await mgr.record(
            tenant_id="t1",
            subject_id="s1",
            purposes=frozenset({"model_training"}),
            legal_basis=LegalBasis.consent,
            consent_text="I agree.",
            now=now,
        )
        assertion = await mgr.require(
            tenant_id="t1", subject_id="s1", purpose="model_training", now=now
        )
        assert assertion.consent_id == rec.consent_id

    asyncio.run(scenario())


def test_consent_withdraw_blocks_require() -> None:
    async def scenario() -> None:
        cat = await default_purpose_catalog()
        mgr = ConsentManager(purposes=cat)
        now = datetime(2026, 4, 21, tzinfo=timezone.utc)
        rec = await mgr.record(
            tenant_id="t1",
            subject_id="s1",
            purposes=frozenset({"model_training"}),
            legal_basis=LegalBasis.consent,
            consent_text="I agree.",
            now=now,
        )
        await mgr.withdraw(consent_id=rec.consent_id, now=now + timedelta(minutes=1))
        with pytest.raises(ConsentWithdrawn):
            await mgr.require(
                tenant_id="t1", subject_id="s1", purpose="model_training",
                now=now + timedelta(minutes=2),
            )

    asyncio.run(scenario())


def test_consent_unknown_purpose_raises() -> None:
    async def scenario() -> None:
        cat = await default_purpose_catalog()
        mgr = ConsentManager(purposes=cat)
        with pytest.raises(PurposeNotDeclared):
            await mgr.record(
                tenant_id="t1",
                subject_id="s1",
                purposes=frozenset({"unknown_purpose"}),
                legal_basis=LegalBasis.consent,
                consent_text="x",
            )

    asyncio.run(scenario())


def test_consent_missing_returns_error() -> None:
    async def scenario() -> None:
        cat = await default_purpose_catalog()
        mgr = ConsentManager(purposes=cat)
        with pytest.raises(ConsentMissing):
            await mgr.require(
                tenant_id="t1", subject_id="s1", purpose="model_training"
            )

    asyncio.run(scenario())


# ---------------------------------------------------------------------------
# DSAR
# ---------------------------------------------------------------------------


def test_dsar_pipeline() -> None:
    async def scenario() -> None:
        dsar = DsarService()
        r = await dsar.submit(
            tenant_id="t1", subject_id="s1", kind=DsarKind.erase
        )
        assert r.state == DsarState.received
        r = await dsar.start(r.request_id)
        assert r.state == DsarState.in_progress
        r = await dsar.complete(r.request_id, notes="done")
        assert r.state == DsarState.completed
        assert r.receipt_seal is not None

    asyncio.run(scenario())


def test_dsar_legal_hold_blocks() -> None:
    async def scenario() -> None:
        dsar = DsarService()
        await dsar.place_legal_hold(tenant_id="t1", subject_id="s1")
        r = await dsar.submit(
            tenant_id="t1", subject_id="s1", kind=DsarKind.erase
        )
        with pytest.raises(LegalHoldInForce):
            await dsar.start(r.request_id)

    asyncio.run(scenario())


# ---------------------------------------------------------------------------
# Tenancy : provisioning + quotas
# ---------------------------------------------------------------------------


def test_tenant_onboard_offboard_flow() -> None:
    async def scenario() -> None:
        kms = InMemoryKmsClient()
        registry = KeyRegistry()
        prov = TenantProvisioner(kms=kms, key_registry=registry)
        t = await prov.onboard(
            tenant_id="tid_test", display_name="TestCo", tier=TenantTier.standard
        )
        assert t.state == "active"
        got = await prov.require_active("tid_test")
        assert got.tenant_id == "tid_test"
        t2 = await prov.offboard(
            tenant_id="tid_test", mode=OffboardMode.hard,
            expected_version=t.state_version,
        )
        assert t2.state == "offboarding_hard"
        t3 = await prov.finalize_hard(
            tenant_id="tid_test", expected_version=t2.state_version
        )
        assert t3.state == "archived"

    asyncio.run(scenario())


def test_quota_exceeded() -> None:
    async def scenario() -> None:
        qm = QuotaManager(
            defaults={
                QuotaKind.api_rps: __import__(
                    "efe_security.tenancy.quota", fromlist=["QuotaSpec"]
                ).QuotaSpec(kind=QuotaKind.api_rps, capacity=2.0, refill_per_s=0.0),
            }
        )
        await qm.consume(tenant_id="t1", kind=QuotaKind.api_rps)
        await qm.consume(tenant_id="t1", kind=QuotaKind.api_rps)
        with pytest.raises(QuotaExceeded):
            await qm.consume(tenant_id="t1", kind=QuotaKind.api_rps)

    asyncio.run(scenario())


# ---------------------------------------------------------------------------
# Lifecycle : erasure + anonymizer + gc_daemon
# ---------------------------------------------------------------------------


def test_erasure_marks_and_deletes() -> None:
    async def scenario() -> None:
        es = ErasureService()
        now = datetime(2026, 4, 21, tzinfo=timezone.utc)
        r = await es.request(
            erasure_id="e1", tenant_id="t1", subject_id="s1", now=now
        )
        r = await es.soft_mark("e1", now=now)
        assert await es.is_marked(tenant_id="t1", subject_id="s1")
        r = await es.hard_delete("e1", now=now + timedelta(days=30))
        assert r.phase.value == "hard_deleted"

    asyncio.run(scenario())


def test_anonymize_text_redacts() -> None:
    out = anonymize_text("call +33 6 12 34 56 78 or email bob@test.org")
    assert "bob@test.org" not in out


def test_gc_daemon_invokes_callbacks() -> None:
    async def scenario() -> None:
        calls: list[tuple[Category, str]] = []

        def scanner(cat: Category, t: datetime) -> int:
            return 1 if cat == Category.inference_events_raw else 0

        def purger(cat: Category, t: datetime) -> int:
            calls.append((cat, "purged"))
            return 1

        policies = {
            Category.inference_events_raw: RetentionPolicy(
                Category.inference_events_raw, 30
            ),
        }
        daemon = GcDaemon(policies=policies, scanner=scanner, purger=purger)
        now = datetime(2026, 4, 21, tzinfo=timezone.utc)
        outcome = await daemon.run_once(now=now)
        assert outcome.purged[Category.inference_events_raw] == 1

    asyncio.run(scenario())


# ---------------------------------------------------------------------------
# Incident + runbook + notification
# ---------------------------------------------------------------------------


def test_counter_detector_fires_on_threshold() -> None:
    async def scenario() -> None:
        det = SimpleCounterDetector(
            window_s=60,
            threshold=3,
            category="authn",
            severity="high",
        )
        now = datetime(2026, 4, 21, tzinfo=timezone.utc)
        for _ in range(2):
            out = await det.observe(
                kind="authn.failure", labels={"tenant_id": "t1"}, value=1.0, now=now
            )
            assert out.incidents == []
        out = await det.observe(
            kind="authn.failure", labels={"tenant_id": "t1"}, value=1.0, now=now
        )
        assert len(out.incidents) == 1
        assert out.incidents[0].category == "authn"

    asyncio.run(scenario())


def test_incident_service_triages_and_notifies() -> None:
    async def scenario() -> None:
        svc = IncidentService()
        notif = NotificationCenter()
        incident = await svc.record(
            IncidentDraft(
                category="pii",
                severity="high",
                summary="test",
                tenant_id="t1",
            )
        )
        incident = await svc.transition(
            incident.incident_id, to_state=IncidentState.triaged
        )
        incident = await svc.transition(
            incident.incident_id, to_state=IncidentState.contained
        )
        draft = notif.build_draft(
            incident,
            affected_categories=("pii",),
            estimated_subjects=42,
        )
        assert "72h" in draft.draft_text or "GDPR" in draft.draft_text

    asyncio.run(scenario())


def test_runbook_registry_runs_playbook() -> None:
    async def scenario() -> None:
        registry = RunbookRegistry()

        async def playbook(incident):
            return {"contained": True, "incident_id": incident.incident_id}

        registry.register(
            Runbook(category=IncidentCategory.authn, playbook=playbook)
        )
        svc = IncidentService()
        incident = await svc.record(
            IncidentDraft(
                category="authn",
                severity="low",
                summary="test",
                tenant_id="t1",
            )
        )
        out = await run_playbook(registry, incident)
        assert out["contained"]

    asyncio.run(scenario())


# ---------------------------------------------------------------------------
# Secrets : rotation + envelope
# ---------------------------------------------------------------------------


def test_rotation_rotates_expired_keys() -> None:
    async def scenario() -> None:
        now_fixed = datetime(2026, 4, 21, tzinfo=timezone.utc)
        reg = KeyRegistry()
        kms = InMemoryKmsClient()
        kms.register_root_key("audit-chain")
        await reg.register(
            KeyEntry(
                key_id="audit-chain",
                alg_id="HMAC-SHA256-v1",
                purpose=KeyPurpose.audit_chain,
                state=KeyState.active,
                created_at=now_fixed - timedelta(days=200),
            )
        )
        scheduler = RotationScheduler(
            registry=reg,
            kms=kms,
            plans=(RotationPlan(purpose=KeyPurpose.audit_chain, max_age_days=90),),
        )
        outcome = await scheduler.run(now=now_fixed)
        assert outcome.rotated  # at least one rotation

    asyncio.run(scenario())


# ---------------------------------------------------------------------------
# Compliance
# ---------------------------------------------------------------------------


def test_compliance_evidence_nonempty() -> None:
    from efe_security.compliance import EvidenceCollector

    items = EvidenceCollector().collect()
    frameworks = {i.framework for i in items}
    assert {"gdpr", "ai_act", "soc2", "hipaa"}.issubset(frameworks)
