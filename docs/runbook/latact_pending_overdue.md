# Runbook — Meta-SLO PENDING-AGE overdue (> 90 jours)

> **Doctrine** : D-LATACT-1 (= aucun PENDING > 90 jours) + D-LATACT-10 (= meta-SLOs).
> **Symptôme** : `SLO-LATACT-PENDING-AGE` FAIL — un SLO `PENDING_*` reste dans `bench_baselines/v*.yaml` > 90 jours sans bascule ACTIVE.

## Détection

* Daily : workflow `latact-contract` job `pending-age-scan` FAIL.
* PagerDuty : `SLO-LATACT-PENDING-AGE` alarm (= severity major).
* Linear : auto-créé un ticket "Φ-D-LATACT.A1 — bascule overdue".

## Causes probables

1. **PR de bascule oubliée** : l'infrastructure est livrée mais le YAML pas updaté.
2. **Bench non-stable** : 3 runs consécutifs flaky empêchent l'activation.
3. **Dependency externe** : runner GPU pas encore provisionné après 90j.
4. **Scope creep** : le SLO original n'est plus pertinent.

## Triage rapide

```bash
# 1. Liste des PENDINGs overdue.
python3 eval/scripts/registry_age_scan.py --mode synth_pass --out /tmp/age.json
jq '.slo_compliance."SLO-LATACT-PENDING-AGE".pendings[] | select(.over_cap == true)' /tmp/age.json

# 2. Pour chaque SLO overdue, vérifier si bench source existe + green.
for slo in $OVERDUE_SLOS; do
  echo "=== $slo ==="
  grep -r "$slo" eval/scripts/
done
```

## Mitigation immédiate

1. **PR oubliée** : finaliser PR `status: ACTIVE` + 3 runs verts.
2. **Bench flaky** : investigate seed/distribution, stabiliser.
3. **Dépendance externe** : escalate à CTO + reschedule expected activation.
4. **Scope creep** : status `WONTFIX` + doc rationale dans archi.

## Mitigation durable

* Bot Slack notification 7 jours avant l'horizon 90j → permet anticipation.
* Pin test `test_latact_registry_age_scan_no_overdue` PR-gate sur tout PR.
* Quarterly review compliance audit (= D-LATACT-1 enforce).

## Cross-references

* SLO : SLO-LATACT-PENDING-AGE (meta)
* Bench : `eval/scripts/registry_age_scan.py`
* DT : DT-LATACT-PENDING-90D-PROD
