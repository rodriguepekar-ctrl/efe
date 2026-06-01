# Runbook — EU AI Act risk reclassification (catalog)

> **Doctrine** : D-COMP-8 — AI Act risk classification.
> **Symptôme** : un catalog change de `ai_act_risk_level`
> (minimal → limited → high) OU `SLO-COMP-AI-ACT-DPO-SIGNOFF` FAIL.

## Triggers

| Source | Trigger | Action |
|---|---|---|
| Customer use case change | Catalog purpose updated | Re-evaluate `ai_act_annex` |
| New Annex III amendment | EU AI Office publishes | Quarterly review of `eu_ai_act.md` |
| Internal audit | DPO assessment flags risk | Re-classify upward (= conservative) |
| Reclassification down | After mitigation | DPO sign-off required |

## Procédure upgrade (= raise to higher risk)

1. **DPO assessment** : DPO confirms the new risk level.
2. **Catalog YAML update** : `ai_act_risk_level` + `ai_act_annex` set.
3. **Routing impact** : if upgraded to `high`, all incoming traffic to this catalog forced Mode-3.
4. **DPIA refresh** : `generate_dpia.py` for the catalog ; DPO sign-off.
5. **ROPA update** : `generate_ropa.py` for all tenants using this catalog.
6. **Tenant notification** : 30 days notice if user-facing impact.
7. **Audit log** : `compliance.ai_act_reclassified` audité.

## Procédure downgrade

1. **DPO assessment** : mitigations sufficient to lower risk.
2. **Conservative approach** : keep Mode-3 for 30 days post-downgrade as safety net.
3. **DPIA closure** : explicit DPIA review marks the mitigations.
4. **Audit log** : `compliance.ai_act_reclassified` with reason.

## Procédure unacceptable

1. **Immediate block** : catalog set to `ai_act_risk_level=unacceptable`.
2. **`route_to_mode` refuses** : RPC returns `PermissionDenied`.
3. **Migration plan** : for existing users.
4. **Audit log** : full trail.

## Mitigation immédiate

* **Si missing risk_level** : default to `high` (= conservative fail-closed).
* **Si missing DPO signoff for high-risk** : block traffic temporarily.

## Cross-references

* SLOs : SLO-COMP-AI-ACT-HIGH-OVERSIGHT, SLO-COMP-AI-ACT-DPO-SIGNOFF
* Benches : `comp_ai_act_risk_classification_evidence.py`, `comp_ai_act_human_oversight_evidence.py`, `comp_ai_act_dpo_signoff_evidence.py`
* Doc : `docs/compliance/eu_ai_act.md`
* Architecture : § 11
