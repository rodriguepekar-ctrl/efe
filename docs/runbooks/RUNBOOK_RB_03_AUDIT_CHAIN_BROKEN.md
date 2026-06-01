# RB-03 — Audit chain HMAC broken (P0)

## Métadonnées

| Champ | Valeur |
|-------|--------|
| Sévérité | **P0** (compliance breach potentiel — GDPR Art. 33 timer 72 h) |
| MTTR cible | 2 h |
| Fréquence attendue | < 1 / an |
| Last reviewed | 2026-05-09 |
| Owner primaire | Compliance Engineer |
| Owner backup | Security Lead |

## Symptômes

- Alerte Prometheus : `EfeAuditChainBroken`
- Grafana panel « Audit chain integrity » → red
- Log : `audit chain verify failed: prev_hmac mismatch at audit_id=N`

## Diagnostic

### 1. Identifier audit_id + tenant impacté

```bash
kubectl exec -n efe deploy/efe-server -- /app/bin/efe-verify-audit-chain --json
```

Output : `{"first_broken_id": 12345, "tenant": "acme"}`

### 2. Lire 10 lignes audit autour du break

```sql
SELECT id, ts, tenant, mode, prev_hmac, hmac
  FROM llm_call_audit
 WHERE id BETWEEN 12340 AND 12350
 ORDER BY id;
```

### 3. Vérifier WAL Postgres

```bash
kubectl exec -n efe efe-postgres-instance1-0 -- \
  pg_waldump --start=$(get_lsn_for_audit_id 12345)
```

## Procédure de résolution

### Étape 1 : Stabiliser (≤ 5 min)

1. **STOP les écritures classify pour le tenant impacté** :

   ```bash
   efe-admin tenant suspend acme \
     --reason "AUDIT_CHAIN_BREAK_INVESTIGATION_$(date +%Y%m%d)" --yes
   ```

   ⚠️ Ne PAS suspendre tous les tenants : isolation par-tenant.

2. **Snapshot DB immédiat (forensics)** :

   ```bash
   kubectl exec -n efe efe-postgres-instance1-0 -- \
     pg_dump -t llm_call_audit -t mode3_review_queue -t ui_audit_log \
       -F custom -f /tmp/audit_snapshot_$(date +%s).pgcustom
   kubectl cp efe-postgres-instance1-0:/tmp/audit_snapshot_*.pgcustom \
     ./forensics/
   ```

### Étape 2 : Diagnostiquer (≤ 30 min)

#### Cas A — break naturel (trigger défaillant)

```sql
SELECT prosrc FROM pg_proc WHERE proname = 'audit_chain_trigger';
```

#### Cas B — manipulation manuelle (UPDATE/DELETE hors trigger)

```bash
grep "llm_call_audit" /var/log/postgresql/*.log | grep -E "UPDATE|DELETE"
```

→ **INCIDENT SECURITE** (escalation Security Lead).

#### Cas C — corruption disque

```bash
pg_checksums -D /var/lib/postgresql/data --check
```

### Étape 3 : Corriger

#### Cas A

Redéployer migration ou hotfix trigger ; recalculer hmac depuis l'audit_id le plus ancien correct.

#### Cas B

1. Escalader Security Lead, lancer forensics
2. Audit accès Postgres (qui a fait UPDATE/DELETE hors app ?)
3. **Notifier DPO sous 72 h** (GDPR Art. 33)

#### Cas C

Restore Postgres depuis backup pgBackRest le plus proche du break, replay WAL jusqu'à juste avant le break.

### Étape 4 : Vérifier

```bash
efe-verify-audit-chain --tenant=acme
# expect: OK, 100 % continuum
```

### Étape 5 : Communiquer

- **Email DPO tenant acme : OBLIGATOIRE** (GDPR Art. 33 — breach notification 72 h)
- Statuspage : « Service intermittently degraded for tenant acme — under investigation »
- Slack `#efe-incidents` : avec audit_id range impacté
- **Si Cas B : page CTO + Security Lead + Legal**

## Post-incident

- **Postmortem OBLIGATOIRE sous 72 h** (GDPR Art. 33 timeline)
- RCA template : `templates/postmortem_p0_audit.md`
- Action items :
  - Test régression ajouté
  - Trigger durci
  - Audit IAM Postgres

## Procédure de validation (NON-DESTRUCTIF en prod)

Reproduire **EN STAGING UNIQUEMENT** :

```sql
UPDATE llm_call_audit SET hmac = 'corrupted'
 WHERE id = (SELECT MAX(id) - 5 FROM llm_call_audit);
```

Doit déclencher l'alerte Prometheus dans ≤ 60 s.

**Test trimestriel** : Game Day, owner Compliance, premier mardi de chaque trimestre.

## Annexes

- Architecture audit chain D14-bis : `docs/EFE_LETTER_D_ARCHITECTURE.md`
- GDPR Art. 33 (breach notification) : 72 h
- AI Act Art. 14 : human oversight required
