"""Compliance mappings — GDPR, AI Act, SOC 2, HIPAA."""

from __future__ import annotations

from efe_security.compliance.ai_act import AI_ACT_MAPPING, AiActSystem
from efe_security.compliance.evidence import (
    ComplianceEvidence,
    EvidenceCollector,
)
from efe_security.compliance.gdpr import GDPR_MAPPING, GdprArticle
from efe_security.compliance.hipaa import HIPAA_MAPPING, HipaaControl
from efe_security.compliance.soc2 import SOC2_MAPPING, Soc2Control

__all__ = [
    "AI_ACT_MAPPING",
    "AiActSystem",
    "ComplianceEvidence",
    "EvidenceCollector",
    "GDPR_MAPPING",
    "GdprArticle",
    "HIPAA_MAPPING",
    "HipaaControl",
    "SOC2_MAPPING",
    "Soc2Control",
]
