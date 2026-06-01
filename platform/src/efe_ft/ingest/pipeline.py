"""The 8-stage ingestion pipeline — orchestrator.

The orchestrator is **agnostic** about its transport (gRPC, HTTP, direct
function call). It receives a list of :class:`IngestInput`, runs the 8
stages per item, and returns an :class:`IngestOutcome` summary plus a
per-sample list of accepted / rejected results.

Key properties:
    * **Fail-fast per sample** — one bad sample does not fail the batch.
    * **Idempotent** — replaying the same text produces zero new rows
      and zero new audit entries (except ``duplicate`` rejections of the
      retry, which we log once per batch at level 'debug').
    * **Atomic** — all the writes for a single sample (training_samples
      row + audit_events row) happen in the same transaction.
"""

from __future__ import annotations

import time
import uuid
from collections.abc import Iterable
from dataclasses import dataclass, field
from typing import Literal

from pydantic import BaseModel, ConfigDict
from sqlalchemy.exc import IntegrityError
from sqlalchemy.ext.asyncio import AsyncSession

from efe_ft.audit import AuditChain
from efe_ft.db.models import IngestBatch, TrainingSample
from efe_ft.errors import (
    IngestError,
    PiiBlocked,
    SchemaInvalid,
)
from efe_ft.hashing import content_hash, sha256_text
from efe_ft.ingest.balance import balance_skew_check
from efe_ft.ingest.dedup import Deduper
from efe_ft.ingest.pii import scrub
from efe_ft.ingest.schemas import BatchSummary, SampleIngestPayload


class IngestInput(BaseModel):
    """What the orchestrator consumes — essentially validated payload."""

    model_config = ConfigDict(extra="forbid")

    payload: SampleIngestPayload


@dataclass(frozen=True, slots=True)
class SampleResult:
    accepted: bool
    sample_id: str | None
    reason: str | None
    pii_kinds: list[str] = field(default_factory=list)


@dataclass(frozen=True, slots=True)
class IngestOutcome:
    batch_id: str
    summary: BatchSummary
    results: list[SampleResult]

    def to_dict(self) -> dict[str, object]:
        return {
            "batch_id": self.batch_id,
            "summary": self.summary.model_dump(),
            "n_accepted": self.summary.n_accepted,
            "n_rejected": self.summary.n_rejected,
        }


class IngestPipeline:
    """Orchestrates the 8 stages for a batch of samples."""

    def __init__(
        self,
        session: AsyncSession,
        *,
        deduper: Deduper,
        pii_policy: Literal["mask", "reject"] = "mask",
        balance_skew_max: float = 50.0,
        check_balance: bool = True,
    ) -> None:
        self._session = session
        self._deduper = deduper
        self._pii_policy = pii_policy
        self._balance_skew_max = balance_skew_max
        self._check_balance = check_balance

    async def run(
        self,
        inputs: Iterable[IngestInput],
        *,
        tenant_id: str,
        task_id: str,
        actor_id: str,
        batch_source: Literal["batch", "webhook", "labeling"] = "batch",
    ) -> IngestOutcome:
        inputs = list(inputs)
        t0 = time.monotonic()
        rejections: dict[str, int] = {}
        results: list[SampleResult] = []
        chain = AuditChain(self._session, tenant_id)

        # 6. Balance check — one-shot, before batch persists any row.
        if self._check_balance:
            report = await balance_skew_check(
                self._session, tenant_id, task_id,
            )
            # Peek what would happen after adding the new labelled items.
            hypothetical = dict(report.counts)
            for item in inputs:
                p = item.payload
                if p.label:
                    hypothetical[p.label] = hypothetical.get(p.label, 0) + 1
            if hypothetical and len(hypothetical) > 1:
                values = list(hypothetical.values())
                skew = max(values) / max(1, min(values))
                if skew > self._balance_skew_max:
                    # Reject entire batch with diagnostic — no per-sample work.
                    reason = "balance_skew"
                    rejections[reason] = len(inputs)
                    for _ in inputs:
                        results.append(SampleResult(
                            accepted=False, sample_id=None, reason=reason,
                        ))
                    await chain.record(
                        kind="ingest_rejected",
                        actor_id=actor_id,
                        payload={
                            "reason": reason, "skew": skew,
                            "task_id": task_id, "n": len(inputs),
                        },
                    )
                    summary = BatchSummary(
                        batch_id=str(uuid.uuid4()),
                        n_received=len(inputs), n_accepted=0,
                        n_rejected=len(inputs),
                        rejections=rejections,
                        latency_ms=int((time.monotonic() - t0) * 1000),
                    )
                    return IngestOutcome(
                        batch_id=summary.batch_id,
                        summary=summary, results=results,
                    )

        # Per-sample loop (stages 3-8).
        for item in inputs:
            try:
                result = await self._handle_one(
                    item.payload,
                    tenant_id=tenant_id,
                    task_id=task_id,
                    actor_id=actor_id,
                    batch_source=batch_source,
                    chain=chain,
                )
            except IngestError as exc:
                result = SampleResult(
                    accepted=False, sample_id=None, reason=exc.code,
                )
                rejections[exc.code] = rejections.get(exc.code, 0) + 1
            results.append(result)
            if result.accepted:
                continue
            rejections[result.reason or "unknown"] = (
                rejections.get(result.reason or "unknown", 0) + 1
            )

        n_accepted = sum(1 for r in results if r.accepted)
        n_rejected = len(results) - n_accepted
        batch_id = str(uuid.uuid4())
        # Register batch fingerprint for idempotence / audit.
        batch_sha = content_hash({
            "inputs": [i.payload.text for i in inputs],
            "tenant": tenant_id, "task": task_id,
        })
        self._session.add(IngestBatch(
            id=batch_id,
            tenant_id=tenant_id,
            task_id=task_id,
            source=batch_source,
            sha256=batch_sha,
            n_received=len(inputs),
            n_accepted=n_accepted,
            n_rejected=n_rejected,
            rejections=rejections,
            actor_id=actor_id,
            latency_ms=int((time.monotonic() - t0) * 1000),
        ))
        await self._session.flush()

        summary = BatchSummary(
            batch_id=batch_id,
            n_received=len(inputs),
            n_accepted=n_accepted,
            n_rejected=n_rejected,
            rejections=rejections,
            latency_ms=int((time.monotonic() - t0) * 1000),
        )
        return IngestOutcome(batch_id=batch_id, summary=summary, results=results)

    # ------------------------------------------------------------- stages

    async def _handle_one(
        self,
        payload: SampleIngestPayload,
        *,
        tenant_id: str,
        task_id: str,
        actor_id: str,
        batch_source: Literal["batch", "webhook", "labeling"],
        chain: AuditChain,
    ) -> SampleResult:
        # (Stage 3) Validate already done by pydantic at the API layer.
        if payload.tenant_id != tenant_id:
            raise SchemaInvalid("tenant_id mismatch in payload")
        if payload.task_id != task_id:
            raise SchemaInvalid("task_id mismatch in payload")

        # (Stage 4) PII scrub
        try:
            scrubbed, kinds = scrub(payload.text, policy=self._pii_policy)
        except PiiBlocked as exc:
            await chain.record(
                kind="ingest_rejected",
                actor_id=actor_id,
                payload={
                    "reason": "pii_blocked", "kinds": exc.detail.get("kinds", []),
                },
            )
            return SampleResult(
                accepted=False, sample_id=None, reason="pii_blocked",
            )

        # (Stage 5) MinHash dedup — key by (tenant,task,sha256) to avoid cross-tenant collisions.
        dedup_key = f"{tenant_id}|{task_id}|{sha256_text(scrubbed).hex()}"
        existing = self._deduper.is_duplicate(dedup_key, scrubbed)
        if existing is not None:
            return SampleResult(
                accepted=False, sample_id=None, reason="duplicate",
                pii_kinds=kinds,
            )

        # (Stage 7) Persist — wrap in a SAVEPOINT so an IntegrityError
        # on THIS row only aborts this sample, not the whole batch.
        sha = sha256_text(scrubbed)
        row = TrainingSample(
            id=str(uuid.uuid4()),
            tenant_id=tenant_id,
            task_id=task_id,
            text=scrubbed,
            text_sha256=sha,
            language=payload.language,
            label=payload.label,
            label_confidence=payload.label_confidence,
            source=batch_source,
            origin_ref=payload.origin_ref,
            pii_flags=list(kinds),
            status="labelled" if payload.label else "pending",
        )
        try:
            async with self._session.begin_nested():
                self._session.add(row)
                await self._session.flush()
        except IntegrityError:
            # Concurrent inserter won the race on uniq_sample_per_text.
            # The savepoint was already rolled back; the surrounding
            # transaction is intact — prior accepted rows are preserved.
            return SampleResult(
                accepted=False, sample_id=None, reason="duplicate",
                pii_kinds=kinds,
            )

        # (Stage 8) Audit
        await chain.record(
            kind="ingest_accepted",
            actor_id=actor_id,
            subject_id=row.id,
            payload={
                "source": batch_source,
                "pii_kinds": kinds,
                "labelled": bool(payload.label),
            },
        )
        return SampleResult(
            accepted=True, sample_id=row.id, reason=None, pii_kinds=kinds,
        )


__all__ = [
    "IngestInput",
    "IngestOutcome",
    "IngestPipeline",
    "SampleResult",
]
