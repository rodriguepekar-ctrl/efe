# EFE Runbooks

Runbooks d'incident formalisés pour les opérateurs EFE.

## Catalogue

| ID | Titre | Sévérité | MTTR | Statut |
|----|-------|----------|------|--------|
| [RB-01](./RUNBOOK_RB_01_DB_DISK_FULL.md) | DB Postgres saturé (> 85 %) | P1 | 30 min | ✅ GA |
| [RB-02](./RUNBOOK_RB_02_LLM_BREAKER_OPEN.md) | LLM circuit breaker ouvert | P2 | 15 min | ✅ GA |
| [RB-03](./RUNBOOK_RB_03_AUDIT_CHAIN_BROKEN.md) | Audit chain HMAC broken | P0 | 2 h | ✅ GA |
| [RB-04](./RUNBOOK_RB_04_KMS_BYOK_ROTATION.md) | Rotation forcée BYOK | P0 | 4 h | ✅ GA |
| [RB-05](./RUNBOOK_RB_05_BUDGET_EXHAUSTED.md) | BudgetExhausted non récupérable | P3 | 4 h | ✅ GA |
| [RB-06](./RUNBOOK_RB_06_CRASH_LOOP.md) | efe-server CrashLoopBackOff | P1 | 30 min | ✅ GA |
| [RB-07](./RUNBOOK_RB_07_LATENCY_P99.md) | Latence p99 classify > 2× SLO | P2 | 1 h | ✅ GA |
| [RB-08](./RUNBOOK_RB_08_MIGRATION_FAILED.md) | Migration DB échouée | P1 | 1 h | ✅ GA |
| [RB-09](./RUNBOOK_RB_09_REVIEW_QUEUE_SATURATED.md) | mode3_review_queue saturée | P2 | 2 h | ✅ GA |
| [RB-10](./RUNBOOK_RB_10_TLS_CERT_EXPIRY.md) | Certificat TLS expirant | P2 | 2 h | ✅ GA |
| [RB-11](./RUNBOOK_RB_11_LLM_QUOTA_NEAR_FULL.md) | Quota tokens LLM à 95 % | P3 | 4 h | ✅ GA |
| [RB-12](./RUNBOOK_RB_12_PG_FAILOVER.md) | Failover Postgres | P1 | 30 min | ✅ GA |

## Format

Chaque runbook respecte un template strict :

1. **Métadonnées** : sévérité, MTTR cible, fréquence attendue, owners
2. **Symptômes** : alertes Prometheus, métriques Grafana, log patterns
3. **Diagnostic** (≤ 3 commandes)
4. **Procédure de résolution** (5 étapes time-boxées)
5. **Communication** (statuspage, Slack, email tenant, page CTO si P0/P1)
6. **Post-incident** (postmortem requis si P0/P1)
7. **Procédure de validation** (Game Day reproductible en staging)

## Game Days

Tous les runbooks doivent être testés trimestriellement en staging.

Calendrier 2026 :
- Q1 : 2026-01-07 — RB-01, RB-02, RB-03
- Q2 : 2026-04-07 — RB-04 (dry-run), RB-05, RB-06
- Q3 : 2026-07-07 — RB-07, RB-08, RB-09 (post Sprint C)
- Q4 : 2026-10-07 — RB-10, RB-11, RB-12 (post Sprint C)

## Validation automatique

```bash
./scripts/runbook-validate.sh --all --env staging
```

Exécute la procédure de validation de chaque runbook contre le cluster `staging`. Échec → ticket Jira auto-ouvert.
