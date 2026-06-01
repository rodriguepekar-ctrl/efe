# Runbook — Postmortem overdue / Compliance evidence stale

> **Severity** : major (Sev2)
> **Owner** : Engineering leadership
> **Linked SLOs** : `SLO-OPS-POSTMORTEM-VELOCITY`,
> `SLO-OPS-COMPLIANCE-POSTURE`, `SLO-OPS-TRUST-CENTER-STALE`
> **Alerts firing** : `OpsPostmortemOverdue`, `OpsComplianceControlStale`,
> `OpsTrustCenterStale`

---

## 0. TL;DR

One of three doctrines is being violated :

  * **D-OPS-5** — postmortems published ≤ 48 h after Sev1/Sev2.
  * **D-OPS-8** — every regulatory control has fresh evidence (< 30 d).
  * **D-OPS-9** — trust center regenerated ≤ 24 h.

These are **process** failures, not production outages. The fix is
**culture + cadence**, not code.

---

## 1. Postmortem velocity

```bash
python3 scripts/ops/postmortem_velocity_audit.py \
    --incident-log target/ops/incidents.jsonl \
    --window-days 30
```

For each row in the `overdue` list :

1. Identify the incident commander.
2. Page them on Slack with `@here` in `#efe-eng` and assign the
   postmortem draft owner.
3. Set a 24 h hard deadline.
4. Add the row to `docs/postmortems/<incident-id>.md` from the template.

> **Anti-pattern** : do **not** write the postmortem yourself unless
> you were on the incident. A postmortem written second-hand misses
> the cognitive trail and looks like blame avoidance.

---

## 2. Stale compliance control

```bash
python3 scripts/ops/conformity_assessment.py
```

Inspect the report :

```bash
jq '.controls[] | select(.fresh==false)' target/ops/conformity_report.json
```

For each stale control :

| Framework | Refresh action |
|---|---|
| GDPR Art. 30 (RoPA) | Regenerate via `scripts/compliance/generate_ropa.py` ; commit |
| GDPR Art. 35 (DPIA) | Re-run DPIA template for the tenant ; commit signed PDF |
| EU AI Act Art. 9 (risk register) | Open the risk register, append the last quarter's row |
| SOC 2 CC* | Cross-reference auditor's findings ; ack & commit |
| ISO 27001 Annex A | Ack against the SoA ; commit |

The evidence is committed under
`docs/compliance/evidence/<framework>/<control>.md` ; the conformity
audit picks up the new mtime automatically.

---

## 3. Trust center stale

```bash
python3 scripts/ops/trust_center_freshness_audit.py --max-age-hours 24
```

Re-run the trust-center generator (= nightly normally, but you can
force) :

```bash
# Triggers Φ-D-OPS § 14 trust generator.
make ops-trust-center
```

Inspect the resulting `trust/index.html` for the badges :

- `Master gate : PASS|BLOCK`
- `SOC 2 : current|stale`
- `GDPR Art. 30 RoPA : current|stale`

If any badge reads `stale`, fix the upstream evidence (= § 2) **before**
the trust center regenerates, otherwise it'll publish a misleading
green.

---

## 4. Escalation

If overdue ≥ 7 days, escalate to :

1. CTO / VP Eng (= owns D-OPS-5)
2. CISO (= owns D-OPS-8)
3. Head of Customer Success (= owns D-OPS-9 / trust center)

Block the next release until resolved (= use the master gate's
`--max-fail 0` flag).

---

## 5. Cross-references

- [`scripts/ops/postmortem_velocity_audit.py`](../../scripts/ops/postmortem_velocity_audit.py)
- [`scripts/ops/conformity_assessment.py`](../../scripts/ops/conformity_assessment.py)
- [`scripts/ops/trust_center_freshness_audit.py`](../../scripts/ops/trust_center_freshness_audit.py)
- [`docs/postmortems/_template.md`](../postmortems/_template.md)
- [`docs/EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md`](../EFE_OPERATIONAL_PRODUCTION_ARCHITECTURE.md) § 10 + § 13
