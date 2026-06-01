# Runbook — Mode-3 Encryption Key Rotation

> Sprint **Φ-D-MODE3 Completion (D73)** — artefact #34.
> Cf. `docs/EFE_MODE3_MULTI_TURN_COMPLETION_ARCHITECTURE.md` § 12.

## Mission

Rotate per-tenant AES-256-GCM session encryption keys every 90 days.
Old keys retained 180 days for replay. New sessions use new key
immediately ; in-flight sessions complete with their original key.

## Symptômes (= scheduled trigger)

* Key age ≥ 90 days for a tenant.
* Alert `EFE_MODE3_KEY_ROTATION_DUE` fires from monitoring.

## Procédure

### Step 1 : Generate new key version

```bash
# Generate new key + seal via KMS.
efe-admin keys rotate --service mode3 --tenant tenant-acme

# Verifies :
# - Old key version : v3
# - New key version : v4
# - KMS-sealed key blob stored in secret store
```

### Step 2 : Activate new key for new sessions

```bash
# Update tenant config.
efe-admin tenants update --tenant tenant-acme \
    --field mode3.encryption.key_version --value 4
```

New session creates use v4 immediately. In-flight sessions continue
with v3 until they terminate.

### Step 3 : Background re-encryption (= optional, post-rotation)

For long-running sessions that won't terminate quickly :

```bash
# Re-encrypt session state with v4 (= preserves session continuity).
efe-admin mode3 reencrypt-sessions --tenant tenant-acme --from v3 --to v4

# Monitor progress.
watch -n 5 'kubectl logs -f deployment/efe-server | grep reencrypt'
```

### Step 4 : Verify v3 still works for replay

```bash
# Pick a v3-encrypted session from the last 30 days.
SESSION_ID=$(psql -t -c "SELECT session_id FROM mode3_session_state
                         WHERE tenant_id = 'tenant-acme'
                         AND key_version = 3
                         LIMIT 1")

# Decrypt + verify.
efe-admin mode3 verify-decrypt --session $SESSION_ID
```

### Step 5 : Schedule v3 deletion (= 180 days post-rotation)

```bash
# Calendar event : delete v3 key 180 days from rotation date.
efe-admin keys schedule-delete --service mode3 --tenant tenant-acme \
    --key-version 3 --at $(date -u -d '+180 days' +%FT%TZ)
```

## Pitfalls

| Mistake | Consequence |
|---|---|
| Delete v3 before 180 days | Replay tool fails for old sessions ; audit gap |
| Rotate without re-encryption job | Long sessions can't access new turns (= corrupted state) |
| Skip KMS seal | Key visible in DB → CRIT compliance breach |
| Rotate during high-traffic | Brief latency spike on key cache refresh |

## Verification post-rotation

```bash
# All new sessions use new key.
psql -c "SELECT key_version, count(*) FROM mode3_session_state
         WHERE tenant_id = 'tenant-acme'
         AND created_at_unix_micros > extract(epoch from now() - interval '1 hour') * 1e6
         GROUP BY key_version"
# Expected : only key_version = 4

# Old key still decrypts replay.
python3 tests/mode3_encryption_adversarial.py --tenant tenant-acme --key-version 3
```

## Escalation

| Sévérité | Qui | Délai |
|---|---|---|
| Routine rotation | self-execute on schedule | 90-day calendar |
| Re-encryption job stuck | `#efe-mode3` Slack | 4h |
| Key derivation failure | PagerDuty SEV3 | 1h |
| Plaintext exposure detected | PagerDuty SEV1 + Security | 5 min |

## Liens

* SLO : `SLO-MODE3-ENCRYPTION-AT-REST`
* Archi § 12 : `docs/EFE_MODE3_MULTI_TURN_COMPLETION_ARCHITECTURE.md`
* Test : `tests/mode3_encryption_adversarial.py`
