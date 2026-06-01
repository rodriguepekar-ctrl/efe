# Runbook — Compliance breach response (72h notify + 24h contain)

> **Doctrine** : D-COMP-4 — Breach notification ≤ 72h (GDPR Art. 33).
> **Symptôme** : un event `compliance.breach_detected` apparaît dans
> `audit_events` ; le triage classe le breach severity ≥ 3 OR PII leak.

## Activation

* Source automatique : 8 détecteurs (cf. archi § 7.2).
* Source manuelle : DPO / SecOps trigger via `efe-admin breach create`.

## Procédure ≤ 24h (containment)

1. **T+0 — Detection** : `compliance.breach_detected` audité.
2. **T+5min — Triage** : severity + category + estimated subjects count.
3. **T+10min — Containment action** : choose
   - `block_tenant_access` : flag tenant, revoke JWTs.
   - `revoke_provider_keys` : rotate sub-processor credentials.
   - `quarantine_audit_chain` : freeze HMAC key for forensic.
4. **T+1h — Forensic snapshot** : preserve evidence (= S3 immutable bucket).
5. **T+24h — Containment confirmed** : SLO `SLO-COMP-BREACH-CONTAIN-24H` PASS.

## Procédure ≤ 72h (DPO notification + authority)

1. **T+30min — DPO notified** : email + SMS + Slack (multi-channel).
2. **T+1h — Triage call** : DPO + Sec Lead + CTO.
3. **T+12h — Affected subjects list** : extract from audit chain + ROPA.
4. **T+24h — Draft authority notification** : CNIL form 33 / ICO form / etc.
5. **T+48h — Legal review** : counsel reviews authority draft.
6. **T+72h — Authority notification submitted** : `compliance.breach_authority_notified` audité.
7. **T+72h+** : individual notification (GDPR Art. 34) if high risk.

## Mitigation immédiate

* **Si data leak confirmé** : freeze tenant; rotate keys; notify subjects.
* **Si chain break détecté** : `comp_audit_chain_break.md` cross-ref.

## Post-mortem

* `compliance.breach_resolved` audité après remediation.
* Update threat model + risk register.
* Update DPIA si applicable.

## Cross-references

* SLOs : SLO-COMP-BREACH-NOTIFY-72H, SLO-COMP-BREACH-CONTAIN-24H
* Bench : `eval/scripts/comp_breach_72h_evidence.py`, `comp_breach_24h_containment_evidence.py`
* Architecture : § 7
