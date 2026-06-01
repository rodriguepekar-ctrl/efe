# Scenario: fraud_alert

**Version**: 1.0.0
**Model**: `claude-sonnet-4-6`
**Catalogs**: `support_triage`

## What this scenario does

A fraud-first triage prompt. The policy is deliberately conservative:
even low-confidence fraud signals raise an alert, because missing
fraud is much more expensive than an extra human review.

## How to run

```bash
efe-admin scenario run fraud_alert \
    --user "Someone charged my card without my consent"
```

## Golden cases

Six cases biased towards fraud signals (EN + FR), plus one explicit
non-fraud question to verify the redirection path.

## Notes

- This scenario intentionally overrides the default confidence
  thresholds downward (≥ 0.50 triggers action) to minimise false
  negatives.
- An important invariant: even on a non-fraud label, if the user's
  text explicitly mentions unauthorised activity (in plain language),
  the prompt asks Claude to raise an alert anyway. This is a textbook
  example of the prompt-level policy that Letter C was built for.
