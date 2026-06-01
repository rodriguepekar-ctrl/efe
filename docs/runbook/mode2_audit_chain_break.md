# Runbook — Mode-2 Audit Chain Break

> Sprint **Φ-D-MODE2 Completion (D68)** — artefact #34.
> Cf. `docs/EFE_MODE2_ANCHORS_COMPLETION_ARCHITECTURE.md` § 12.

## Symptômes (= CRIT severity)

| Indicateur | Implication |
|---|---|
| `SLO-MODE2-AUDIT-CHAIN-INTEGRITY` fails | HMAC chain corrupted |
| `replay_audit_chain.py` reports `broken_links > 0` | Chain integrity compromise |
| Audit replay shows mismatched prev_hash | Insertion / removal detected |
| Audit replay shows mismatched hmac_sha256 | Tampering or key drift |

⚠️ **All audit chain breaks are CRIT severity** — they imply either
data corruption, malicious tampering, or HMAC key issues. Treat as
security incident until proven otherwise.

## Diagnostic chirurgical

### Étape 1 : confirm the break

```bash
# Per-tenant replay over the past 7 days.
python3 scripts/mode2/replay_audit_chain.py \
    --tenant X \
    --since $(date -u -d '7 days ago' +%FT%TZ) \
    --out /tmp/replay.json

# Examine the broken_links.
jq '.slo_compliance["SLO-MODE2-AUDIT-CHAIN-INTEGRITY"].broken_links' /tmp/replay.json
```

### Étape 2 : classify the break

| Pattern | Likely cause |
|---|---|
| `prev_hash mismatch` at index 0 (= genesis) | Chain reset (= expected on tenant creation) |
| `prev_hash mismatch` mid-chain | Insertion / removal of records |
| `HMAC mismatch` on all records | Wrong HMAC key in replay |
| `HMAC mismatch` on subset | Tampering OR key rotation |
| `chain_length = 0` | Empty chain (= no Mode-2 calls yet) |

### Étape 3 : check HMAC key correctness

```bash
# Verify current key matches the period.
vault kv get secret/efe/audit/mode2_hmac_key

# If key was rotated, replay each segment with the appropriate key :
python3 scripts/mode2/replay_audit_chain.py --tenant X \
    --since 2026-05-01 --until 2026-05-10 \
    --hmac-key-hex $OLD_KEY

python3 scripts/mode2/replay_audit_chain.py --tenant X \
    --since 2026-05-10 \
    --hmac-key-hex $NEW_KEY
```

### Étape 4 : if tampering suspected

```bash
# Cross-reference with WAL / database backups.
# Check Postgres audit_log table for record consistency :
psql -c "SELECT request_id, hmac_sha256, prev_hash FROM mode2_attestations
         WHERE tenant_id = 'X' AND request_id IN (...affected request_ids)
         ORDER BY timestamp_unix_micros"

# Compare HMAC values against database vs chain replay output.
# Diverging values → tampering.
```

## Mitigation immédiate

### Cas 1 : Key rotation handled improperly

Restart `replay_audit_chain.py` with the correct key for the period. Chain may not be "broken" after all.

### Cas 2 : Database corruption (= rare)

```bash
# Restore from last known-good backup.
pg_restore -t mode2_attestations /backups/mode2_$DATE.pg

# Re-run replay to verify integrity.
python3 scripts/mode2/replay_audit_chain.py --tenant X
```

### Cas 3 : Tampering confirmed

1. **STOP** : block writes to `mode2_attestations` via pg revoke INSERT.
2. Open security incident immediately.
3. Snapshot Postgres for forensic.
4. Notify legal + compliance team.
5. Conduct full audit chain review.

## Prevention

* HMAC key rotation : every 90 days.
* Daily automated `replay_audit_chain.py` runs via `mode2-audit-replay.yml` workflow.
* PostgreSQL row-level encryption on `mode2_attestations.hmac_sha256` column.
* Multi-region replication of audit log.

## Escalation

| Sévérité | Qui | Délai |
|---|---|---|
| Key rotation misconfig | self-resolve | 1h |
| Database corruption | `#efe-incidents` | 30min |
| Tampering suspected | PagerDuty SEV1 + Security + Legal | 5min |
| Cross-tenant leakage in chain | PagerDuty SEV1 + GDPR officer | 5min |

## Liens

* SLO : `SLO-MODE2-AUDIT-CHAIN-INTEGRITY`, `SLO-MODE2-AUDIT-COVERAGE`
* Archi § 12 : `docs/EFE_MODE2_ANCHORS_COMPLETION_ARCHITECTURE.md`
* Tool : `scripts/mode2/replay_audit_chain.py`
* Companion HMAC chain : `docs/bench/d14bis_hmac_audit_chain_architecture.md` (D14-bis baseline archi)
* Related runbook : `docs/runbooks/d14bis_audit_chain_broken.md` (D14-bis specific recovery)
