# Runbook — Streaming RPC round emission regression

> **Symptôme** : un client gRPC streaming Mode-1 ou Mode-2 reçoit
> seulement 1-2 chunks (Final ± Error) au lieu des N RoundUpdate
> attendus. Le dashboard `efe_streaming_chunks_total` montre une
> chute brutale du `payload_kind="RoundUpdate"`.
>
> **Doctrine violée** : D-STREAM-1 (= wire-progressive opposable).

## Détection

* Alert Prometheus : `rate(efe_streaming_chunks_total{payload_kind="RoundUpdate"}[5m]) == 0`
  alors que `rate(efe_request_completion_total{rpc="ExecuteStreaming"}) > 0`.
* CI bench `stream_round_emission_bench.py --mode synth_pass` retourne `measured_pass_count = 0`.

## Causes probables

1. **Backend `execute_streaming` non câblé au `RoundEmitter`** — le code
   appelle `backend.execute(...)` (unaire) au lieu de
   `backend.execute_streaming(..., &emitter)`.
2. **Default impl du trait `StreamingExecuteBackend` utilisée** — la
   default impl émet 1 round synthétique. Une override est requise.
3. **`emitter.is_cancelled()` retourne `true` trop tôt** — `tx.is_closed()`
   probe trop agressivement (= avant le premier round).
4. **Refactor du `SwarmController::execute_streaming`** a perdu la boucle
   d'émission.

## Triage rapide

```bash
# 1. Vérifier les chunks émis sur un stream récent (= traces OTel).
otelcli query -s streaming.ExecuteStreaming -f "chunk.payload_kind"

# 2. Confirmer le bench échoue localement.
python3 eval/scripts/stream_round_emission_bench.py --mode synth_pass

# 3. Lire les logs serveur pour `efe.swarm` round count.
kubectl logs -n efe efe-server-XXX | grep "round_idx="
```

## Mitigation immédiate

1. **Si trait default impl utilisée** : vérifier que la prod backend impl
   `StreamingExecuteBackend` avec sa propre `execute_streaming` qui appelle
   `controller.execute_streaming(req, emitter).await`.
2. **Si refactor SwarmController** : `git revert` du commit fautif ; la
   boucle `for round_idx in 1..=max_rounds { ... emitter.emit(...) ... }`
   doit être préservée.
3. **Si probe cancel trop tôt** : retirer le `tx.is_closed()` du milieu
   du round computation ; le probe est uniquement entre rounds.

## Mitigation durable

* Pin test `test_stream_round_emission_synth_pass_emits_n_rounds` doit
  être renforcé à exiger `n_rounds_emitted >= max(1, requested_max_rounds - 1)`.
* Ajouter un test d'intégration end-to-end (= realism) dans
  `crates/efe-server/tests/streaming_realism.rs` qui invoque le vrai
  binaire serveur et vérifie via tonic client le nombre de chunks.

## Cross-references

* Architecture : § 5 (Mode-1) + § 6 (Mode-2)
* SLO : `SLO-STREAM-MODE1-ROUNDS` + `SLO-STREAM-MODE2-ROUNDS`
* Bench : `eval/scripts/stream_round_emission_bench.py`
* Pin tests : `test_stream_round_emission_synth_pass_emits_n_rounds`,
  `test_stream_round_emission_synth_fail_detects_regression`
