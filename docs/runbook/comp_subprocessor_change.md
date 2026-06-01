# Runbook — Sub-processor lifecycle change (add / remove / region change)

> **Doctrine** : D-COMP-6 — Sub-processor lifecycle.
> **Symptôme** : nouveau LLM provider ajouté/retiré OU region change, OU
> `SLO-COMP-SUBPROCESSOR-INVENTORY` FAIL (= inventory > 7 jours).

## Triggers

| Action | Trigger | Notification ≥ |
|---|---|---|
| Add new provider | PR merged in `crates/efe-server/config/subprocessors.yaml` | 30 jours tenant notice |
| Remove provider | Sunset PR | 30 jours tenant notice + erasure cascade chez le provider |
| Region change | Provider expands to new region | 30 jours tenant notice |
| Security incident at provider | Breach detected | 72h tenant notice |

## Procédure d'ajout (≥ 30 jours avant prod)

1. **T-30d : DPIA update** : auto-régénération via `generate_dpia.py` pour catalogs affected.
2. **T-30d : ROPA rebuild** : `generate_ropa.py` per tenant qui utilise ce provider.
3. **T-30d : Tenant notification email** : DPO contact + 30-day preview.
4. **T-30d : Audit log** : `compliance.subprocessor_added` audité avec details.
5. **T-30d : Sub-processor inventory update** : `subprocessors.yaml` + trust center.
6. **T-7d : Confirm rollout** : aucun tenant a opt-out (= `third_party_sharing=false` exclude).
7. **T-0 : Provider enabled** : routing activé pour tenants opt-in.

## Procédure de retrait

1. **T-30d** : tenant notification + migration plan.
2. **T-7d** : start migration ; new requests routed away.
3. **T-0** : provider disabled in `subprocessors.yaml`.
4. **T+30d** : erasure cascade chez le provider (= request data deletion).

## Region change

1. **T-30d** : DPO email + region change ; tenants `data_region` strict check.
2. **T-0** : provider region updated in `PROVIDER_REGIONS` map.
3. **T+0** : `comp_data_residency_evidence` re-run.

## Mitigation provider incident

* Breach handler : `compliance.breach_detected` avec `source=subprocessor`.
* 72h notification cascade (cf. `comp_breach_response.md`).

## Cross-references

* M29 : `EFE_M29_DATA_RIGHTS_ARCHITECTURE.md` § sub-processor mgmt
* SLO : SLO-COMP-SUBPROCESSOR-INVENTORY
* Bench : `eval/scripts/comp_subprocessor_inventory_evidence.py`
