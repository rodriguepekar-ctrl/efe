"""Canonical valid sampling responses for M20 helper tests.

Plain module (NOT a conftest) imported absolutely by `test_m20_*` files.
Splitting these out of conftest.py keeps the global fixture surface
small, while still being shared across the 5 helper-tool test modules.
"""
from __future__ import annotations

import json


def make_summarize_response() -> str:
    """Canonical valid response for `summarize_classification`."""
    return json.dumps({
        "explanation": (
            "The sentence describes a hardware boot issue, with strong "
            "anchors on physical device terms."
        ),
        "key_factors": [
            "verb 'boot'",
            "noun 'laptop'",
            "post-update timing",
        ],
        "confidence_assessment": "high",
        "suggested_alternatives": [],
    })


def make_compare_bundles_response() -> str:
    return json.dumps({
        "accuracy_delta_pp": 1.2,
        "divergence_examples": [
            {"sentence": "x", "label_a": "a", "label_b": "b",
             "interpretation": "minor"},
        ],
        "summary": (
            "Version B improves accuracy by 1.2pp with no systematic "
            "regressions detected."
        ),
        "recommendation": "promote",
    })


def make_generate_sentences_response(count: int = 3) -> str:
    sentences = [f"Test sentence number {i}" for i in range(count)]
    return json.dumps({
        "sentences": sentences,
        "difficulty_self_assessment": {s: "medium" for s in sentences},
        "rationale": (
            "Generated balanced sentences covering varied phrasings "
            "while avoiding redundancy with the existing set."
        ),
    })


def make_confusion_response() -> str:
    return json.dumps({
        "top_confusion_pairs": [
            {"true_class": "billing", "predicted_class": "refund",
             "rate": 0.12, "interpretation": "overlap"},
        ],
        "proposed_actions": [
            {"action": "merge", "classes": ["billing", "refund"],
             "rationale": "high bidirectional confusion suggests overlap"},
        ],
        "summary": (
            "Two pairs above threshold ; merge recommended for billing/"
            "refund and split for tech_support."
        ),
    })


def make_runbook_response() -> str:
    return json.dumps({
        "runbook_markdown": (
            "# Runbook : Log Volume Excess — DRAFT\n\n## Symptoms\n\n"
            "Logging is exceeding the M19-OBS-3 threshold of 100 KB/min.\n\n"
            "## Diagnosis\n\n1. Check the otel-collector logs.\n"
            "2. Look at `efe_mcp_log_lines_total` rate.\n\n## Mitigation\n\n"
            "1. Reduce log level to WARN temporarily.\n2. Restart fluentbit.\n\n"
            "## Root cause investigation\n\nCheck recent deploys for verbose "
            "logging additions.\n\n## Permanent fix candidates\n\n"
            "Add per-tenant log rate limits.\n\n## Communication template\n\n"
            "We are seeing increased log volume in production."
        ),
        "estimated_resolution_time_min": 20,
        "primary_diagnostics": [
            "check otel-collector logs",
            "inspect fluentbit metrics",
        ],
        "risk_level": "low",
    })


__all__ = [
    "make_summarize_response",
    "make_compare_bundles_response",
    "make_generate_sentences_response",
    "make_confusion_response",
    "make_runbook_response",
]
