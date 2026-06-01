# RB_M28_WEBHOOK_DELIVERY_FAIL — Webhook delivery failure mitigation

**Severity** : P2 (rate > 5%) | P3 (circuits open).
**Trigger** : `M28WebhookDeliveryFailureRate` or `M28WebhookCircuitsOpen`.

## 1. Identify scope
```bash
psql -d efe -c "
  SELECT subscription_id, count(*) AS attempts,
         sum(case when outcome IN ('fail','drop') then 1 else 0 end) AS fails
  FROM webhook_delivery WHERE attempted_at > NOW() - INTERVAL '1 hour'
  GROUP BY subscription_id ORDER BY fails DESC LIMIT 20;"
```
Single customer vs broad ?

## 2. Single customer
- Check their endpoint manually : `curl -fsS <url>` from a delivery
  worker pod.
- If their endpoint 404s, they removed it → mark sub inactive :
  ```sql
  UPDATE webhook_subscription SET active = FALSE
   WHERE id = '<sub_id>';
  ```
- If their endpoint 500s : circuit breaker will catch ; check
  `circuit_state`. Manually close if needed :
  ```sql
  UPDATE webhook_subscription
     SET circuit_state='closed', consecutive_fails=0, circuit_opens_at=NULL
   WHERE id = '<sub_id>';
  ```

## 3. Broad failure (multiple customers)
- Likely an issue on our side. Check delivery worker logs.
- Common causes : NATS lag, DB pool saturation, network egress block.
- Roll back recent deploy if symptom started post-rollout.

## 4. Replay events lost during outage
```bash
curl -X POST https://api.efe.ai/v1/webhooks/<id>/replay \
  -H "Authorization: Bearer <token>" \
  -d '{"since": "2026-05-11T08:00:00Z", "topics": ["agent.run.completed"]}'
```

## 5. Post-incident
- File incident report.
- If recurring : raise circuit-breaker `CIRCUIT_TRIP_AT_FAILS` from
  10 → 5 (more aggressive open) ; tune retry schedule.
