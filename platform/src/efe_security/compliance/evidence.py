"""Evidence collector — generate artifacts for auditors."""

from __future__ import annotations

from dataclasses import dataclass, field
from datetime import datetime, timezone
from typing import Any

from efe_security.compliance.ai_act import AI_ACT_MAPPING
from efe_security.compliance.gdpr import GDPR_MAPPING
from efe_security.compliance.hipaa import HIPAA_MAPPING
from efe_security.compliance.soc2 import SOC2_MAPPING

__all__ = ["ComplianceEvidence", "EvidenceCollector"]


@dataclass(frozen=True, slots=True)
class ComplianceEvidence:
    framework: str
    item: str
    title: str
    efe_controls: tuple[str, ...]
    collected_at: datetime
    metadata: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {
            "framework": self.framework,
            "item": self.item,
            "title": self.title,
            "efe_controls": list(self.efe_controls),
            "collected_at": self.collected_at.astimezone(timezone.utc).isoformat(),
            "metadata": self.metadata,
        }


@dataclass(slots=True)
class EvidenceCollector:
    """Produce a point-in-time compliance package."""

    def collect(self) -> list[ComplianceEvidence]:
        now = datetime.now(tz=timezone.utc)
        out: list[ComplianceEvidence] = []
        for a in GDPR_MAPPING:
            out.append(
                ComplianceEvidence(
                    framework="gdpr",
                    item=a.number,
                    title=a.title,
                    efe_controls=a.efe_controls,
                    collected_at=now,
                )
            )
        for s in AI_ACT_MAPPING:
            out.append(
                ComplianceEvidence(
                    framework="ai_act",
                    item=s.number,
                    title=s.title,
                    efe_controls=s.efe_controls,
                    collected_at=now,
                )
            )
        for s in SOC2_MAPPING:
            out.append(
                ComplianceEvidence(
                    framework="soc2",
                    item=s.cc,
                    title=s.title,
                    efe_controls=s.efe_controls,
                    collected_at=now,
                )
            )
        for h in HIPAA_MAPPING:
            out.append(
                ComplianceEvidence(
                    framework="hipaa",
                    item=h.section,
                    title=h.title,
                    efe_controls=h.efe_controls,
                    collected_at=now,
                )
            )
        return out
