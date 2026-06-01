# Runbook — AL Concept Drift Detected

> Sprint **Φ-D-AL Completion (D78)** — artefact #31.

## Symptômes

| Indicateur | Implication |
|---|---|
| `efe_al_drift_detected_total` rate ≥ 1 | Wasserstein > 0.15 |
| `SLO-AL-CONCEPT-DRIFT` fail | Per-label drift > 0.10 |
| Customer reports : "system seems confused about a label" | User-visible drift |

## Diagnostic

```bash
# Per-label drift breakdown.
python3 eval/scripts/al_drift_detector.py --mode live \
    --baseline-fixture /tmp/baseline.json \
    --current-fixture /tmp/current.json
```

## Common causes

* New product launch → user vocabulary shift
* Seasonal effect (= holiday queries)
* Adversarial campaign
* Pipeline change upstream (= different tokenizer)

## Mitigation

1. Increase uncertainty sampling budget temporarily.
2. Trigger retune earlier than daily cron.
3. Add new labels for the drifted classes.
4. If adversarial : engage security team.

## Escalation

| Drift magnitude | Action |
|---|---|
| W ≤ 0.10 | monitor |
| 0.10 < W ≤ 0.15 | `#efe-al` Slack |
| W > 0.15 | PagerDuty SEV3 |
| Per-label drift > 0.20 | SEV2 |
