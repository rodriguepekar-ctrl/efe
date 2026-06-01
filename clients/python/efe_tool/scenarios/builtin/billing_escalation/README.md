# Scenario: billing_escalation

**Version**: 1.0.0
**Model**: `claude-sonnet-4-6`
**Catalogs**: `support_triage`

## What this scenario does

Specialises the support router for a **billing-first** desk. The
policy tightens the fraud handling (any fraud-looking message is
escalated to the fraud team, not billing) and introduces a
three-zone decision for billing itself (act / warn / clarify).

The prompt is intentionally stricter on refund policy — it forbids
Claude from stating a refund amount without human review, which is a
common production safety gate.

## How to run

```bash
efe-admin scenario run billing_escalation \
    --user "I was charged twice for the same order"
```

## Golden cases

Five cases, biased towards billing-ish language. Includes a
deliberately-tricky case (fraud dressed as billing) to ensure the
policy routes it to the fraud team.
