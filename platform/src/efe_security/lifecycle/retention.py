"""Retention categories + policies."""

from __future__ import annotations

import enum
from dataclasses import dataclass

__all__ = ["Category", "RetentionPolicy", "default_retention_policies"]


class Category(str, enum.Enum):
    inference_events_raw = "inference_events_raw"
    inference_events_scrubbed = "inference_events_scrubbed"
    baselines_active = "baselines_active"
    audit_log = "audit_log"
    incidents = "incidents"
    consent_records = "consent_records"
    artifacts_ml = "artifacts_ml"
    prompts_blob = "prompts_blob"


@dataclass(frozen=True, slots=True)
class RetentionPolicy:
    category: Category
    retention_days: int
    allow_erasure_override: bool = True
    legal_hold_exempt: bool = False


def default_retention_policies() -> dict[Category, RetentionPolicy]:
    return {
        Category.inference_events_raw: RetentionPolicy(Category.inference_events_raw, 30),
        Category.inference_events_scrubbed: RetentionPolicy(
            Category.inference_events_scrubbed, 365
        ),
        Category.baselines_active: RetentionPolicy(
            Category.baselines_active, 3650, allow_erasure_override=False
        ),
        Category.audit_log: RetentionPolicy(
            Category.audit_log,
            365,
            allow_erasure_override=False,
            legal_hold_exempt=False,
        ),
        Category.incidents: RetentionPolicy(
            Category.incidents, 2555, allow_erasure_override=False
        ),
        Category.consent_records: RetentionPolicy(
            Category.consent_records, 1095, allow_erasure_override=False
        ),
        Category.artifacts_ml: RetentionPolicy(Category.artifacts_ml, 730),
        Category.prompts_blob: RetentionPolicy(Category.prompts_blob, 0),
    }
