# Runbook — `P2_BURN_FAST`

> **Sévérité** : critical (PagerDuty).
> **SLO violé** : P2-§3.2 (POC p90 ≤ 30 min).
> **Sprint d'origine** : D19 P2-Foundation.
> **Référence architecture** : `docs/EFE_P2_SELF_SERVICE_ARCHITECTURE.md`
> § 5.3.

---

## TL;DR

Burn rate 1 h ≥ 14.4× → on consomme l'error budget mensuel **50× plus
vite que soutenable**. Au rythme actuel, les 10 % de POCs autorisés
à dépasser 30 min seront consommés en 1 h. Action immédiate requise
sinon la pénalité SLA §3.2 s'applique en fin de mois.

---

## 1. Triage initial (≤ 5 min)

### 1.1 Confirmer le burn

```bash
# Burn rate 1 h
curl -s 'http://prometheus:9090/api/v1/query?query=efe:p2_burn_rate:1h' \
  | jq '.data.result[0].value[1]'

# p90 POC sur 1 h
curl -s 'http://prometheus:9090/api/v1/query?query=efe:p2_ttfc_p90_seconds{plan_target="poc"}' \
  | jq '.data.result[0].value[1]'
```

- Si burn < 14.4 : flapping, attendre 5 min.
- Si burn ≥ 14.4 ET p90 > 1 800 : confirmé.

### 1.2 Identifier l'étape qui dérive

Dashboard `efe-p2-funnel` panel **#6 "Provisioning p99"** : un layer
× cloud est rouge ?

```bash
# Quel layer est en dérive ?
curl -s 'http://prometheus:9090/api/v1/query?query=efe:p2_provisioning_p99_seconds' \
  | jq '.data.result[] | {labels: .metric, value: .value[1]}'
```

Si **layer_2 GCP** est > 600 s par exemple → GKE control plane est
lent. Si tous les layers sont OK mais p90 dérive → c'est ailleurs
(catalog seed, first call).

### 1.3 Identifier l'étape qui drop

Dashboard panel **#8 "Top error_class"** ou :

```bash
# Top 5 abandons sur 1 h
curl -s 'http://prometheus:9090/api/v1/query?query=topk(5, sum by (step, reason) (increase(efe_p2_funnel_drop_total[1h])))' \
  | jq '.data.result[] | {labels: .metric, count: .value[1]}'
```

Cas typique → step `provisioning` + reason `cloud_quota_exceeded`
= **on a atteint un quota cloud côté Fournisseur**.

---

## 2. Causes typiques

### 2.1 Cloud quota / outage régional

**Symptôme** : `P2_PROVISIONING_LONG_TAIL` fire en plus de `P2_BURN_FAST`.

**Action** :
1. Vérifier le status page du cloud incriminé (AWS, GCP, Azure).
2. Si quota Fournisseur (EIP, EBS, …) :
   - Ouvrir un ticket urgent au cloud.
   - Temporairement rediriger les nouveaux POCs vers les autres clouds
     (`PUT /admin/p2/cloud_routing {"primary": "azure"}`).
3. Si outage régional :
   - Forcer les nouveaux POCs sur région alternative
     (`PUT /admin/p2/region_routing {"aws": "us-west-2"}`).

### 2.2 Module Terraform cassé (deploy récent)

**Symptôme** : `P2_BURN_FAST` ET les rollbacks (`efe_p2_funnel_rollback_total`)
augmentent.

**Action** :
1. Identifier le deploy récent qui touche `terraform/modules/**` :
   ```bash
   git log --since='2 hours ago' -- terraform/modules/
   ```
2. Rollback le module au commit précédent :
   ```bash
   cd terraform/modules/<cloud>
   git checkout HEAD~1
   terraform plan
   ```
3. Re-déployer le portail si nécessaire.

### 2.3 Service tiers down (Stripe, Auth0, SES)

**Symptôme** : drop élevé à étapes 3 (email) ou 4 (payment).

**Action** :
1. Status page du service tiers (status.stripe.com, status.auth0.com).
2. Fall-back :
   - Stripe webhook lag → désactiver temporairement le pré-paiement
     pour les POC (`PUT /admin/p2/payment_required=false` ; cap durée 4 h).
   - SES down → switcher Postmark : `PUT /admin/p2/email_provider=postmark`.

### 2.4 Drift portal Next.js (regression UI)

**Symptôme** : drop élevé à étape 2 (signup_form) avec error_class
`internal`.

**Action** :
1. Vérifier les logs Loki : `{app="efe-portal"} |= "error"`.
2. Rollback du portal :
   ```bash
   helm rollback efe-portal -n efe
   ```

### 2.5 Catalog template missing après refactor

**Symptôme** : drop à étape 9 (catalog_seed) avec error_class
`catalog_template_missing`.

**Action** :
1. Identifier le template manquant :
   ```bash
   kubectl logs -n efe -l app=efe-server | grep -i "template_missing"
   ```
2. Re-seed le template ou rollback du commit qui a renommé.

---

## 3. Mitigations préventives (auto-shed)

### 3.1 Désactiver les nouveaux signups (extrême)

**Quand** : si > 30 % des sessions échouent sur > 1 h.

```bash
kubectl exec -n efe efe-server-0 -- \
  curl -X POST localhost:8081/admin/p2/disable_new_signups \
    --data '{"duration_min": 30}'
```

Effet : les nouveaux `POST /v1/trial/signup` retournent
`503 Service Unavailable` avec `Retry-After: 1800`. Les sessions
en cours **ne sont pas** affectées. Les pages publiques affichent
un banner "we're sorry, signup temporarily paused".

### 3.2 Cap le rate de nouveaux POCs

```bash
# Limiter à 10 nouveaux POC / heure (au lieu de illimité)
kubectl exec -n efe efe-server-0 -- \
  curl -X PUT localhost:8081/admin/p2/rate_limit \
    --data '{"new_poc_per_hour": 10}'
```

---

## 4. Post-incident (≤ 24 h)

1. **RCA publique** sur `https://status.efe.ai/incidents/<id>`.
2. **Post-mortem interne** : template `docs/runbook/p2_post_mortem_template.md`.
3. **Ajout test régression** : si la cause est identifiée, ajouter
   un scénario chaos dans `scripts/tests/test_p2_*.py`.
4. **Promote `error_class`** : si la cause était `internal` =>
   créer un code `error_class` dédié.
5. **Vérifier signature audit** : `efe-attest activation verify`
   sur le mois en cours doit toujours PASS (sinon l'opposabilité
   est compromise).

---

## 5. Escalade

| Durée incident | Décision |
|----------------|----------|
| < 30 min | Engineer on-call gère seul |
| 30-60 min | Engager tech lead + customer success lead |
| 1-2 h | Disable new signups (§ 3.1) ; informer commercial |
| 2-4 h | Communiquer aux trials en cours via in-app banner |
| > 4 h | Activer cellule de crise ; revue ESG / EFFOR |

---

## 6. Liens utiles

- Dashboard : https://grafana.efe.ai/d/efe-p2-funnel
- Architecture P2 : `docs/EFE_P2_SELF_SERVICE_ARCHITECTURE.md`
- SLA template §3.2 : `docs/sla/sla_template.md`
- Post-mortem template : `docs/runbook/p2_post_mortem_template.md` (à livrer D20)
- Status page : https://status.efe.ai
