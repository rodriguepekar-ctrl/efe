# Runbook — Data residency violation detected

> **Doctrine** : D-COMP-7 — Data residency strict (fail-closed).
> **Symptôme** : `audit_events` query trouve un event
> `compliance.residency_violation` OR `SLO-COMP-RESIDENCY-LEAK` FAIL.

## Detection

Source automatique : `validate_provider_for_tenant` lève
`PermissionDenied` quand un tenant `data_region=EU` tente d'utiliser un
provider US-only (= `openai`, `anthropic`).

Source CI : `comp_data_residency_evidence.py` synth_fail mode.

## Causes probables

1. **Misconfigured tenant** : `data_region` non set ou en `GLOBAL` par erreur.
2. **New provider region** : provider a ajouté une nouvelle region non mappée.
3. **Routing bypass** : un code path court-circuite `validate_provider_for_tenant`.
4. **Backend mock** : test data avec EU tenant pointé vers US.

## Triage rapide

```bash
# 1. Identifier le violation.
psql efe -c "
SELECT event_id, ts_unix_ms, tenant_id, payload_json
FROM audit_events
WHERE action = 'compliance.residency_violation'
  AND ts_unix_ms > extract(epoch from now() - interval '7 days') * 1000
ORDER BY ts_unix_ms DESC LIMIT 20;"

# 2. Vérifier tenant config.
cat crates/efe-server/config/tenants/<TENANT>.yaml | grep -i region

# 3. Vérifier provider regions.
python3 -c "
from eval.scripts.comp_data_residency_evidence import PROVIDER_REGIONS
import json
print(json.dumps({k: list(v) for k, v in PROVIDER_REGIONS.items()}, indent=2))"
```

## Mitigation immédiate

1. **Fail-closed actif** : aucun donnée n'a en réalité été routée (= `PermissionDenied` levé).
2. **Audit** : `compliance.residency_violation` audité ; pas de breach formel (= block fonctionne).
3. **Si donnée a réellement leak** : déclencher `comp_breach_response.md`.

## Mitigation durable

* Pin test `test_audit_d_comp_7_residency_strict_fail_closed`.
* Bench `comp_data_residency_evidence` nightly.
* Alert si > 5 violations / jour pour un même tenant (= misconfig).

## Cross-references

* SLO : SLO-COMP-RESIDENCY-LEAK
* Bench : `eval/scripts/comp_data_residency_evidence.py`
* Architecture : § 10
* DT : DT-COMP-LIVE-RESIDENCY-PROD (= prod multi-region verification)
