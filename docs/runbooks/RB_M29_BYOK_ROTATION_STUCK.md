# RB_M29_BYOK_ROTATION_STUCK — Stuck BYOK rotation

**Severity** : P2 (running > 1h).
**Trigger** : `M29ByokRotationStuck`.

## 1. Find the rotation
```sql
SELECT id, tenant_id, state, rows_reencrypted,
       now() - started_at AS dur, new_key_fpr
  FROM byok_rotation_job
 WHERE state IN ('preparing','reencrypting','committing')
 ORDER BY started_at ASC;
```

## 2. Examine progress
```sql
SELECT table_name, rows_done, checkpointed_at
  FROM byok_rotation_progress
 WHERE rotation_id = '<id>'
 ORDER BY rows_done DESC;
```

If `rows_done` is increasing : not stuck, just slow. Wait or scale worker.

If frozen for > 30 min : worker crashed mid-write.

## 3. Restart the worker
```bash
kubectl -n efe-stack rollout restart deployment efe-byok-rotation-worker
```

The worker resumes from `byok_rotation_progress` checkpoints
(idempotent). No data loss.

## 4. Reads should still work
The customer's reads should NOT be impacted — the two-key window
means both old + new keys decrypt during rotation (ADR-M29-005).

Verify by reading from a tenant table :
```sql
SELECT pgp_sym_decrypt(encrypted_payload, '<old_key>') FROM agent_state LIMIT 1;
```

## 5. Cancel if needed
```sql
-- Only safe if state IN ('pending','preparing') :
UPDATE byok_rotation_job SET state='cancelled', completed_at=NOW()
 WHERE id = '<id>' AND state IN ('pending','preparing');
```

Cancellation during `reencrypting` or later is dangerous : some rows
are already under the new key. Run the rotation to completion or
manually re-decrypt with the new key.

## 6. Post-incident
- If recurring : scale up worker resources or split rotation across
  multiple workers via partitioning by table.
- Update SLO `M29-4` if needed.
