"""Notification drafting — GDPR 72h notice template."""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any

from efe_security.incident.triage import IncidentRecord

__all__ = ["DraftGdprNotice", "NotificationCenter"]


@dataclass(frozen=True, slots=True)
class DraftGdprNotice:
    severity: str
    incident_id: str
    observed_at: str
    affected_tenants: tuple[str, ...]
    affected_categories: tuple[str, ...]
    estimated_subjects: int
    measures_taken: tuple[str, ...]
    draft_text: str

    def to_dict(self) -> dict[str, Any]:
        return {
            "severity": self.severity,
            "incident_id": self.incident_id,
            "observed_at": self.observed_at,
            "affected_tenants": list(self.affected_tenants),
            "affected_categories": list(self.affected_categories),
            "estimated_subjects": self.estimated_subjects,
            "measures_taken": list(self.measures_taken),
            "draft_text": self.draft_text,
        }


_GDPR_TEMPLATE = (
    "GDPR Article 33 notification — draft.\n\n"
    "An incident of severity {severity} was detected at {observed_at}.\n"
    "Incident id : {incident_id}.\n"
    "Affected tenants : {tenants}.\n"
    "Affected data categories : {categories}.\n"
    "Estimated impacted data subjects : {subjects}.\n"
    "Measures already taken : {measures}.\n\n"
    "This document is a draft. The DPO must review and approve before "
    "transmission to the supervisory authority within 72h of detection."
)


@dataclass(slots=True)
class NotificationCenter:
    _drafts: list[DraftGdprNotice] = field(default_factory=list)

    def build_draft(
        self,
        incident: IncidentRecord,
        *,
        affected_tenants: tuple[str, ...] = (),
        affected_categories: tuple[str, ...] = ("pii",),
        estimated_subjects: int = 0,
        measures_taken: tuple[str, ...] = (),
    ) -> DraftGdprNotice:
        tenants = affected_tenants or ((incident.tenant_id,) if incident.tenant_id else ())
        text = _GDPR_TEMPLATE.format(
            severity=incident.severity.value,
            observed_at=incident.detected_at.astimezone(timezone.utc).isoformat(),
            incident_id=incident.incident_id,
            tenants=", ".join(tenants) or "N/A",
            categories=", ".join(affected_categories),
            subjects=estimated_subjects,
            measures=", ".join(measures_taken) or "none yet",
        )
        draft = DraftGdprNotice(
            severity=incident.severity.value,
            incident_id=incident.incident_id,
            observed_at=incident.detected_at.astimezone(timezone.utc).isoformat(),
            affected_tenants=tenants,
            affected_categories=affected_categories,
            estimated_subjects=estimated_subjects,
            measures_taken=measures_taken,
            draft_text=text,
        )
        self._drafts.append(draft)
        return draft

    def list(self) -> list[DraftGdprNotice]:
        return list(self._drafts)
