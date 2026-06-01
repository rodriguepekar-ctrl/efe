# RB_M29_ERASURE_VERIFY — Verify a tenant erasure completed

**Severity** : P1 (RGPD compliance critical).
**Owner** : SRE + DPO.
**Trigger** : `M29ErasureStuckInProgress` OR post-erasure audit.

## 1. Confirm state
```sql
SELECT id, state, requested_at, completed_at,
       now() - completed_at AS dur_since_done
  FROM erasure_request
 WHERE tenant_id = '<slug>'
 ORDER BY requested_at DESC LIMIT 1;
```

## 2. Schema-traversal verification

Run the verification script (validates every tenant-scoped table is
clean) :

```bash
psql -d efe -f scripts/m29/verify_erasure_complete.sql -v tenant_id='<slug>'
```

Expected output : "0 orphan rows" for every table.

If any table reports rows : the cascade missed it. Investigate via :
```sql
\dt+
-- check which tables have tenant_id columns that weren't in cascade.
SELECT table_name FROM information_schema.columns
WHERE column_name = 'tenant_id'
  AND table_schema = 'public';
```

## 3. Pseudonymization integrity

For preserved audit rows :
```sql
SELECT count(*) FROM agent_audit
 WHERE tenant_id = '<slug>'
   AND actor_subject NOT LIKE 'sha256:%';
```
Must return 0 — all actor fields should be SHA-256 hashes after
pseudonymisation.

## 4. Final audit URL pushed

```sql
SELECT final_audit_url FROM erasure_request
 WHERE tenant_id = '<slug>' AND state = 'done';
```

If `final_audit_url IS NULL` and the customer registered an export
sink : the worker failed at push. Investigate worker logs.

## 5. Issue compliance certificate

The DPO emails a "RGPD Art. 17 completion certificate" to the
customer with :
- Date of request, date of completion.
- Number of records deleted.
- SHA-256 hash of the (pseudonymized) audit chain final row.
- Reference to the customer's S3 sink push (if applicable).

Template in `docs/templates/RGPD_ART17_COMPLETION_CERT.md`.

## 6. Post-incident
- If cascade missed a table, file a P1 bug + update
  `scripts/m29/verify_erasure_complete.sql` to include it.
- Add a CI test that introspects schema + asserts cascade completeness.
