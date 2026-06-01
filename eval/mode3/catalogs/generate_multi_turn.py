#!/usr/bin/env python3
"""Generate the 9 additional multi-turn bench catalogs.

Sprint **Φ-D-MODE3 Completion (D71)** — artefact #09.
Cf. ``docs/EFE_MODE3_MULTI_TURN_COMPLETION_ARCHITECTURE.md`` § 16.

Builds on the structure of ``multi-turn-routing.yaml`` (hand-curated)
to produce 9 more catalogs spanning the Mode-3 use-case spectrum.
Each catalog has 100 test sessions, each 2-5 turns long, designed to
exercise convergence at varying difficulties.

Output : ``eval/mode3/catalogs/<id>.yaml`` × 9.

Determinism : same seed → same output. Pin tested by
`test_mode3_bench_catalogs_count`.
"""

from __future__ import annotations

import hashlib
import sys
from pathlib import Path

import yaml  # type: ignore[import-not-found]

THIS_DIR = Path(__file__).parent

CATALOGS = [
    {
        "id": "multi-turn-medical",
        "description": "Medical triage multi-turn (4 risk levels)",
        "labels": [
            ("critical", "Life-threatening, immediate action"),
            ("urgent", "Serious, prompt attention"),
            ("routine", "Standard care"),
            ("informational", "Inquiry, no clinical action"),
        ],
    },
    {
        "id": "multi-turn-legal",
        "description": "Legal advice routing (6 categories)",
        "labels": [
            ("contracts", "Contract review / drafting"),
            ("disputes", "Litigation / mediation"),
            ("compliance", "Regulatory compliance"),
            ("intellectual-property", "IP / trademarks"),
            ("employment", "Employment law"),
            ("other", "Catch-all"),
        ],
    },
    {
        "id": "multi-turn-emotion",
        "description": "Emotion detection multi-turn (6 emotions)",
        "labels": [
            ("joy", "Happiness"),
            ("sadness", "Grief"),
            ("anger", "Frustration"),
            ("fear", "Anxiety"),
            ("surprise", "Astonishment"),
            ("neutral", "No marked emotion"),
        ],
    },
    {
        "id": "multi-turn-intent",
        "description": "Intent classification multi-turn (8 intents)",
        "labels": [
            ("question", "Information request"),
            ("command", "Direct imperative"),
            ("complaint", "Dissatisfaction"),
            ("compliment", "Appreciation"),
            ("smalltalk", "Casual filler"),
            ("emergency", "Urgent distress"),
            ("clarification", "Rephrase request"),
            ("other", "Catch-all"),
        ],
    },
    {
        "id": "multi-turn-finance",
        "description": "Finance request routing (5 types)",
        "labels": [
            ("loan-application", "Borrowing"),
            ("account-management", "Profile / preferences"),
            ("transaction-dispute", "Charge contestation"),
            ("investment-advice", "Portfolio / strategy"),
            ("other", "Catch-all"),
        ],
    },
    {
        "id": "multi-turn-onboarding",
        "description": "Software onboarding flow (7 stages)",
        "labels": [
            ("signup", "Account creation"),
            ("verification", "Identity check"),
            ("first-use", "First feature use"),
            ("integration", "Connect external service"),
            ("upgrade", "Plan upgrade"),
            ("offboarding", "Account closure"),
            ("other", "Catch-all"),
        ],
    },
    {
        "id": "multi-turn-product-discovery",
        "description": "Product discovery for e-commerce (5 needs)",
        "labels": [
            ("price-comparison", "Comparing prices"),
            ("specs-request", "Detailed specs needed"),
            ("availability", "In stock check"),
            ("recommendation", "Suggest products"),
            ("other", "Catch-all"),
        ],
    },
    {
        "id": "multi-turn-hr",
        "description": "HR self-service multi-turn (6 topics)",
        "labels": [
            ("payroll", "Salary / benefits"),
            ("leave", "Vacation / sick days"),
            ("performance", "Reviews / goals"),
            ("recruitment", "Job postings"),
            ("policy", "Company policies"),
            ("other", "Catch-all"),
        ],
    },
    {
        "id": "multi-turn-it-support",
        "description": "IT support multi-turn (5 categories)",
        "labels": [
            ("hardware", "Device issue"),
            ("software", "App / OS issue"),
            ("network", "Connectivity issue"),
            ("access", "Permission / login"),
            ("other", "Catch-all"),
        ],
    },
]


def gen_session(catalog_id: str, label_id: str, index: int) -> dict:
    """Build a synthetic multi-turn session deterministically.

    **Audit fix Φ-D-MODE3** : minimum 4 turns so that the convergence
    detector (= cosine ≥ 0.95 for 2 consecutive turns) has enough
    runway to detect convergence by turn 3. Sessions of 2-3 turns
    never converge under the doctrinal window=2 rule.
    """
    seed_bytes = hashlib.sha256(
        f"{catalog_id}/{label_id}/{index}".encode()
    ).digest()
    # 4-6 turns (min 4 for convergence detection with window=2)
    n_turns = 4 + (seed_bytes[0] % 3)
    turns = []
    for t in range(n_turns):
        # Synthetic content : (= mock ; live mode uses real text corpus)
        h = seed_bytes[1 + t : 4 + t].hex()
        turns.append(f"turn-{t}-content-{label_id}-{h}")
    return {
        "id": f"session-{catalog_id}-{index:03d}",
        "turns": turns,
        "expected_label": label_id,
        "expected_convergence_turn": min(n_turns, 5),
    }


def gen_catalog(spec: dict) -> dict:
    """Generate full catalog with 100 sessions."""
    labels = [
        {"id": lid, "description": ldesc}
        for lid, ldesc in spec["labels"]
    ]
    test_sessions = []
    n_per_label = max(2, 100 // len(spec["labels"]))
    idx = 0
    for label_id, _ in spec["labels"]:
        for k in range(n_per_label):
            test_sessions.append(gen_session(spec["id"], label_id, idx))
            idx += 1
    return {
        "id": spec["id"],
        "version": "1.0.0",
        "description": spec["description"],
        "labels": labels,
        "test_sessions": test_sessions,
        "metadata": {
            "total_sessions": len(test_sessions),
            "convergence_target_turns": 5,
            "max_turns": 10,
            "freeze_date": "2026-05-13",
        },
    }


def main() -> int:
    for spec in CATALOGS:
        out = THIS_DIR / f"{spec['id']}.yaml"
        catalog = gen_catalog(spec)
        out.write_text(
            yaml.safe_dump(catalog, sort_keys=False, allow_unicode=True),
            encoding="utf-8",
        )
        print(f"wrote {out} ({len(catalog['test_sessions'])} sessions)",
              file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
