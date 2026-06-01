# 03 — Drift detection : when to re-fit your catalog

> **Time** : 15 minutes
> **Pre-requisite** : tutorial 02 completed

## Goal

Recognize the symptoms of catalog drift and know when to re-fit your classes.

## What is drift ?

Catalog drift = the distribution of incoming texts has changed enough that
your fitted classes no longer match well. Common causes :

| Cause | Example |
|---|---|
| New product feature | Customers asking about `mobile_app` when your catalog only has `desktop` |
| Seasonal change | Holiday-season queries don't match year-round examples |
| New customer segment | B2C messages while your catalog was trained on B2B |
| Vocabulary shift | New slang / jargon emerges |
| Adversarial inputs | Prompt injection attempts |

## Step 1 — Set up : load a small catalog

1. Sign in as `acme`
2. Click **Catalogs** → **＋ New catalog**
3. Name : `narrow-router`
4. Define 2 classes :
   - `account-balance` : "What's my balance", "Show me my account total", "How much money do I have"
   - `transfer-money` : "Send 50 dollars to John", "Transfer money to my savings", "Move funds to checking"
5. Register

## Step 2 — In-distribution classification (works well)

Click **Classify**, pick `narrow-router`, try :

| Input | Expected behavior |
|---|---|
| `"What's my checking balance?"` | high score `account-balance` |
| `"Move $100 to savings"` | high score `transfer-money` |

Scores should be > 0.5 (above default threshold).

## Step 3 — Out-of-distribution classification (drift signal !)

Now try inputs that don't match either class :

| Input | What happens |
|---|---|
| `"What's the weather today?"` | both classes score LOW (~0.1-0.2) |
| `"Cancel my subscription"` | both LOW |
| `"My account was hacked report fraud"` | both medium-low |

**Symptom of drift** : top-K all below threshold = your catalog isn't covering
this input.

## Step 4 — Drift visualization

1. Run 5-10 in-distribution + 5-10 out-of-distribution classifications
2. Click **Audit log**
3. Each event has `metadata.top_score`

Plot the distribution :
- In-distribution : peaks > 0.5
- Out-of-distribution : flat low (~0.1-0.3)

In production, you'd export audit log to Loki + Grafana and visualize over time.
Cf. D13 dashboard `classify_drift.md` runbook (`docs/runbooks/d13_classify_drift.md`).

## Step 5 — Three drift scenarios

### Scenario A : new class needed

You see many `"My account was hacked report fraud"` queries scoring low. Solution :
add a new class `fraud-report`.

1. Catalogs → click `narrow-router` → add new class `fraud-report` with examples
2. Save (creates v2, increments fingerprint)
3. Re-classify the same fraud query → now matches well

### Scenario B : existing class needs more examples

Top class is right but score is borderline (0.35-0.5). Solution : add more examples
to that class.

1. Edit the class
2. Add 5-10 paraphrases of the borderline queries
3. Save (creates v3)

### Scenario C : adversarial inputs

Classification works but you want to detect adversarial / out-of-scope inputs
explicitly. Solution : add an `out-of-scope` class with diverse examples.

## Step 6 — Versioning + rollback

EFE catalogs are **immutable per version**. Each `Register` creates a new version :

| Version | Fingerprint | Created at |
|---|---|---|
| v1 | a1b2c3... | 2026-04-30 12:00 |
| v2 | d4e5f6... | 2026-04-30 12:30 |
| v3 | g7h8i9... | 2026-04-30 13:00 |

The latest version is the active one for new classifications. Old versions are
preserved (in production, indefinitely ; in the playground, until restart).

If a new version performs worse, you can rollback by re-registering an earlier
fingerprint as the active version.

## Step 7 — Alerting on drift (production only)

Production EFE ships a Prometheus alert :

```yaml
- alert: D13ObsClassifyDrift
  expr: |
    quantile_over_time(0.5,
      efe_classify_top_score{tenant=~"$tenant"}[1h]
    ) < 0.4
  for: 10m
  labels:
    severity: warning
  annotations:
    summary: "Tenant {{ $labels.tenant }} classify scores dropped"
    runbook_url: https://docs.efe.ai/runbooks/d13_classify_drift
```

When the median classify score drops < 0.4 for 1 hour, the alert fires +
the runbook (`docs/runbooks/d13_classify_drift.md`) guides the operator.

## Recap

You've learned :
- Drift = input distribution changes vs catalog
- Symptom : top-K all below threshold
- 3 mitigations : add class / add examples / add out-of-scope class
- Versioning : every Register creates an immutable new version
- Production alerting via D13 dashboard

## Next : [04 — Production deployment](04_production_deploy.md)
