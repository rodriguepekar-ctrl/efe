# Runbook — GDPR Art. 17 erasure failure (≤ 30 days SLO violation)

> **Doctrine** : D-COMP-5 — Right to erasure ≤ 30 days.
> **Symptôme** : `SLO-COMP-ERASURE-30D` FAIL ; un erasure request approche
> ou dépasse les 30 jours sans cascade complète.

## Détection

* Daily : workflow `comp-bench.yml` `erasure-drill` SLO FAIL.
* Live alert : `audit_events` query : `compliance.erasure_request` sans
  matching `compliance.erasure_complete` dans les 25 jours.

## Causes probables

1. **PG cascade incomplète** : table `audit_events` ou `sessions` non purgée.
2. **S3 backup residue** : backup ZIP contient toujours le tenant data.
3. **Glacier cold storage** : archive non touchée par le cascade.
4. **Sub-processor non-cascadé** : l'erasure ne se propage pas chez le LLM provider.
5. **Job stuck** : worker erasure ne tourne plus, queue saturée.

## Triage rapide

```bash
# 1. Identifier le request ID en retard.
psql efe -c "
SELECT erasure_id, tenant_id, requested_at, status
FROM erasure_requests
WHERE status != 'complete'
  AND requested_at < NOW() - INTERVAL '25 days'
ORDER BY requested_at ASC;"

# 2. Inspecter les layers concernés.
python3 scripts/m29/inspect_erasure.py --erasure-id <ID>
# Output : PG status, S3 status, Glacier status, sub-proc status.

# 3. Re-trigger cascade.
python3 scripts/m29/retrigger_erasure.py --erasure-id <ID> --layers all
```

## Mitigation immédiate

1. **Layer-by-layer** : forcer la purge sur le layer bloquant.
2. **Sub-processor** : envoyer le DPO request au provider (= LLM).
3. **Audit** : `compliance.erasure_complete` doit être audité avec liste des layers.

## Mitigation durable

* Pin test `test_comp_erasure_30d_below_cap` doit rester vert.
* Bench `comp_erasure_30d_evidence.py --mode synth_pass` au nightly.
* Alert PagerDuty si requête > 25 jours sans complete.

## Cross-references

* M29 : `EFE_M29_DATA_RIGHTS_ARCHITECTURE.md` § 7
* SLO : SLO-COMP-ERASURE-30D
* Bench : `eval/scripts/comp_erasure_30d_evidence.py`
