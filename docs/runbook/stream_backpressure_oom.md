# Runbook — Streaming backpressure / OOM

> **Symptôme** : la mémoire du process `efe-server` grandit linéairement
> en présence de clients streaming lents (= 1 read/s) sans converger.
> SLO `SLO-STREAM-BACKPRESSURE-RTT` rapporte > 128 MB/h growth.
>
> **Doctrine violée** : D-STREAM-5 (= bounded buffer mpsc(64)).

## Détection

* Alert Grafana : `process_resident_memory_bytes` croissante > 100 MB/h.
* `efe_streaming_open_streams{rpc=...}` reste élevé pendant heures.
* CI bench `stream_backpressure_bench.py` rapporte `growth_bytes_per_hour
  > 134217728`.

## Causes probables

1. **`mpsc::channel(N)` avec N != 64** — un refactor a élargi le buffer
   à un nombre arbitraire (= viole `SLO-STREAM-BUFFER-64`).
2. **Producer qui spawn de nouvelles tasks au lieu de `send().await`** —
   pattern `tokio::spawn(async { tx.send(...).await })` au lieu de
   `tx.send(...).await` direct → spawn unbounded.
3. **Channel jamais drainé côté receiver** — un bug dans `ReceiverStream`
   qui ne consomme pas les chunks queue'd.
4. **Slow consumer + tail mode** — `ListAuditEvents` tail mode avec un
   client à 1 Hz contre 1000 events/s de produit → buffer remplit en
   < 1 minute.
5. **`emitter.emit().await` non-blocking** — si l'`await` est shadowed
   par un `try_send()`, le backpressure naturel disparaît.

## Triage rapide

```bash
# 1. Vérifier que mpsc bound = 64 partout.
python3 eval/scripts/stream_static_lint.py
# → SLO-STREAM-BUFFER-64 doit être PASS

# 2. Identifier le RPC à fort buffer.
curl http://efe-server:9090/metrics | grep efe_streaming_open_streams

# 3. Heap profile via tokio-console.
TOKIO_CONSOLE_BIND=0.0.0.0:6669 ./efe-server &
tokio-console http://efe-server:6669
# Chercher les tasks bloquées sur `send.await`.
```

## Mitigation immédiate

1. **Throttling client externe** : ajouter un rate-limit Envoy au niveau
   ingress pour drop les clients qui ne lisent pas (= TCP RST si pas
   d'ACK pendant N secondes).
2. **Forcer la fermeture** : KILL les streams avec `efe_streaming_open_streams_age_seconds > 3600`.
3. **Si OOM imminent** : `kill -SIGTERM` du pod ; orchestration K8s
   redémarrera avec mémoire fraiche.

## Mitigation durable

* Pin test static `test_stream_mpsc_64_in_existing_handlers` vert sur
  chaque PR.
* Limite par-tenant : `efe_streaming_open_streams{tenant} ≤ 100`.
* Idle close opérationnel pour le tail mode (= 30 s sans event = probe + close).
* Bench `stream_backpressure_bench.py` en nightly avec 1000 events
  produits contre 1 lu.

## Cross-references

* Architecture : § 10.1 + D-STREAM-5
* SLO : `SLO-STREAM-BACKPRESSURE-RTT`, `SLO-STREAM-BUFFER-64`
* Bench : `eval/scripts/stream_backpressure_bench.py`, `stream_static_lint.py`
