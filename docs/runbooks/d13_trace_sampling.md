# d13_trace_sampling — Runbook : no spans being received

> **Severity** : warning.
> **Alert** : `D13ObsTraceSamplingMisconfigured`.
> **Owner** : SRE.

## 1. Alert summary

No spans received by the OTel collector / Tempo despite
active traffic. Either :
- OTel SDK isn't initialised on the server.
- Sample rate is 0.
- OTLP endpoint is unreachable from the pod.

## 2. First 5 minutes

1. Check server config :
   ```bash
   kubectl exec deploy/efe-server -- env | grep EFE_OTEL
   ```
   Expect `EFE_OTEL_ENABLED=true` and a valid endpoint.

2. Check connectivity from the pod :
   ```bash
   kubectl exec deploy/efe-server -- nc -zv tempo-distributor 4317
   ```

3. Check Tempo distributor logs :
   ```bash
   kubectl logs deploy/tempo-distributor | tail -100
   ```

## 3. Common root causes

| Cause | Symptom | Fix |
|-------|---------|-----|
| OTel SDK off | env var missing | Set `observability.otel.enabled=true` in values |
| Wrong endpoint | DNS / TCP fail | Fix `observability.tempo.endpoint` |
| Sample rate = 0 | Spans created but not exported | Set `observability.otel.sampleRate=0.05` |
| Network policy blocking | Connection refused | Add NetworkPolicy egress |

## 4. Mitigation

- Helm upgrade with corrected values.
- If urgent, force-sample on errors only :
  `observability.otel.forceSampleOnError=true`.

## 5. Resolution

- Add an integration test : assert non-zero spans on smoke
  traffic.

## 6. Escalation

If OTel SDK init fails on every pod : page platform team.
