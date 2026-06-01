# EFE — Runbook d'opérations

> Version : 1.0 (MVP). Maintenu par l'équipe core EFE.
> Chaque section d'incident correspond à un lien canonique référencé
> depuis les alertes Prometheus (voir `deploy/helm/efe/templates/prometheusrule.yaml`).

## 1. Généralités

### 1.1 Architecture

EFE est un service gRPC Rust mono-processus. Voir [`M12_ARCHITECTURE.md`](../M12_ARCHITECTURE.md)
pour la topologie complète.

### 1.2 On-call

- **Rotation** : `efe-oncall` sur PagerDuty (cycle hebdomadaire).
- **Escalade** : après 15 min sans prise en charge, l'alerte escalade
  au manager on-call puis au staff engineer.

### 1.3 Canaux

- **Alertes** : `#alerts-efe` (Slack), PagerDuty.
- **Incidents** : `#incident-efe`. Tout incident Sev1/Sev2 crée
  automatiquement une war-room.
- **Post-mortem** : template dans §4.

## 2. Procédures courantes

### 2.1 Déploiement standard

```sh
helm upgrade --install efe ./deploy/helm/efe \
  --namespace efe --values values.prod.yaml \
  --set image.digest=sha256:<digest>
kubectl -n efe rollout status deploy/efe --timeout=120s
EFE_HOST=... scripts/smoke.sh
```

### 2.2 Rollback

```sh
helm rollback efe --namespace efe
kubectl -n efe rollout status deploy/efe --timeout=120s
EFE_HOST=... scripts/smoke.sh
```

Le rollback préserve l'audit log (append-only sur PVC).

### 2.3 Rotation de clés JWT

1. Ajouter la nouvelle clé K2 en **première ligne** du Secret
   `efe-hs256` — K1 reste présente pour la validation.
2. Attendre la propagation du Secret (≤ 60 s via
   external-secrets + reloader) ; vérifier via
   `kubectl -n efe logs deploy/efe | grep "HS256 keys loaded"`.
3. Émetteur de tokens : basculer sur K2 pour la signature.
4. Attendre 24 h (TTL max des tokens émis).
5. Retirer K1 du Secret.

### 2.4 Nettoyage d'audit

Le PVC est 5 Gi par défaut. Rotation via un CronJob `logrotate`
hebdomadaire (à activer séparément — hors chart MVP). En urgence :

```sh
kubectl -n efe exec deploy/efe -- sh -c 'true > /var/lib/efe/audit/audit.jsonl'
# …ou mieux : snapshot + vide
kubectl -n efe exec deploy/efe -- cp /var/lib/efe/audit/audit.jsonl /tmp/audit.bak
```

> ⚠️ Ne jamais supprimer le fichier pendant que le serveur écrit :
> l'`AuditSink` de M11 garde un file descriptor ouvert.

## 3. Incidents

### <a id="high-error-rate"></a>3.1 `#high-error-rate`

**Sévérité** : warning
**Signal** : `EFEHighErrorRate` (rate(`efe_grpc_requests_total{code!="OK"}`) > seuil).

**Impact client** : erreurs gRPC visibles (codes 13/14 majoritaires).

**Diagnostic** :
```sh
# Top des codes d'erreur
curl -s <metrics-url> | grep '^efe_grpc_requests_total{code!="OK"' | sort -k2 -n
# Logs récents
kubectl -n efe logs -l app.kubernetes.io/name=efe --tail=200 | jq 'select(.level=="ERROR")'
# Top tenants impactés
curl -s <metrics-url> | grep '^efe_errors_total{' | sort -k2 -nr | head
```

**Mitigations** :
- Rollback si lié à une release : voir §2.2.
- Si un tenant flood : augmenter temporairement `rateLimit.defaultCapacity`
  via `helm upgrade --set config.rateLimit.defaultCapacity=50`.
- Si un backend externe est en panne : isoler via feature flag.

### <a id="high-latency"></a>3.2 `#high-latency`

**Sévérité** : warning
**Signal** : `EFEHighLatencyP95` (p95 Execute > 900 ms).

**Diagnostic** :
```sh
# p50 / p95 / p99
curl -s <metrics-url> | grep 'efe_grpc_request_duration_seconds_bucket{method="/efe.v1.SwarmService/Execute"'
# CPU throttling ?
kubectl -n efe top pods
# In-flight saturation ?
curl -s <metrics-url> | grep 'efe_grpc_requests_in_flight'
```

**Mitigations** :
- Scale-up manuel : `kubectl -n efe scale deploy/efe --replicas=10`.
- Augmenter les limites CPU si throttling.
- Si effondrement latent (mode collapse) : voir §3.4.

### <a id="pod-restart-loop"></a>3.3 `#pod-restart-loop`

**Sévérité** : critical
**Signal** : `EFEPodRestartLoop`.

**Diagnostic** :
```sh
kubectl -n efe describe pod -l app.kubernetes.io/name=efe
kubectl -n efe logs --previous deploy/efe --tail=200
```

**Causes fréquentes** :
- OOMKilled : voir `Last State: Terminated / Reason: OOMKilled`.
- Config invalide (ConfigError au bootstrap) : log `parse ServerConfig`.
- PVC not bound : `kubectl -n efe describe pvc efe-audit`.
- Image pull failure : `ImagePullBackOff`.

**Mitigations** :
- OOM : augmenter `resources.limits.memory` ou réduire `maxConcurrentStreams`.
- Config : corriger le ConfigMap puis `helm upgrade`.
- PVC : vérifier la storageClass disponible.

### <a id="confidence-drop"></a>3.4 `#confidence-drop`

**Sévérité** : warning
**Signal** : `EFEConfidenceDrop` (p50 confidence < seuil).

**Diagnostic** :
```sh
# Histogramme de confidence
curl -s <metrics-url> | grep '^efe_swarm_confidence'
# Taux de mode_collapse
curl -s <metrics-url> | grep 'efe_swarm_duration_seconds_count{verdict="mode_collapse"'
```

**Mitigations** :
- Si `mode_collapse` élevé : vérifier les poids des templates (M3) — une
  dégradation des weights corrompus est probable.
- Augmenter `max_rounds` via la SwarmSpec côté client (pas dans le
  serveur).

### <a id="rate-limit-storm"></a>3.5 `#rate-limit-storm`

**Sévérité** : warning
**Signal** : `EFERateLimitStorm`.

**Diagnostic** :
```sh
# Tenants les plus drop
curl -s <metrics-url> | grep 'efe_ratelimit_drops_total' | sort -k2 -nr | head
```

**Mitigations** :
- Si tenant malveillant : bloquer en amont (WAF / ingress).
- Si croissance organique : augmenter temporairement le quota
  `rateLimit.defaultCapacity`.

### <a id="audit-backpressure"></a>3.6 `#audit-backpressure`

**Sévérité** : warning
**Signal** : `EFEAuditBackpressure` (PVC < seuil de libre).

**Diagnostic** :
```sh
kubectl -n efe exec deploy/efe -- du -sh /var/lib/efe/audit
kubectl -n efe get pvc efe-audit
```

**Mitigations** :
- Snapshot puis tronquer (cf. §2.4).
- Si urgent : étendre le PVC (`kubectl edit pvc efe-audit` si la
  `StorageClass` supporte `allowVolumeExpansion: true`).
- Long-terme : CronJob de rotation/archivage S3 nightly.

### <a id="policy-rejection-spike"></a>3.7 `#policy-rejection-spike`

**Sévérité** : warning
**Signal** : `EFEPolicyRejectionSpike`.

**Diagnostic** :
```sh
# Répartition par code d'invariant (I1..I7)
curl -s <metrics-url> | grep 'efe_policy_rejections_total'
# Audit JSONL pour les raisons exactes
kubectl -n efe exec deploy/efe -- tail -n 200 /var/lib/efe/audit/audit.jsonl \
    | jq 'select(.event == "policy.rejected") | {code,reason}'
```

**Mitigations** :
- I1 / I2 (contraction / inclusion) : vérifier la logique côté client.
- I7 (quota tenant) : augmenter le quota via l'admin API ou le bridge.

### <a id="bootstrap-failure"></a>3.8 `#bootstrap-failure`

Le pod crash avant `rollout status` complet.

**Diagnostic** :
```sh
kubectl -n efe logs deploy/efe --tail=100
```

**Causes typiques** :
- `parse ServerConfig from environment` : env var malformée.
- `auth.hs256_secrets: auth is enabled but no HS256 secrets were loaded` :
  Secret non monté, mauvais path.
- `bind metrics endpoint` : port déjà pris (rare en k8s).

**Action** : corriger la configuration, `helm upgrade`.

### <a id="secret-rotation"></a>3.9 `#secret-rotation`

Voir §2.3. Si la rotation a accidentellement supprimé la dernière clé :

```sh
# Restaurer depuis backup Vault
vault kv get -format=json secret/efe/hs256 | jq -r '.data.data.keys'
# Patch inline (DERNIER RECOURS)
kubectl -n efe patch secret efe-hs256 --type=merge -p '{"stringData":{"hs256":"<b64-key>\n"}}'
```

### <a id="rollback"></a>3.10 `#rollback`

Voir §2.2. Si le rollback Helm échoue lui-même :

```sh
helm history efe --namespace efe
helm rollback efe <revision-N-1> --namespace efe --wait --timeout 2m
# En dernier recours : redéployer la version précédente à la main
kubectl -n efe set image deploy/efe efe=ghcr.io/efe-project/efe@sha256:<digest-N-1>
```

### <a id="per-tenant-error-rate"></a>3.11 `#per-tenant-error-rate`

Alerte `EfeTenantHighErrorRate` — un tenant spécifique dépasse 10 %
d'erreurs sur 5 min. Différent de `#high-error-rate` (cluster-wide).

**Triage** :

```sh
# Identifier le tenant concerné (label `tenant_id` dans l'alerte).
kubectl -n efe logs deploy/efe-server --tail=500 | grep '"tenant_id":"<TENANT>"'

# Top des codes d'erreur pour ce tenant (Loki).
{app="efe", tenant="<TENANT>"} | json | level="error"
```

**Causes courantes** : tenant qui envoie des inputs malformés
(payload trop gros, dim mismatch), tenant suspended dont les
clients existants utilisent des JWT pre-suspend (D15-C1 fixe ça),
catalog corrompu côté tenant. Si l'erreur est concentrée sur un
seul tenant et n'affecte pas le reste, **isoler-le** plutôt que
rollback global.

### <a id="tenant-rate-limit"></a>3.12 `#tenant-rate-limit`

Alerte `EfeTenantRateLimitSaturated` — un tenant a saturé son
bucket pendant > 5 min consécutives.

**Triage** :

```sh
# Quel tenant + quel rate ?
kubectl -n efe exec deploy/efe-server -- \
    curl -s localhost:9090/metrics | grep efe_ratelimit_drops_total
```

**Causes courantes** : (1) tenant légitime qui a un pic de trafic
attendu — augmenter `EFE_RATELIMIT_CAPACITY` côté config si SLA
le permet ; (2) client compromis qui DoS — suspend le tenant via
`efe-onboard suspend <TENANT>` (audit trail D14-bis) ; (3)
mauvaise config — `EFE_RATELIMIT_REFILL` trop bas.

### <a id="tenant-cone-storm"></a>3.13 `#tenant-cone-storm`

Alerte `EfeTenantConeRegisterStorm` — un tenant register-cone
plus de N fois par minute. Indique soit (a) catalog corruption
forçant des re-registers, soit (b) client buggé qui re-registers
les mêmes cones en boucle.

**Triage** :

```sh
# Voir les register events récents (audit log D14-bis).
kubectl -n efe exec deploy/efe-server -- \
    psql -c "SELECT timestamp, payload FROM audit_events \
             WHERE action='cone.register' AND tenant_id='<TENANT>' \
             ORDER BY timestamp DESC LIMIT 10;"
```

**Mitigation** : si client buggé, escalade au support pour
contacter le client. Si catalog corruption, voir
`#confidence-drop` puis le runbook de catalog-evolution.

### <a id="cardinality-bomb"></a>3.14 `#cardinality-bomb`

Alerte `EfeTenantCardinalityExplosion` — un tenant a fait exploser
sa cardinality budget (e.g. trop de catalog versions, trop de
unique class names). Cf. architecture D13 § 9 cardinality budget.

**Triage** :

```sh
# Top tenants par row count dans catalogs.
kubectl -n efe exec deploy/efe-server -- \
    psql -c "SELECT tenant_id, COUNT(*) FROM catalogs \
             GROUP BY tenant_id ORDER BY 2 DESC LIMIT 5;"
```

**Causes courantes** : (1) tenant qui register catalog versions
sans nettoyer les vieilles (rotation manquante) ; (2) attaque
intentionnelle (rare). **Mitigation** : exécuter `efe-onboard
upgrade-catalog --tenant=<X> --gc-old-versions` (D16 path) puis
re-vérifier la cardinality. Si le tenant est legit mais a
légitimement besoin de plus, augmenter `EFE_CATALOG_MAX_VERSIONS_PER_NAME`
côté Helm values.

## 4. Template post-mortem

```
# Post-mortem — <titre incident>

**Date** : YYYY-MM-DD HH:MM UTC
**Sévérité** : Sev1 / Sev2 / Sev3
**Durée** : HH:MM
**Impact** : clients affectés, RPCs affectés, requêtes perdues

## Timeline
- HH:MM — détection (alerte / ticket / report manuel)
- HH:MM — prise en charge par on-call
- HH:MM — diagnostic posé
- HH:MM — mitigation appliquée
- HH:MM — résolution confirmée

## Cause racine

## Facteurs aggravants

## Actions correctives (avec dates cibles)
- [ ] …

## Leçons apprises
```

## 5. Contacts d'escalade

| Rôle                    | Contact                       |
|-------------------------|-------------------------------|
| On-call primaire        | PagerDuty schedule `efe-oncall` |
| On-call secondaire      | Manager plateforme            |
| Staff engineer          | @efe-staff Slack              |
| Product                 | @efe-pm Slack                 |
| Sécurité (fuite PII)    | `security@example.com`        |
| SRE plateforme          | `#sre-help`                   |

## 6. Liens outils

- Dashboard Grafana : `https://grafana.example.com/d/efe-overview`
- Logs Loki : `https://loki.example.com/?query={app="efe"}`
- Traces Jaeger : `https://jaeger.example.com/search?service=efe`
- Metrics Prometheus : `https://prometheus.example.com/graph`
- Repo : `https://github.com/efe-project/efe`
- Runbook (ce document) : `https://github.com/efe-project/efe/blob/main/docs/runbook.md`
- SLO : [`docs/slo.md`](./slo.md)
- Architecture M12 : [`M12_ARCHITECTURE.md`](../M12_ARCHITECTURE.md)
