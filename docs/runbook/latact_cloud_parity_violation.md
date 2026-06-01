# Runbook — Cloud parity violation (3-cloud)

> **Doctrine** : D-LATACT-6 (= 3-cloud parité strict).
> **Symptôme** : `SLO-P2-CLOUD-PARITY` FAIL — variance TTFC > 20 % entre GCP/AWS/Azure.

## Détection

* Alert : `p2_cloud_parity_bench --mode synth_pass` exit 1 en nightly.
* Dashboard `p2_activation_funnel` : barre Azure visiblement plus haute.

## Causes probables

1. **Azure AKS provisioning slow** : Standard_D4s_v3 capacity tight dans westeurope.
2. **GCP cold start** : nouvelle région sans warmpool.
3. **AWS Network latency** : VPC peering manquant dans nouvelle région.
4. **Terraform module drift** : ressources différentes entre 3 modules.
5. **Auth/Authz different** : Azure AD vs IAM vs Service Account = différentes latences.

## Triage rapide

```bash
# 1. Mesure live des p90 TTFC par cloud.
python3 eval/scripts/p2_cloud_parity_bench.py --mode synth_pass --n-per-cloud 100
# Output identifie le cloud outlier.

# 2. Vérifier ressources provisionnées équivalentes.
for cloud in aws gcp azure; do
  cd infra/terraform/$cloud
  terraform plan -no-color | grep "will be created" | wc -l
  cd -
done

# 3. Vérifier latences réseau.
for region in us-east-1 europe-west1 westeurope; do
  ping -c 5 efe-${region}.example.com
done
```

## Mitigation immédiate

1. **Capacity tight** : essayer Standard_D4s_v4 (= nouvelle génération) ou autre region (e.g. northeurope).
2. **Cold start** : warmpool 1 node minimum.
3. **Network** : peering ou Anycast IP.
4. **Terraform drift** : diff entre 3 modules, aligner sur smallest = AWS.
5. **Auth latency** : pre-warm tokens IAM/SA.

## Mitigation durable

* Pin test `test_latact_cloud_parity_under_20pct` enforce nightly.
* Module Terraform partagé (= `infra/terraform/common/`) pour les 3 clouds.
* Quarterly cross-cloud parity re-bench (= D-LATACT-6).

## Cross-references

* SLO : SLO-P2-CLOUD-PARITY, SLO-P2-TTFC-1, SLO-P2-ABANDON-1
* Bench : `eval/scripts/p2_cloud_parity_bench.py`
* DT : DT-LATACT-3CLOUDS-APPLY, DT-LATACT-CLOUD-AZURE-GOV
