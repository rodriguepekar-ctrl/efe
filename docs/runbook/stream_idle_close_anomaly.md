# Runbook — Streaming idle close anomaly

> **Symptôme** : un client `ListAuditEvents` en tail mode (until_unix_ms=0)
> ne reçoit jamais de chunks pendant > 30s et le serveur ne ferme PAS le
> stream. OU inverse : le serveur ferme prématurément (< 5s) alors que
> le client lit normalement.
>
> **Doctrine violée** : D-STREAM-3 (= cancellation détectable) +
> indirectement D-STREAM-10 (= closure trace opposable).

## Détection

* Alert Prometheus : `histogram_quantile(0.99, rate(efe_streaming_total_duration_seconds_bucket[1h])) > 3600`
  pour le tail mode (= stream qui ne ferme jamais).
* Inversement : `efe_streaming_close_total{reason="idle_timeout"}` qui
  fire toutes les 5s (= probe trop agressif).

## Causes probables

1. **`tokio::select! { tx.closed() => ... }` ne fire pas** — bug dans
   le drop semantics du `Sender` (= upstream tonic / hyper).
2. **`PgListener::recv()` block forever** — pas de timeout, et la
   connexion TCP PG est intacte.
3. **Probe interval trop court** — `tokio::sleep(Duration::from_secs(N))`
   avec N < 30 → fermeture prématurée.
4. **TCP keepalive PG côté serveur mal configuré** — la connexion
   listener est "morte" sans que le serveur le sache.
5. **Reverse-proxy (Envoy/Nginx) avec idle timeout court** — TCP RST
   client-side après 60s sans data mais le serveur croit le client vivant.

## Triage rapide

```bash
# 1. Vérifier les flags idle.
curl http://efe-server:9090/metrics | grep efe_audit_tail_keepalive_total
# Sain : 1 incr toutes les 30s par stream actif.

# 2. Tester la fermeture en local.
python3 eval/scripts/stream_idle_bench.py --mode synth_pass

# 3. Vérifier les TCP keepalive PG.
psql efe -c "SHOW tcp_keepalives_idle;"
# Recommandé : 30s
```

## Mitigation immédiate

1. **Stream qui ne ferme jamais** :
   - Forcer la fermeture côté reverse-proxy : Envoy `stream_idle_timeout: 35s`.
   - Restart du pod si OOM imminent.

2. **Fermeture trop agressive** :
   - Augmenter `tokio::sleep(Duration::from_secs(30))` côté server.
   - Vérifier que `tokio::select!` n'a pas un `_ = sleep_short =>` accidentel.

## Mitigation durable

* Pin test `test_stream_idle_below_30s` + nightly bench.
* Configuration explicite par tenant : `idle_timeout_ms` dans le request
  proto v2 (futur sprint).
* Métric dédiée : `efe_audit_tail_keepalive_total{tenant}` pour observer
  les probes par tenant.
* TCP keepalive Linux : `net.ipv4.tcp_keepalive_time = 60` au K8s level.

## Cross-references

* Architecture : § 10.3 (Idle timeout)
* SLO : `SLO-STREAM-IDLE-MS`
* Bench : `eval/scripts/stream_idle_bench.py`
* DT-STREAM-IDLE-PROD (= reconciliation prod)
