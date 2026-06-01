"""EvalService — submit / get / list / decide evaluations.

This is the caller-facing verb surface. It coordinates the state
machine, metric persistence, decision engine invocation, champion
promotion, and audit logging.

Happy path (``submit`` → ``process``):

    1. submit: create :class:`EvalRun` in ``queued``, return view.
    2. process: transition to ``running``, invoke runner, persist
       metrics → ``measured``, invoke decision engine, record audit,
       transition to ``decided`` with decision set.
    3. if PROMOTE: transition to ``promoted``, register champion,
       create rollback_monitor.
    4. if REJECT: transition to ``rejected``, record audit.
    5. if HUMAN_REVIEW: transition to ``human_review``, create
       :class:`ReviewRequest`.
"""

from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass, field
from datetime import datetime, timedelta
from typing import Any

from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from efe_gate.champion.audit import AuditChain, AuditKind
from efe_gate.champion.registry import ChampionInfo, ChampionRegistry
from efe_gate.clock import Clock, SystemClock
from efe_gate.comparator import ComparisonEngine
from efe_gate.config import (
    DecisionSettings,
    ReviewSettings,
    RollbackSettings,
    StatsSettings,
)
from efe_gate.db.models import (
    EvalMetric,
    EvalRun,
    GoldenSet,
    PromotionGate,
    ReviewRequest,
    RollbackMonitor,
)
from efe_gate.decision import (
    Decision,
    DecisionEngine,
    DecisionOutcome,
    PromotionRule,
)
from efe_gate.errors import (
    MissingRule,
    NotFound,
    TenantMismatch,
    ValidationError,
)
from efe_gate.ids import new_uuid
from efe_gate.metrics.bundle import MetricsBundle
from efe_gate.runner import EvalInputs, EvalScorer, run_evaluation
from efe_gate.state_machine import EvalState, StateMachine

# ---------------------------------------------------------------------------
# DTOs
# ---------------------------------------------------------------------------


@dataclass(frozen=True, slots=True)
class SubmitEvalInput:
    tenant_id: str
    task_id: str
    task_type: str  # "classif" | "retrieval" | "distill"
    candidate_run_id: str
    candidate_artifact_uri: str
    submitted_by: str
    golden_set_id: str | None = None
    metric_names: tuple[str, ...] = ()
    tags: dict[str, Any] = field(default_factory=dict)


@dataclass(frozen=True, slots=True)
class EvalRunView:
    id: str
    tenant_id: str
    task_id: str
    candidate_run_id: str
    candidate_uri: str
    champion_run_id: str | None
    golden_set_id: str
    state: EvalState
    decision: str | None
    decision_reason: str | None
    submitted_by: str
    created_at: datetime
    updated_at: datetime
    started_at: datetime | None
    finished_at: datetime | None
    audit_chain_hash: str | None
    tags: dict[str, Any]

    @classmethod
    def from_row(cls, r: EvalRun) -> EvalRunView:
        return cls(
            id=r.id,
            tenant_id=r.tenant_id,
            task_id=r.task_id,
            candidate_run_id=r.candidate_run_id,
            candidate_uri=r.candidate_uri,
            champion_run_id=r.champion_run_id,
            golden_set_id=r.golden_set_id,
            state=EvalState(r.state),
            decision=r.decision,
            decision_reason=r.decision_reason,
            submitted_by=r.submitted_by,
            created_at=r.created_at,
            updated_at=r.updated_at,
            started_at=r.started_at,
            finished_at=r.finished_at,
            audit_chain_hash=(
                r.audit_chain_hash.hex() if r.audit_chain_hash else None
            ),
            tags=dict(r.tags or {}),
        )

    def as_dict(self) -> dict[str, Any]:
        return {
            "id": self.id,
            "tenant_id": self.tenant_id,
            "task_id": self.task_id,
            "candidate_run_id": self.candidate_run_id,
            "candidate_uri": self.candidate_uri,
            "champion_run_id": self.champion_run_id,
            "golden_set_id": self.golden_set_id,
            "state": self.state.value,
            "decision": self.decision,
            "decision_reason": self.decision_reason,
            "submitted_by": self.submitted_by,
            "created_at": self.created_at.isoformat(),
            "updated_at": self.updated_at.isoformat(),
            "started_at": (
                self.started_at.isoformat() if self.started_at else None
            ),
            "finished_at": (
                self.finished_at.isoformat() if self.finished_at else None
            ),
            "audit_chain_hash": self.audit_chain_hash,
            "tags": dict(self.tags),
        }


# ---------------------------------------------------------------------------
# EvalService
# ---------------------------------------------------------------------------


class EvalService:
    def __init__(
        self,
        session: AsyncSession,
        *,
        state_machine: StateMachine,
        registry: ChampionRegistry,
        audit_chain: AuditChain,
        scorer: EvalScorer,
        decision_settings: DecisionSettings,
        stats_settings: StatsSettings,
        rollback_settings: RollbackSettings,
        review_settings: ReviewSettings,
        clock: Clock | None = None,
    ) -> None:
        self._session = session
        self._sm = state_machine
        self._registry = registry
        self._audit = audit_chain
        self._scorer = scorer
        self._decision_settings = decision_settings
        self._stats_settings = stats_settings
        self._rollback_settings = rollback_settings
        self._review_settings = review_settings
        self._clock: Clock = clock or SystemClock()

    # -------- submit / get / list --------

    async def submit(self, inp: SubmitEvalInput) -> EvalRunView:
        self._validate_submit(inp)
        golden = await self._pick_golden(
            inp.tenant_id, inp.task_id, inp.golden_set_id,
        )
        champion = await self._registry.get_or_none(
            inp.tenant_id, inp.task_id,
        )
        run = EvalRun(
            id=new_uuid(),
            tenant_id=inp.tenant_id,
            task_id=inp.task_id,
            candidate_run_id=inp.candidate_run_id,
            candidate_uri=inp.candidate_artifact_uri,
            champion_run_id=champion.run_id if champion else None,
            champion_artifact_uri=(
                champion.artifact_uri if champion else None
            ),
            golden_set_id=golden.id,
            state=EvalState.QUEUED.value,
            state_version=0,
            submitted_by=inp.submitted_by,
            tags=dict(inp.tags),
        )
        self._session.add(run)
        await self._session.flush()
        return EvalRunView.from_row(run)

    async def get(
        self, eval_id: str, *, tenant_id: str | None = None,
    ) -> EvalRunView:
        run = await self._get_or_raise(eval_id, tenant_id=tenant_id)
        return EvalRunView.from_row(run)

    async def list(
        self,
        *,
        tenant_id: str,
        task_id: str | None = None,
        state: EvalState | None = None,
        page_size: int = 50,
        page_token: str | None = None,
    ) -> tuple[Sequence[EvalRunView], str | None]:
        stmt = (
            select(EvalRun)
            .where(EvalRun.tenant_id == tenant_id)
            .order_by(EvalRun.created_at.desc(), EvalRun.id.desc())
        )
        if task_id is not None:
            stmt = stmt.where(EvalRun.task_id == task_id)
        if state is not None:
            stmt = stmt.where(EvalRun.state == state.value)
        offset = int(page_token) if page_token else 0
        stmt = stmt.limit(page_size + 1).offset(offset)
        rows = list((await self._session.execute(stmt)).scalars())
        next_token: str | None = None
        if len(rows) > page_size:
            next_token = str(offset + page_size)
            rows = rows[:page_size]
        return (
            tuple(EvalRunView.from_row(r) for r in rows),
            next_token,
        )

    # -------- process (drive to a decision) --------

    async def process(
        self,
        *,
        eval_id: str,
        candidate_bundle: MetricsBundle | None = None,
        champion_bundle: MetricsBundle | None = None,
    ) -> EvalRunView:
        """Advance an eval from QUEUED to DECIDED (+ PROMOTED / …).

        The ``candidate_bundle`` / ``champion_bundle`` arguments are
        injected by tests that bypass the runner. In production, if
        they are omitted the service invokes the runner.
        """
        run = await self._get_or_raise(eval_id)
        if run.state != EvalState.QUEUED.value:
            raise ValidationError(
                f"cannot process eval in state {run.state!r}",
            )
        # Transition to RUNNING.
        await self._sm.transition(
            run_id=run.id, to_state=EvalState.RUNNING,
            actor="eval-service", reason="runner started",
        )

        # Runner or injected bundles.
        if candidate_bundle is None or champion_bundle is None:
            golden = await self._session.get(GoldenSet, run.golden_set_id)
            if golden is None:
                raise NotFound(
                    f"golden set {run.golden_set_id} vanished",
                )
            inputs = EvalInputs(
                tenant_id=run.tenant_id, task_id=run.task_id,
                task_type=run.tags.get("task_type", "classif"),
                candidate_uri=run.candidate_uri,
                champion_uri=run.champion_artifact_uri,
                golden_snapshot_uri=golden.snapshot_uri,
                metric_names=tuple(
                    run.tags.get("metric_names") or ("accuracy",),
                ),
            )
            cand, champ = await run_evaluation(
                inputs,
                scorer=self._scorer,
                bootstrap_resamples=self._stats_settings.bootstrap_resamples,
                conf=self._stats_settings.default_ci,
                seed=self._stats_settings.rng_seed,
            )
            candidate_bundle = cand
            champion_bundle = champ or MetricsBundle()
        # Persist raw metrics before deciding.
        await self._persist_metrics(run, candidate_bundle)
        await self._sm.transition(
            run_id=run.id, to_state=EvalState.MEASURED,
            actor="eval-service", reason="metrics computed",
        )

        # Load rule.
        rule = await self._load_rule(run.tenant_id, run.task_id)

        # Decide.
        engine = DecisionEngine(
            comparator=ComparisonEngine(
                delta_margin_default=(
                    self._decision_settings.default_equivalence_margin
                ),
                bootstrap_resamples=self._stats_settings.bootstrap_resamples,
                conf=self._stats_settings.default_ci,
                seed=self._stats_settings.rng_seed,
            ),
        )
        decision = engine.decide(
            champion=champion_bundle or MetricsBundle(),
            candidate=candidate_bundle,
            rule=rule,
        )

        # Record DB state + audit.
        await self._apply_decision(run, decision, rule)
        await self._session.refresh(run)
        return EvalRunView.from_row(run)

    # -------- review confirmation --------

    async def confirm_review(
        self,
        *,
        eval_id: str,
        reviewer_id: str,
        approved: bool,
        reason: str = "",
        tenant_id: str | None = None,
    ) -> EvalRunView:
        """Apply a review decision. Dual sign-off enforced if required."""
        run = await self._get_or_raise(eval_id, tenant_id=tenant_id)
        if run.state != EvalState.HUMAN_REVIEW.value:
            raise ValidationError(
                f"cannot confirm review in state {run.state!r}",
            )
        # Locate review request.
        stmt = select(ReviewRequest).where(
            ReviewRequest.eval_run_id == run.id,
        )
        rr = (await self._session.execute(stmt)).scalar_one_or_none()
        if rr is None:
            raise NotFound(
                f"no review request for eval {run.id!r}",
            )
        # Append signature.
        sigs = list(rr.signatures or [])
        existing = {s["reviewer_id"] for s in sigs}
        if reviewer_id in existing:
            raise ValidationError(
                f"reviewer {reviewer_id!r} has already signed",
            )
        sigs.append({
            "reviewer_id": reviewer_id,
            "approved": bool(approved),
            "reason": reason,
            "at": self._clock.now().isoformat(),
        })
        rr.signatures = sigs

        need_two = rr.requires_dual_signoff
        sigs_approvals = [s for s in sigs if s.get("approved")]
        sigs_rejections = [s for s in sigs if not s.get("approved")]

        if sigs_rejections:
            rr.status = "rejected"
            rr.resolved_at = self._clock.now()
            await self._sm.transition(
                run_id=run.id, to_state=EvalState.REJECTED,
                actor=reviewer_id, reason=f"review rejected: {reason}",
            )
            run.decision = "reject"
            run.decision_reason = f"review rejected by {reviewer_id}"
            await self._audit.record(
                kind=AuditKind.REVIEW_OVERRIDE_REJECT,
                actor=reviewer_id, subject_id=run.id,
                payload={"reason": reason, "signatures": sigs},
            )
        elif len(sigs_approvals) >= (2 if need_two else 1):
            rr.status = "approved"
            rr.resolved_at = self._clock.now()
            await self._promote_candidate(run, actor=reviewer_id)
            await self._audit.record(
                kind=AuditKind.REVIEW_OVERRIDE_APPROVE,
                actor=reviewer_id, subject_id=run.id,
                payload={"reason": reason, "signatures": sigs},
            )
        # else: still pending, more sigs needed — no transition.
        await self._session.flush()
        await self._session.refresh(run)
        return EvalRunView.from_row(run)

    # -------- helpers --------

    async def _pick_golden(
        self, tenant_id: str, task_id: str,
        golden_set_id: str | None,
    ) -> GoldenSet:
        if golden_set_id:
            g = await self._session.get(GoldenSet, golden_set_id)
            if g is None:
                raise NotFound(
                    f"golden set {golden_set_id!r} not found",
                )
            if g.tenant_id != tenant_id or g.task_id != task_id:
                raise TenantMismatch(
                    f"golden set {golden_set_id!r} belongs to "
                    f"different tenant/task",
                )
            return g
        stmt = (
            select(GoldenSet)
            .where(GoldenSet.tenant_id == tenant_id)
            .where(GoldenSet.task_id == task_id)
            .where(GoldenSet.deprecated.is_(False))
            .order_by(GoldenSet.locked_at.desc())
            .limit(1)
        )
        g = (await self._session.execute(stmt)).scalar_one_or_none()
        if g is None:
            raise NotFound(
                f"no golden set for {tenant_id}/{task_id}",
            )
        return g

    async def _load_rule(
        self, tenant_id: str, task_id: str,
    ) -> PromotionRule:
        row = await self._session.get(PromotionGate, (tenant_id, task_id))
        if row is None:
            raise MissingRule(
                f"no promotion_gates row for {tenant_id}/{task_id}",
            )
        return PromotionRule.from_dict(dict(row.rule or {}))

    async def _persist_metrics(
        self, run: EvalRun, bundle: MetricsBundle,
    ) -> None:
        for metric in bundle.metrics.values():
            self._session.add(EvalMetric(
                id=new_uuid(),
                eval_run_id=run.id,
                metric_name=metric.name,
                role=metric.role.value,
                group_key="__all__",
                value=float(metric.value),
                ci_lower=(
                    None if metric.ci_lower is None
                    else float(metric.ci_lower)
                ),
                ci_upper=(
                    None if metric.ci_upper is None
                    else float(metric.ci_upper)
                ),
                sample_size=int(metric.sample_size),
            ))
        for g, group_metrics in bundle.by_group.items():
            for metric in group_metrics.values():
                self._session.add(EvalMetric(
                    id=new_uuid(),
                    eval_run_id=run.id,
                    metric_name=metric.name,
                    role=metric.role.value,
                    group_key=g,
                    value=float(metric.value),
                    ci_lower=(
                        None if metric.ci_lower is None
                        else float(metric.ci_lower)
                    ),
                    ci_upper=(
                        None if metric.ci_upper is None
                        else float(metric.ci_upper)
                    ),
                    sample_size=int(metric.sample_size),
                ))

    async def _apply_decision(
        self, run: EvalRun, decision: Decision, rule: PromotionRule,
    ) -> None:
        await self._sm.transition(
            run_id=run.id, to_state=EvalState.DECIDED,
            actor="decision-engine", reason=decision.reason,
        )
        run.decision = decision.outcome.value
        run.decision_reason = decision.reason

        # Persist comparison details into eval_metrics. We fetch
        # explicitly rather than via ``run.metrics`` because lazy
        # relationship loads trigger MissingGreenlet under async.
        stmt_metrics = (
            select(EvalMetric)
            .where(EvalMetric.eval_run_id == run.id)
            .where(EvalMetric.group_key == "__all__")
        )
        existing = list(
            (await self._session.execute(stmt_metrics)).scalars(),
        )
        by_name = {m.metric_name: m for m in existing}
        for cmp in decision.comparisons:
            m = by_name.get(cmp.metric_name)
            if m is None:
                continue
            m.vs_champion_delta = cmp.delta
            m.p_value = cmp.p_value
            m.passed = cmp.verdict.value in ("pass", "better")

        if decision.outcome is DecisionOutcome.PROMOTE:
            await self._promote_candidate(run, actor="eval-service")
        elif decision.outcome is DecisionOutcome.REJECT:
            await self._sm.transition(
                run_id=run.id, to_state=EvalState.REJECTED,
                actor="eval-service", reason=decision.reason,
            )
            await self._audit.record(
                kind=AuditKind.DECISION_REJECT,
                actor="eval-service",
                subject_id=run.id,
                payload={
                    "reason": decision.reason,
                    "comparisons": [c.as_dict() for c in decision.comparisons],
                },
            )
        elif decision.outcome is DecisionOutcome.HUMAN_REVIEW:
            await self._sm.transition(
                run_id=run.id, to_state=EvalState.HUMAN_REVIEW,
                actor="eval-service", reason=decision.reason,
            )
            review = ReviewRequest(
                id=new_uuid(),
                eval_run_id=run.id,
                tenant_id=run.tenant_id,
                task_id=run.task_id,
                reason=decision.reason,
                requires_dual_signoff=(
                    rule.review_policy.require_dual_signoff
                ),
                signatures=[],
                status="pending",
                deadline=self._clock.now() + timedelta(
                    hours=rule.review_policy.timeout_hours,
                ),
                created_at=self._clock.now(),
            )
            self._session.add(review)
            await self._audit.record(
                kind=AuditKind.DECISION_REVIEW,
                actor="eval-service", subject_id=run.id,
                payload={
                    "reason": decision.reason,
                    "comparisons": [
                        c.as_dict() for c in decision.comparisons
                    ],
                    "reviewers": list(rule.review_policy.reviewers),
                },
            )
        else:  # pragma: no cover — exhaustive
            raise AssertionError(
                f"unhandled decision outcome {decision.outcome}",
            )

    async def _promote_candidate(
        self, run: EvalRun, *, actor: str,
    ) -> ChampionInfo:
        # Pull the per-metric snapshot from the current eval rows.
        stmt = (
            select(EvalMetric)
            .where(EvalMetric.eval_run_id == run.id)
            .where(EvalMetric.group_key == "__all__")
        )
        rows = list((await self._session.execute(stmt)).scalars())
        snapshot: dict[str, Any] = {}
        for r in rows:
            snapshot[r.metric_name] = {
                "role": r.role,
                "value": float(r.value),
                "ci_lower": (
                    None if r.ci_lower is None else float(r.ci_lower)
                ),
                "ci_upper": (
                    None if r.ci_upper is None else float(r.ci_upper)
                ),
            }
        champion = await self._registry.promote(
            tenant_id=run.tenant_id, task_id=run.task_id,
            run_id=run.candidate_run_id,
            artifact_uri=run.candidate_uri,
            eval_run_id=run.id,
            metrics_snapshot=snapshot,
            promoted_by=actor,
        )
        # Transition eval run to PROMOTED.
        await self._sm.transition(
            run_id=run.id, to_state=EvalState.PROMOTED,
            actor=actor, reason="champion replaced",
        )
        # Create rollback monitor.
        deadline = self._clock.now() + timedelta(
            minutes=self._rollback_settings.window_minutes,
        )
        self._session.add(RollbackMonitor(
            id=new_uuid(),
            tenant_id=run.tenant_id,
            task_id=run.task_id,
            run_id=run.candidate_run_id,
            eval_run_id=run.id,
            started_at=self._clock.now(),
            deadline=deadline,
            state="watching",
            observed_metrics={},
        ))
        await self._session.flush()
        return champion

    async def _get_or_raise(
        self, eval_id: str, *, tenant_id: str | None = None,
    ) -> EvalRun:
        run = await self._session.get(EvalRun, eval_id)
        if run is None:
            raise NotFound(f"eval run {eval_id!r} not found")
        if tenant_id is not None and run.tenant_id != tenant_id:
            raise TenantMismatch(
                f"eval run {eval_id!r} in different tenant",
            )
        return run

    def _validate_submit(self, inp: SubmitEvalInput) -> None:
        if not inp.tenant_id or not inp.task_id:
            raise ValidationError("tenant_id and task_id required")
        if not inp.candidate_run_id:
            raise ValidationError("candidate_run_id required")
        if not inp.candidate_artifact_uri:
            raise ValidationError("candidate_artifact_uri required")
        if not inp.submitted_by:
            raise ValidationError("submitted_by required")


# Unused-import silencer
_ = Any


__all__ = [
    "EvalRunView",
    "EvalService",
    "SubmitEvalInput",
]
