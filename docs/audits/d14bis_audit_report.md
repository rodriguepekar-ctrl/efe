# Sprint D14-bis — Audit interne

> **Date** : 2026-04-27 (post-livraison D14-bis).
> **Méthodologie** : revue ligne-à-ligne des 2 569 lignes livrées par
> D14-bis (audit_hmac.rs 813 + audit_keys.rs 368 + migration 0003 148
> + CLI bin 299 + integration tests 302 + telemetry instruments 256
> + Python pin 383). Comparaison code vs
> `docs/bench/d14bis_hmac_audit_chain_architecture.md`.
> Adversarial reading axé **crypto** : timing attacks, length-extension,
> replay, genesis bypass, chain integrity races.
> **Pattern référence** : audits D8 (8 bugs, 2 critiques),
> D12-bis (7 bugs, 2 critiques), D14 (7 bugs, 1 critique).

---

## 1. Bilan

| # | Sévérité | Domaine | Symptôme | Statut |
|--:|:--------:|---------|----------|:------:|
| **C1** | 🔴 CRIT | Runtime/shutdown | `runtime.rs` spawnait une boucle inline `loop { ticker.tick(); self_verify_pass(...) }` SANS shutdown ; la fonction proprement écrite `periodic_self_verify(pool, keys, interval, shutdown)` (avec `shutdown.notified()`) était **dead code**. Au shutdown, la boucle continuait de tourner contre un pool en cours de fermeture → erreurs sqlx + log noise + tâche fuit. | ✅ Fixé |
| **C2** | 🔴 CRIT | Async lifecycle | `runtime.spawn(audit)` détaché, perdu au shutdown. Aucune `JoinSet` ni await-all-on-shutdown. Mutations committed pouvaient perdre leur audit → trou dans la chaîne, exact opposé de "tamper-evident". | ✅ Fixé |
| **C3** | 🔴 CRIT | Threat model | Empty chain attack (DBA + `DROP TRIGGER` + `TRUNCATE` + recreate) : verify dit "intact" pour table vide. Mitigation D15 (checkpoint externe) documentée mais non livrée. | ⏸ DT-D14bis.1 (D15 scope) |
| **M1** | 🟠 MED | Crypto timing | `verify_chain` faisait `hex_lower(&computed) != stored_hmac` (string compare non constant-time) → timing-side-channel theorical leak des préfixes HMAC (faible risque, verify offline). | ✅ Fixé |
| **M2** | 🟠 MED | Robustness | `now_unix_ms()` retournait `0` silencieusement si clock skew avant epoch → audit log avec timestamp suspect, opérateur aveugle. | ✅ Fixé |
| **M3** | 🟠 MED | Cleanup | `#[async_trait]` macro inutile sur `impl AuditSink for HmacChainedAuditSink` (méthode `emit` sync). Code mort. | ✅ Fixé |
| **M4** | 🟠 MED | Sémantique fan-out | `TracingAuditSink.emit` toujours, même si SQL fail → Loki voit l'event que SQL n'a pas. Comportement intentionnel mais non documenté. | ✅ Fixé (doc + ordre changé) |
| **M5** | 🟠 MED | UX CLI | Verify d'une chaîne empty (jamais utilisée) dit "intact" — signal mensonger pour CLI/régulateur qui s'attend à un état cohérent. | ⏸ DT-D14bis.2 (UX, non bloquant) |
| **L1** | 🟡 LOW | Loki ordering | `TracingAuditSink.emit` était DANS le spawn (post-SQL) au lieu de pré-spawn → Loki delay et possibles décorrélations sous load. | ✅ Fixé |

**Méthodologie audit** : revue ligne-à-ligne avec accent particulier
sur :

* **Crypto correctness** : RFC 2104 HMAC, length-extension resistance
  (HMAC garantit par construction), determinism canonical_payload,
  replay protection (`event_id UUID v7 UNIQUE`), key rotation
  multi-key verify.
* **Concurrency** : advisory lock per-chain, cache pollution under
  rollback, spawn ordering vs shutdown, JoinSet semantics.
* **DDL ↔ code** : CHECK constraints alignés avec validation Rust,
  trigger semantics (UPDATE/DELETE/TRUNCATE coverage).
* **Helm/config** : env var rendering conditional, Secret volume
  mount paths.

L'audit D14-bis a confirmé que les 10 décisions architecturales du
document `d14bis_hmac_audit_chain_architecture.md` sont
correctement implémentées. Les 3 bugs critiques trouvés sont tous
des **bugs d'intégration** (entre l'archi et le runtime) plutôt que
des bugs de design crypto.

---

## 2. Détail des fixes

### 2.1 Fix C1+C2 — Cycle de vie shutdown du sink HMAC

**Symptôme exact** : deux problèmes liés.

1. **C1 — Inline loop sans shutdown** : `runtime.rs::build` faisait
   ```rust
   tokio::spawn(async move {
       let mut ticker = tokio::time::interval(...);
       ticker.tick().await;
       loop {
           ticker.tick().await;
           if let Err(e) = self_verify_pass(...).await { tracing::error!(...) }
       }
   });
   ```
   Pas de `shutdown.notified()`. Quand le shutdown signal arrive,
   la boucle continue de tourner contre un pool qui se ferme → erreurs
   sqlx + leak.

2. **C2 — Spawn detach sans drain** : `HmacChainedAuditSink::emit()`
   spawnait via `tokio::spawn(...)` détaché. À shutdown, les tâches
   en cours sont aborted ; les mutations committed mais audit-pending
   perdent leur trace.

**Fix livré** :

* **`audit_hmac.rs`** :
  - `Inner` ajoute un `pending: Mutex<JoinSet<()>>` (audit fix C2).
  - `HmacChainedAuditSink::drain_pending(timeout)` : nouvelle méthode
    publique qui await tous les sign-tasks en cours, avec budget
    bounded ; au-delà du budget, abort + bump
    `efe_audit_sign_failures_total{error="drain_abort"}`.
  - `emit()` : utilise `try_lock` sur le JoinSet ; en cas de
    contention (drain en cours), fallback détaché + bump
    `audit_sign_failures_total{error="drain_race"}`.

* **`runtime.rs`** :
  - `build()` ne spawn PLUS de boucle inline. Il stocke un
    `AuditSigningHandles { sink, self_verify_every_s }` dans le
    `Server` struct.
  - `serve(addr, shutdown)` :
    1. Spawn `crate::catalog::audit_hmac::periodic_self_verify(pool, keys, interval, shutdown.clone())` — la fonction qui était dead code et qui a `shutdown.notified()` natif.
    2. Après le serve future résout, appelle
       `handles.sink.drain_pending(force_abort_after / 2)` AVANT de
       retourner.

**Tests ajoutés** :

* `test_runtime_rs_spawns_periodic_self_verify` (Python pin) :
  vérifie que le runtime utilise bien `periodic_self_verify` et
  pas un inline loop.
* `test_runtime_rs_drains_pending_audit_at_shutdown` (Python pin) :
  vérifie la présence de `drain_pending` dans `serve`.
* `test_audit_hmac_rs_drain_pending_is_public` (Python pin).

### 2.2 Fix M1 — Constant-time hex compare

**Symptôme** : `if hex_lower(&computed) != stored_hmac { ... }`
court-circuite au premier byte différent. Un attaquant qui mesure
la durée de verify sur des HMAC adverses peut récupérer ~quelques
bits par mesure (timing-side-channel classique).

**Fix livré** :

* Nouveau helper public `ct_hex_eq(a: &str, b: &str) -> bool` dans
  `audit_hmac.rs`. Algo standard :
  ```rust
  if a.len() != b.len() { return false; }
  let mut diff: u8 = 0;
  for (x, y) in a.bytes().zip(b.bytes()) { diff |= x ^ y; }
  diff == 0
  ```
  Le XOR-OR ne short-circuit pas — durée constante peu importe la
  position du premier diff.
* `verify_chain` utilise `ct_hex_eq` pour la chain link check ET
  la HMAC compare (2 sites, ~3 occurrences avec le helper appelé
  dans les tests).

**Tests ajoutés** :

* `ct_hex_eq_matches_string_eq_on_valid_inputs` : sanity equality.
* `ct_hex_eq_does_not_short_circuit_on_first_diff` : property
  test sur diff au dernier byte.
* `test_audit_hmac_rs_uses_constant_time_compare` (Python pin) :
  ≥ 3 occurrences de `ct_hex_eq(` (définition + appels).

### 2.3 Fix M2 — Clock skew warning

**Symptôme** : `SystemTime::now().duration_since(UNIX_EPOCH)
.unwrap_or(0)` masquait silencieusement un clock avant 1970.

**Fix livré** :

```rust
match SystemTime::now().duration_since(UNIX_EPOCH) {
    Ok(d) => d.as_millis() as i64,
    Err(e) => {
        tracing::warn!(
            target: "efe.audit.hmac",
            err = %e,
            "wall-clock is BEFORE UNIX_EPOCH ; audit timestamp set to 0 — \
             fix the system clock (e.g. NTP, CMOS battery) immediately"
        );
        0
    }
}
```

Pin test Python : `test_audit_hmac_rs_clock_skew_warns_explicitly`.

### 2.4 Fix M3 — `#[async_trait]` cleanup

**Symptôme** : `impl AuditSink for HmacChainedAuditSink` avait
`#[async_trait]` mais la méthode `emit` est sync. Macro inerte
sur sync méthodes ; juste du bruit.

**Fix** : import `async_trait` retiré de `audit_hmac.rs` ;
attribut `#[async_trait]` retiré de l'`impl`. Build clean.

### 2.5 Fix M4+L1 — Sémantique fan-out + Loki ordering

**Symptômes liés** :

* **M4** : la TracingAuditSink fan-out était DANS le spawn, après
  le check `if let Err(...)`. Si SQL fail, Loki recevait quand
  même l'event — comportement intentionnel mais non documenté
  (le code semblait suggérer une exclusion mutuelle).

* **L1** : la TracingAuditSink fan-out était APRÈS la SQL
  persist, donc les events Loki arrivaient dans l'ordre des
  spawn-completions, pas l'ordre des registry-emit.

**Fix livré** :

```rust
fn emit(&self, event: &AuditEvent) {
    // L1 fix : fan-out FIRST so Loki ordering matches registry-emit.
    if self.inner.chain_to_tracing {
        super::audit::TracingAuditSink.emit(event);
    }
    // ... spawn the SQL persist ...
}
```

* La doc `emit()` est étendue avec une section
  **"Fan-out semantics (audit fix M4)"** explicitant que Loki
  voit toujours l'event, SQL ajoute la chaîne crypto par-dessus.

Pin test Python : `test_audit_hmac_rs_emit_fanouts_to_tracing_first`.

---

## 3. Items non-fixés (documentés)

### 3.1 C3 — Empty chain DBA attack (DT-D14bis.1)

**Symptôme** : un DBA qui a accès à `DROP TRIGGER audit_events_no_*`
peut TRUNCATE puis recreate les triggers ; la chaîne entière
disparaît, le verify dit "intact" car aucune row à vérifier.

**Pourquoi non fixé en D14-bis** : la mitigation est
**checkpointing externe** — on émet périodiquement le `chain_head_seq`
vers un sink hors-DB (e.g. signed log file, S3 + Object Lock,
external transparency log) ; à verify on compare le checkpoint
externe avec la table. Si l'externe a un seq=N et la table dit
seq=0, on sait que tout a disparu.

**Coût** : une chaîne d'archival externe + un nouveau verify path
qui croise les deux sources. Sprint dédié = D15.

**Décision** : tracker comme **DT-D14bis.1** dans le sprint
closure D14-bis ; visibility ops via `pgaudit` extension externe
(out-of-EFE) qui audit les DDL.

### 3.2 M5 — UX verify chaîne empty (DT-D14bis.2)

**Symptôme** : `efe-verify-audit --chain tenant:nonexistent` dit
"intact" pour une chaîne qui n'existe pas. UX confondante.

**Fix possible** : si `n_events = 0`, distinguer "chain not found"
vs "chain intact". Le rapport JSON pourrait inclure un `state:
"empty" | "intact" | "broken"`.

**Décision** : tracker comme **DT-D14bis.2** ; non bloquant pour
v0.1.14.5.

---

## 4. Anti-régression

### 4.1 Vérifications post-fix

```bash
# Lib (sink + helpers + verify + new ct_hex_eq) — 289 tests verts
cargo test -p efe-server --lib

# D14-bis integration (gated EFE_PG_TEST_URL) — 8 tests verts
cargo test -p efe-server --test d14bis_audit_hmac_integration

# Cross-sprint regression
cargo test -p efe-server --test d12bis_catalog_integration \
    --test d14_postgres_catalog_integration --test integration_grpc \
    --test shutdown_behavior --test startup_sequence
# 28 + 22 + 20 + 6 + 8 = 84 tests verts

# Python full suite — 592 verts
PYTHONPATH=clients/python python3 -m pytest scripts/tests/ -q
```

**Total** : 289 + 8 + 84 + 592 = **973 tests verts**.

### 4.2 Tests ajoutés en D14-bis-audit

| Fichier | Avant | Après | Δ |
|---------|------:|------:|---:|
| `audit_hmac.rs::tests` | 9 | 11 | +2 (`ct_hex_eq` 2 tests) |
| `scripts/tests/test_d14bis_audit_hmac.py` | 34 | 39 | +5 (constant-time, drain_pending public, periodic_self_verify wired, drain_pending in serve, clock skew warn) |
| **Total D14-bis** | 43 | 50 | **+7** |

---

## 5. Sign-off audit D14-bis

| Vérification | Résultat |
|--------------|:--------:|
| Architecture vs livraison cohérente | ✅ post-fix |
| HMAC RFC 2104 + SHA-256 | ✅ |
| Canonical payload byte-stable (BTreeMap) | ✅ |
| Replay protection (UUID v7 UNIQUE) | ✅ |
| Append-only triggers (UPDATE/DELETE/TRUNCATE) | ✅ |
| Constant-time HMAC compare | ✅ post-fix M1 |
| Clock skew explicit warn | ✅ post-fix M2 |
| Periodic self-verify with shutdown | ✅ post-fix C1 |
| Drain pending sign tasks at shutdown | ✅ post-fix C2 |
| Loki fan-out ordering preserved | ✅ post-fix L1 |
| Fan-out semantics documented | ✅ post-fix M4 |
| Async-trait cleanup | ✅ post-fix M3 |
| Empty chain DBA attack | ⏸ DT-D14bis.1 (D15) |
| Verify CLI UX for empty chain | ⏸ DT-D14bis.2 (non bloquant) |
| Helm + ConfigMap + Deployment guards | ✅ |
| 11 env vars + validate gating | ✅ |
| 6 D14-bis instruments + 2 spans | ✅ |
| 39 Python pin tests | ✅ |
| 8 Rust integration tests gated | ✅ |
| Cross-sprint regression (D8/D11/D12/D12-bis/D13/D14) | ✅ 0 régression |

**Audit D14-bis fermé** — 9 bugs trouvés (3 CRIT, 5 MED, 1 LOW),
7 fixés, 2 documentés comme dette technique (D15 scope ou non bloquant).

---

## 6. Annexe — Decisions de fixes

| Décision | Alternatives écartées |
|----------|----------------------|
| C1 : utiliser `periodic_self_verify` du module audit_hmac | rester avec inline loop dans runtime (pas de shutdown handling propre) |
| C2 : `JoinSet` + `drain_pending` au shutdown | spawn detach simple (perd events au shutdown) |
| C2 : `try_lock` + fallback detached spawn dans emit | bloquer l'emit (sync trait method peut pas await async lock) |
| M1 : `ct_hex_eq` hand-rolled XOR-OR | dep `subtle` crate (overkill pour 64-byte hex) |
| M2 : `tracing::warn!` + return 0 | panic (catastrophique pour un audit sink) ou silent default (masking the problem) |
| M3 : retirer async_trait | garder (warning unused import) |
| M4 : doc explicite + ordre fan-out modifié | inverser ne pas fan-out si SQL fail (perd compat Loki) |
| L1 : fan-out PRE-spawn | post-spawn (Loki delay non-déterministe) |
| C3 : DT-D14bis.1 D15 scope | fix D14-bis (sprint dédié à external checkpointing) |
| M5 : DT-D14bis.2 non bloquant | fix D14-bis (UX seulement, pas correctness) |
