# Runbook — `P1_BURN_FAST`

> **Sévérité** : critical (PagerDuty).
> **SLO violé** : C-1 (Mode-1 GPU p99 ≤ 200 ms).
> **Sprint d'origine** : D19 P1-Foundation.
> **Référence architecture** : `docs/EFE_P1_SURFACE_ARCHITECTURE.md`
> § 5.3.4 + § 8.3 (procédure post-mortem).

---

## TL;DR

Burn rate 1 h ≥ 14.4× → on consomme l'error budget mensuel **50× plus
vite que soutenable**. À ce rythme, les 43 min d'indispo mensuelles
sont épuisées en 1 h. Action immédiate requise.

---

## 1. Triage initial (≤ 5 min)

### 1.1 Confirmer l'alerte n'est pas une fausse alarme

```bash
# Métrique racine : p99 sur 5 min
curl -s 'http://prometheus:9090/api/v1/query?query=efe:p1_e2e_p99_seconds' \
  | jq '.data.result[0].value[1]'
```

- Si la valeur est `< 0.2` (200 ms) : possible flapping ; vérifier
  les 30 dernières minutes (panel #5 du dashboard `efe-p1-slo`).
- Si la valeur est `≥ 0.2` : confirmer en regardant le `p95` sur le
  même panel — si p95 < 100 ms et p99 > 200 ms, c'est un **problème
  de queue de distribution** (cf. § 2.3).

### 1.2 Vérifier les déploiements récents

```bash
kubectl get pods -n efe -l app=efe-server -o wide
kubectl describe deployment -n efe efe-server | grep -A2 'Conditions:'
# Récents (≤ 1 h) ?
```

Sur le dashboard `efe-p1-slo`, les annotations "Deploys" marquent les
push récents. Si la corrélation temporelle est nette, **ROLLBACK
EN PREMIER** (cf. § 3.1).

### 1.3 Vérifier la santé des GPU

```bash
kubectl exec -n efe efe-server-0 -- nvidia-smi --query-gpu=utilization.gpu,memory.used,memory.total --format=csv
```

- GPU utilization > 95 % soutenue → contention, cf. § 2.1.
- `memory.used / memory.total` > 90 % → OOM imminent, cf. § 2.2.
- 1 GPU manquant (`No devices were found`) → eviction Kubernetes,
  cf. § 2.4.

---

## 2. Causes typiques

### 2.1 GPU contention (autre tenant agressif)

**Symptôme** : `efe_p1_embed_inference_duration_seconds` p99 explose
(panel #6 dashboard) mais les autres étages restent normaux.

**Action** :
1. Identifier le tenant gourmand :
   ```bash
   kubectl logs -n efe efe-server-0 | jq 'select(.fields.budget_remaining_ms < 40) | .fields.tenant_id' | sort | uniq -c | sort -rn | head -3
   ```
2. Si **rate-limit raisonnable mais trafic burst** : auto-shed
   prend déjà le relais (gardefou #11 du doc P1) ; observer.
3. Si **rate-limit dépassé** : ouvrir un ticket support tenant
   pour upgrade plan.

### 2.2 GPU memory pressure

**Symptôme** : alerte `P1_BURN_FAST` + `efe_gpu_oom_total > 0`.

**Action** :
1. Identifier la source de pression :
   ```bash
   kubectl logs -n efe efe-server-0 | grep -i 'cuda.*memory'
   ```
2. **Si batch coalescer est OFF** : activer (`EFE_BATCH_COALESCE=true`).
3. **Si batch coalescer est ON** : réduire `EFE_BATCH_MAX_SIZE` de 32
   à 16.
4. **Si bundle a leak** : redeploy avec image précédente.

### 2.3 Tail-latency outliers (fusion ou audit chain)

**Symptôme** : p95 normal, p99 dérive. Panel #6 montre que un étage
spécifique consomme > son budget canonique.

**Top suspect 1** : Étage 7 (fusion 100 ms) — typique d'un swarm
size accidentellement gonflé (debug build, ou tenant qui pousse
`agents_count > 256`).

```bash
# Distribution des agents_count
curl -s 'http://prometheus:9090/api/v1/query?query=histogram_quantile(0.99,sum(rate(efe_swarm_agents_count_bucket[5m])) by (le))' | jq '.data.result'
```

**Top suspect 2** : Étage 10 (audit chain 4 ms) — `fsync` lent suite
à un disque saturé.

```bash
kubectl exec -n efe efe-server-0 -- df -h /var/lib/efe
```

### 2.4 GPU eviction (cluster ops a draine un node)

**Symptôme** : 1 ou 2 réplicas reportent `efe_gpu_memory_used_bytes == 0`,
les autres encaissent 100 % du trafic.

**Action** : laisser le HPA monter ; si pas de spare nodes, voir
§ 3.2 (auto-shed agressif).

---

## 3. Mitigations

### 3.1 Rollback du dernier deploy

```bash
# Identifier la dernière révision GREEN
kubectl rollout history deployment/efe-server -n efe

# Rollback à la révision N-1
kubectl rollout undo deployment/efe-server -n efe

# Confirmer
kubectl rollout status deployment/efe-server -n efe --timeout=120s
```

Vérifier que l'alerte se résout dans les **5 min** post-rollback
(dashboard panel #2 burn rate doit redescendre vers 0).

### 3.2 Activer l'auto-shed agressif (préserver le SLO au prix de l'availability)

```bash
# Sur un replica spécifique (test) :
kubectl exec -n efe efe-server-0 -- \
  curl -X POST localhost:8081/admin/shed --data '{"rate":0.25}'

# OU global via env (redeploy minimal) :
kubectl set env deployment/efe-server -n efe EFE_AUTO_SHED_RATE=0.25
```

Effet : 25 % du trafic Mode-1 retourne `RESOURCE_EXHAUSTED` (503).
Compromis documenté en § 7.1 doc P1 : on dégrade C-12 (availability)
pour préserver C-1 (latence).

### 3.3 Cap le trafic via Envoy (en upstream)

Si l'auto-shed serveur n'est pas suffisant :

```bash
kubectl patch configmap envoy-config -n efe --type merge -p '
data:
  envoy.yaml: |
    rate_limits:
    - actions:
      - { request_headers: { header_name: ":path", descriptor_key: "p1" } }
      stage: 0
      limit:
        unit: minute
        requests_per_unit: 8000   # = 133 RPS = 67 % du nominal 200
'
kubectl rollout restart -n efe deployment/envoy
```

### 3.4 Scale-up préventif

```bash
# Forcer 2 réplicas supplémentaires (au-delà du HPA)
kubectl scale deployment/efe-server -n efe --replicas=5
```

⚠️ Effet visible en 60-90 s (le temps du pre-warm de l'embedder
sur les nouveaux pods). **Pendant ce délai**, les 3 réplicas
restants encaissent ; potentiel d'aggraver le burn temporairement.

---

## 4. Post-incident (≤ 24 h)

1. **RCA publique** : poster dans `https://status.efe.ai` la nature
   de l'incident, l'impact mesuré, la résolution.
2. **Post-mortem interne** : utiliser le template
   `docs/runbook/p1_post_mortem_template.md`. Blameless review.
3. **Ajout test régression** : si le burn était dû à un bug
   identifié, ajouter un test ciblé dans
   `crates/efe-server/tests/p1_*.rs` ou un scénario chaos dans
   `tests/p1_chaos_*.rs`.
4. **Mise à jour baseline si dérive matérielle** : si le burn était
   dû à une nouvelle baseline de hardware (kernel upgrade, GPU
   driver), exécuter `efe-attest sign-baseline` et committer le
   nouveau `host_fingerprint_baseline`.

---

## 5. Escalade

| Durée incident | Décision |
|----------------|----------|
| < 15 min | Engineer on-call gère seul |
| 15-60 min | Engager le tech lead de l'équipe Core |
| 1-4 h | Inviter Director Engineering ; informer Customer Success |
| 4-12 h | Activer la cellule de crise (1 SRE + 1 dev senior + 1 PM) |
| > 12 h | Communiquer aux clients impactés sous 30 min |

---

## 6. Liens utiles

- Dashboard SLO : https://grafana.efe.ai/d/efe-p1-slo
- Traces (Tempo) : https://tempo.efe.ai/explore?datasource=tempo&queryType=traceql
- Logs (Loki) : https://loki.efe.ai/explore
- Architecture P1 : `docs/EFE_P1_SURFACE_ARCHITECTURE.md`
- SLA template : `docs/sla/sla_template.md`
- Burn calc : `docs/EFE_P1_SURFACE_ARCHITECTURE.md` Annexe D
