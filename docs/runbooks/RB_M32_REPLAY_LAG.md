# RB_M32_REPLAY_LAG — NATS replay tail not catching up

**Severity** : P2 (index drift). P1 if tail > 24 h.
**Trigger** : NATS subject lag metric > 6 h.

## 1. Diagnose
```bash
# NATS subject lag
nats stream info VECTOR-EVENTS | grep -E "Messages|Last seq"

# Compare to in-memory index cardinality
kubectl -n efe-system exec deploy/efe-stack-server -- \
    curl -s http://localhost:8080/v1/vector/collections/<name>/status
```

## 2. Common causes

### 2.1 Replayer pod blocked
- The replay Job is stuck on a slow document (large payload).
- Mitigation : kill the Job + restart.

### 2.2 NATS consumer offset corruption
- The replayer's durable consumer offset is past EOL.
- Mitigation : delete + recreate the durable consumer.

### 2.3 Subject misnamed
- Recent topology change renamed `vector.upsert` ⇒ replayer
  hits empty stream.
- Mitigation : verify `efe-server` and replayer agree on the
  subject name.

## 3. Customer impact
- Inserts processed but not yet visible in the search index
  (eventually consistent). Visible by comparing `vector_doc`
  count vs `efe_vector_collection_size` gauge.

## 4. Mitigation
1. Manually run replayer for the affected collection :
   ```bash
   COLLECTION_ID=<id> scripts/m32/replay_runner.sh
   ```
2. Confirm cardinality matches PG mirror.

## 5. Post-incident
- File P2 ticket.
- If recurrent : evaluate increasing replayer concurrency.
