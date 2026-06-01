# RB-09 — `mode3_review_queue` saturée (> 1000 pending)

## Métadonnées

| Champ | Valeur |
|-------|--------|
| Sévérité | **P2** (compliance backlog) |
| MTTR cible | 2 h |
| Fréquence attendue | 1-2 / mois sur tenants Mode-3 |
| Last reviewed | 2026-05-09 |
| Owner primaire | Compliance Engineer |
| Owner backup | DPO tenant |

## Symptômes

- Métrique : `efe_mode3_review_queue_pending > 1000`
- Logs : `Mode-3 call queued for review (threshold not reached + max_turns hit)`
- Email DPO tenant : « N pending reviews »
- Grafana panel « Mode-3 review queue » → spike

## Diagnostic

### 1. Per-tenant queue size

```sql
SELECT tenant_id, COUNT(*) AS pending,
       MIN(requested_at) AS oldest_pending
  FROM mode3_review_queue
 WHERE status = 'pending'
 GROUP BY tenant_id
 ORDER BY pending DESC;
```

### 2. Convergence rate dégradé ?

```bash
curl -s "http://prometheus:9090/api/v1/query?query=sum(rate(efe_mode3_calls_total{converged=%22true%22}[1h]))/sum(rate(efe_mode3_calls_total[1h]))"
```

Cible normale : **> 70 %**. < 50 % = problème.

### 3. Pattern non-converged

```sql
SELECT proposed_label, COUNT(*) AS n
  FROM mode3_review_queue
 WHERE status = 'pending'
   AND tenant_id = '<tenant>'
 GROUP BY proposed_label
 ORDER BY n DESC
 LIMIT 10;
```

Si un label domine → bug catalog ou prompt.

## Procédure de résolution

### Étape 1 : Identifier le tenant impacté

Si **un seul tenant** a > 80 % du backlog → cible spécifique.

### Étape 2 : Action

#### Court terme : DPO traite via portail

Le DPO accède à `/reviews` sur le portail et approuve / rejette en batch :

```text
Portail : https://portal.<domain>/reviews
```

Capacité humaine ~ 50-100 reviews/heure pour un DPO entraîné.

#### Moyen terme : tuner `max_turns`

```bash
efe-admin tenant update <tenant> --max-mode3-turns 5 --yes
```

Plus de turns = plus de chance de converger sans review.

#### Long terme : analyse pattern

Si même `(catalog, sentence_pattern)` revient → ajout d'anchors :

```bash
efe-admin catalog seed <tenant> --bundle ./additional-anchors.jsonl --bundle-version v1.1.0 --yes
```

### Étape 3 : Anti-spam

Si > 5000 pending → temporairement pause Mode-3 :

```bash
efe-admin tenant update <tenant> --disable-mode3 --yes
```

Le DPO traite le backlog, puis re-enable :

```bash
efe-admin tenant update <tenant> --enable-mode3 --yes
```

### Étape 4 : Vérifier

Attendre 30 min après actions correctives :

```sql
SELECT COUNT(*) FROM mode3_review_queue WHERE status = 'pending';
```

Cible : tendance baissière.

### Étape 5 : Communiquer

- Email DPO tenant impacté : action requise
- Slack `#efe-incidents` si > 5000 pending
- Pas d'escalation automatique (P2)

## Post-incident

- Pattern récurrent (même tenant > 3 mois consécutifs) → revue contractuelle (Mode-3 inadapté ?)
- Postmortem si > 10 000 pending atteint

## Procédure de validation (Game Day trimestriel)

Inject 1500 pending en staging :

```sql
INSERT INTO mode3_review_queue (
  review_token, tenant_id, audit_id, catalog, bundle_version,
  sentence_hash, proposed_label, proposed_confidence,
  concept_stack, expires_at
)
SELECT gen_random_uuid()::text, 'staging-tenant', i,
       'default', 1, md5(random()::text),
       'test_label', 0.5,
       '{"entries":[]}'::jsonb,
       now() + interval '7 days'
  FROM generate_series(1, 1500) AS i;
```

Doit déclencher l'alerte Prometheus dans ≤ 5 min.

**Test trimestriel** : owner Compliance.

## Annexes

- Architecture Mode-3 : `docs/EFE_M22_MODE3_MULTI_TURN_ARCHITECTURE.md`
- Schema : `crates/efe-server/migrations/0011_m22_mode3_review_queue.sql`
- Portal page : `apps/portal-admin/app/(authenticated)/reviews/page.tsx`
