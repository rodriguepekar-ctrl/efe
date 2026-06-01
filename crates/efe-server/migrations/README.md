# Sprint D14 — Catalog database migrations

This directory holds the forward-only SQL migrations applied by
`efe-server` at boot when `EFE_CATALOG_BACKEND=postgres`. The
canonical reference is `docs/bench/d14_postgres_catalog_backend_architecture.md`.

## Conventions

* **Forward-only.** No `down.sql`. If a change must be reverted,
  write a new `00NN_revert_<change>.sql` that explicitly drops or
  alters the affected objects. This avoids the well-known reversal
  hazard (the down path is rarely tested as carefully as the
  forward path).
* **Numbering.** `00NN_<verb>_<subject>.sql` ; lexicographically
  sorted. sqlx applies them in order.
* **Idempotency.** Prefer `IF NOT EXISTS` clauses for tables /
  indexes / extensions. Constraints and PKs are not idempotent at
  creation but the `_sqlx_migrations` lock guarantees exactly-once
  application.
* **Atomicity.** Each migration is wrapped in a transaction by
  sqlx ; mid-migration crash → automatic rollback.
* **No application code.** Migrations are pure DDL. They MUST NOT
  call into the EFE crate code path — that would couple deploy
  ordering to binary version.

## Application

* **At boot** by `runtime::Server::build` when
  `EFE_CATALOG_BACKEND=postgres`. Failure → fail-fast.
* **In tests** by the integration tests under
  `crates/efe-server/tests/d14_postgres_*.rs` via
  `sqlx::migrate!("../migrations").run(&pool)`.
* **Manual** by the operator via `sqlx-cli` :

  ```bash
  cargo install sqlx-cli --no-default-features --features postgres
  sqlx migrate run --source crates/efe-server/migrations \
      --database-url postgres://efe:efe@localhost:5432/efe_catalog_db
  ```

## Production playbook

1. **Dev** — write the migration, run it on the local PG.
2. **CI** — passes the migration on a freshly-spawned PG (the
   `tests/d14_postgres_*` integration suite handles this).
3. **Staging** — Helm rollout ; `efe-server` migrates at boot.
4. **Prod** — Helm rolling upgrade ; first replica to start runs
   the migration. The `_sqlx_migrations` advisory lock prevents
   double-application.

## If a migration fails in prod

* Pod fails fast → orchestrator restarts.
* If the second attempt fails : revert the binary (Helm
  rollback) ; deploy a new migration that addresses the root
  cause. **Never edit a previously-applied migration in place.**
