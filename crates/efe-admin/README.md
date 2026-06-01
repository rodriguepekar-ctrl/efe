# `efe-admin` — EFE Tenant Administration CLI

> **Sprint** : GLOSS A (D8) — Go-Live & Onboarding Self-Service.
> Cf. [`docs/EFE_GO_LIVE_ONBOARDING_ARCHITECTURE.md`](../../docs/EFE_GO_LIVE_ONBOARDING_ARCHITECTURE.md) § 3.

`efe-admin` is the **administrative command-line interface** for an EFE
deployment. It wraps the `mcp-server` `m23_*` tools and provides a
UX-friendly surface for Solutions Engineers (SE) to provision tenants,
seed catalogs, attest compliance, and run post-deploy smoke tests —
without touching SQL or gRPC by hand.

The binary is statically compiled (musl on Linux, native on macOS /
Windows), ~5 MB, and runs in **≤ 50 ms cold** for `--help`. Every write
operation requires either `--yes` or an interactive TTY confirmation.

---

## Table of contents

- [Installation](#installation)
- [Quick start (30-minute POC)](#quick-start-30-minute-poc)
- [Command reference](#command-reference)
- [Configuration](#configuration)
- [Authentication modes](#authentication-modes)
- [Output formats](#output-formats)
- [Exit codes](#exit-codes)
- [Shell completion](#shell-completion)
- [Troubleshooting](#troubleshooting)
- [Sprint provenance](#sprint-provenance)

---

## Installation

### Homebrew (macOS / Linux)

```bash
brew tap efe-ai/efe
brew install efe-admin
```

### Cargo (any platform with rustc ≥ 1.85)

```bash
cargo install --path crates/efe-admin
```

### GitHub Releases (pre-built binaries)

```bash
# Pick the right asset for your OS / arch
curl -L https://github.com/efe-ai/efe/releases/latest/download/efe-admin-linux-amd64 \
    -o efe-admin
chmod +x efe-admin
./efe-admin version
```

Each release is **Sigstore-signed** ; verify with `cosign verify-blob`
against `https://github.com/efe-ai/efe/.github/workflows/release-efe-admin.yml`.

### Docker

```bash
docker run --rm efe/admin:latest version
```

---

## Quick start (30-minute POC)

Once a cluster is provisioned (`terraform apply` + `helm install
efe-stack`), the SE workflow looks like this:

```bash
# 1. Point at the cluster (token-based auth)
export EFE_ADMIN_CLUSTER=acme
echo "<jwt>" > ~/.config/efe-admin/acme.token
chmod 600 ~/.config/efe-admin/acme.token

# 2. Provision the tenant
efe-admin tenant create acme \
    --plan enterprise \
    --max-rps 1000 \
    --max-tokens-per-day 1000000 \
    --data-residency EU_ONLY \
    --contact-email ops@acme.com \
    --yes

# 3. Seed the first catalog
efe-admin catalog seed acme \
    --bundle ./tenants/acme/support-triage.jsonl \
    --bundle-version 1.0.0 \
    --catalog default \
    --yes

# 4. Attest compliance (AI Act Art. 14 + GDPR Art. 22)
efe-admin compliance attest acme \
    --ai-act-art14 \
    --gdpr-art22 \
    --signed-by dpo@acme.com \
    --dpo-contact dpo@acme.com \
    --yes

# 5. Issue an API key (30-day expiry, classify scope)
efe-admin keys create acme --scope classify --expires-in 30d

# 6. Run the post-deploy smoke test
efe-admin smoke-test acme

# 7. Confirm health
efe-admin health
```

End-to-end target : **≤ 30 minutes** from a fresh laptop on AWS,
**≤ 45 minutes** on Azure or GCP (cf. R-07 in architecture § 14).

---

## Command reference

`efe-admin` exposes **24 commands** organised into 13 top-level groups.
Every write command requires `--yes` or an interactive `[y/N]` prompt.

### Tenant management

| Command | Purpose | Required scope |
|---|---|---|
| `efe-admin tenant create <name>` | Provision a new tenant | `efe:admin:tenants:write` |
| `efe-admin tenant list` | List tenants (`--status` filter) | `efe:admin:read` |
| `efe-admin tenant get <name>` | Show tenant details | `efe:admin:read` |
| `efe-admin tenant update <name>` | Update quotas / residency / contact | `efe:admin:tenants:write` |
| `efe-admin tenant suspend <name>` | Set max_rps=0 (reversible) | `efe:admin:tenants:write` |
| `efe-admin tenant delete <name>` | Irreversible delete with audit purge schedule | `efe:admin:tenants:write` |

#### Slug validation

Tenant names must match `^[a-z0-9-]{3,32}$` (no leading or trailing
hyphen, no uppercase). Bad slugs return exit 64 with a typed error.

#### Plans

`trial` (default, max_rps=100), `growth`, `enterprise`.

#### Data residency

`EU_ONLY` (aliases : `eu`, `eu_only`), `US_ONLY` (`us`, `us_only`),
`any` (no constraint). Residency is enforced at the storage layer
(Postgres replica + Redis region pinning).

### Catalog management

| Command | Purpose |
|---|---|
| `efe-admin catalog seed <tenant> --bundle FILE.jsonl --bundle-version V` | Seed anchors from a JSONL bundle |
| `efe-admin catalog list <tenant>` | List catalogs |
| `efe-admin catalog promote <tenant> --catalog ID --to-version V` | Promote a catalog version |

Bundles must be JSONL (one anchor per line) with the schema documented
in [`docs/EFE_GO_LIVE_ONBOARDING_ARCHITECTURE.md`](../../docs/EFE_GO_LIVE_ONBOARDING_ARCHITECTURE.md) § 10. Comment lines (`# …`) and blank
lines are skipped. Bad JSON or empty `text` fields fail eagerly.

### Compliance

| Command | Purpose |
|---|---|
| `efe-admin compliance attest <tenant> --ai-act-art14 --gdpr-art22 --signed-by EMAIL --dpo-contact EMAIL` | Record the AI Act Art. 14 + GDPR Art. 22 attestation |
| `efe-admin compliance verify <tenant>` | Verify current attestation state |

`--dpo-contact` is **mandatory** whenever `--gdpr-art22` is set —
the CLI rejects the call locally before any backend roundtrip
(see `crates/efe-admin/src/commands/compliance.rs`).

### API keys

| Command | Purpose |
|---|---|
| `efe-admin keys create <tenant> --scope SCOPE [--expires-in DURATION]` | Issue a new API key |
| `efe-admin keys rotate <tenant> --key-id ID` | Rotate (issue replacement, old stays valid 1 h grace) |
| `efe-admin keys revoke <tenant> --key-id ID --reason TEXT` | Revoke immediately |
| `efe-admin keys list <tenant> [--all]` | List active keys (`--all` for revoked + expired) |

`--scope` ∈ `read_only | classify | admin`. Default = `classify`.
`--expires-in` accepts humantime durations : `30d`, `90d`, `1y`,
or the literal `never` for non-expiring service-account keys.

### Quotas

| Command | Purpose |
|---|---|
| `efe-admin quotas get <tenant>` | Show current quotas |
| `efe-admin quotas set <tenant> [--max-rps N] [--max-tokens-per-day N]` | Update quotas (atomic) |

### Audit

| Command | Purpose |
|---|---|
| `efe-admin audit query <tenant> [--since DATE] [--mode MODE] [--limit N]` | Query the audit log |
| `efe-admin audit export <tenant> --since DATE --to FILE.jsonl` | Export to JSONL for DPO archival |

`--mode` ∈ `MODE_2 | MODE_AUTO_ESC | MODE_3`. The audit log is HMAC-
chained (cf. migration `0013_audit_chain_hmac.sql`) ; the CLI displays
each event's `chain_position` so DPOs can spot gaps.

### Smoke-test, health, version

| Command | Purpose |
|---|---|
| `efe-admin smoke-test <tenant> [--sentences FILE.jsonl]` | End-to-end classify smoke against the deployment |
| `efe-admin health [--cluster URL]` | gRPC health snapshot |
| `efe-admin version` | Print version + connectivity info |

The default smoke-test fixture ships embedded — no external file is
required. The fixture covers `classify` with 12 sentences and asserts
p99 latency ≤ 5 s + accuracy ≥ 0.80 against the canary catalog.

### Login (experimental, OIDC device flow)

| Command | Purpose |
|---|---|
| `efe-admin login [--idp-url URL] [--client-id ID]` | Run the OAuth2 device flow against your IdP (Keycloak / Okta / Azure AD) |

Compiled behind the `sso` cargo feature. Off in v1.0.0 — file an issue
if you need it enabled.

### Shell completion

| Command | Purpose |
|---|---|
| `efe-admin completion <SHELL>` | Generate completion script for bash / zsh / fish / pwsh / elvish |

Example :

```bash
efe-admin completion zsh > ~/.zsh/completions/_efe-admin
```

### M23 / M24 / M25 surfaces (post-GA)

`efe-admin` also exposes operator surfaces for the post-MVP milestones :

- `efe-admin curation …` — M23 LLM-assisted taxonomy curation (propose
  splits / merges, review queue, promote, drift events, cost).
- `efe-admin learning …` — M24 active learning loop (queue, retune,
  promote, GDPR Art. 17 erasure, DPO consent management).
- `efe-admin agent …` — M25 agent runtime EXPERIMENTAL (runs,
  approvals, templates, DPIA, cost reporting).

These are **off by default** and gated by feature flags on the
backend ; they ship in the same binary so that operators get one stable
CLI surface across the platform lifecycle.

---

## Configuration

`efe-admin` reads its configuration from
`~/.config/efe-admin/config.toml` by default (override with
`--config /path/to/file` or `$EFE_ADMIN_CONFIG`).

```toml
[default]
cluster        = "https://efe.example.com"
auth_method    = "token"
token_file     = "~/.config/efe-admin/token"
output_format  = "table"
confirm        = "interactive"

[clusters.acme]
cluster        = "https://efe.acme.com"
auth_method    = "token"
token_file     = "~/.config/efe-admin/acme.token"

[clusters.local]
cluster        = "http://localhost:8081"
auth_method    = "none"
```

### Precedence (highest first)

1. CLI flag (`--cluster acme`, `--format json`, …)
2. Env var (`EFE_ADMIN_CLUSTER`, `EFE_ADMIN_FORMAT`, …)
3. Config file `[clusters.<alias>]` table
4. Config file `[default]` table
5. Hard-coded defaults (`http://localhost:8081`, `table`, …)

### Environment variables

| Name | Purpose |
|---|---|
| `EFE_ADMIN_CLUSTER` | Cluster alias |
| `EFE_ADMIN_CONFIG` | Override config file path |
| `EFE_ADMIN_FORMAT` | `table | json | yaml` |
| `EFE_ADMIN_TOKEN` | Inline bearer token (debug — prefer `token_file`) |
| `EFE_ADMIN_LOG` | Tracing filter (`warn` default, `debug`, `trace`) |
| `KUBERNETES_SERVICE_ACCOUNT_TOKEN` | Auto-picked up when running in-cluster |

---

## Authentication modes

| Mode | Use case | Config |
|---|---|---|
| **`none`** | Local dev (`k3d`), CI fixtures | `auth_method = "none"` |
| **`token`** | POC + bridge to production | `auth_method = "token"` + `token_file` (0600 perm) |
| **`sso`** (`sso` cargo feature) | Production w/ Keycloak / Okta / AAD | `auth_method = "sso"` + `idp_url` + `client_id` |
| **`kube_sa`** | CI/CD inside the cluster | `auth_method = "kube_sa"` — picks up SA token automatically |

Tokens are **never** logged. If a token leaks into stderr via
`-vv`, file an issue — it is a critical bug.

### RBAC scopes

The CLI propagates the token without inspection. Scope enforcement is
done **server-side** (mcp-server + efe-server). The expected claims
are :

```text
efe:admin:tenants:write    # tenant create/delete/suspend/update
efe:admin:catalogs:write   # catalog seed/promote
efe:admin:compliance:attest# compliance attest
efe:admin:keys:write       # keys create/rotate/revoke
efe:admin:quotas:write     # quotas set
efe:admin:audit:read       # audit export
efe:admin:read             # everything else read
```

`smoke-test`, `health`, `version`, `completion` require **no scope**.

---

## Output formats

| Flag | Output |
|---|---|
| (default) | Colored `comfy-table` ASCII table |
| `--format json` | Strict JSON, jq-friendly, stable schema (`apiVersion: efe.io/v1`) |
| `--format yaml` | YAML — copy/paste friendly for Helm `values.yaml` |
| `-q, --quiet` | Minimum output, scriptable |
| `-v, --verbose` | Debug logs on stderr ; doesn't affect stdout payload |
| `-vv` | Trace logs (very chatty) |

Example :

```bash
$ efe-admin tenant list --format table
┌──────────┬─────────────┬────────────┬──────────┬──────────────────────┐
│ name     │ plan        │ status     │ max_rps  │ created_at           │
├──────────┼─────────────┼────────────┼──────────┼──────────────────────┤
│ acme     │ enterprise  │ active     │ 1000     │ 2026-04-12T10:23:11Z │
│ globex   │ trial       │ active     │ 100      │ 2026-04-30T15:01:42Z │
│ initech  │ enterprise  │ suspended  │ 0        │ 2026-03-08T09:14:00Z │
└──────────┴─────────────┴────────────┴──────────┴──────────────────────┘
3 tenants (active=2, suspended=1)
```

JSON output stays schema-stable across point releases ; breaking
changes bump the `apiVersion` field.

---

## Exit codes

| Code | Meaning | Source |
|---:|---|---|
| 0 | Success | `Ok(())` |
| 1 | User abort (pressed N at confirm) | `AdminError::UserAbort` |
| 2 | Backend / generic error | `AdminError::Backend` + catch-all |
| 3 | Auth failure (expired token, missing scope) | `AdminError::AuthFailed` |
| 64 | CLI syntax error (sysexits.h `EX_USAGE`) | bad args, validation failure |

Use `efe-admin <cmd> --format json` and check exit code to script
reliably.

---

## Shell completion

```bash
# bash
efe-admin completion bash | sudo tee /etc/bash_completion.d/efe-admin

# zsh
efe-admin completion zsh > "${fpath[1]}/_efe-admin"

# fish
efe-admin completion fish > ~/.config/fish/completions/efe-admin.fish

# PowerShell
efe-admin completion power-shell | Out-String | Invoke-Expression
```

---

## Troubleshooting

### `auth failure: invalid token`

The token at `token_file` is malformed or expired. Re-issue from the
IdP and overwrite the file with `0600` permissions.

### `backend error [confirm_required]`

A write tool was called without `confirm=true`. The CLI sets this
automatically when you pass `--yes` or confirm interactively — if you
see this, file a bug.

### `network error: connection refused`

The configured `cluster` URL is unreachable. Verify with
`efe-admin health --cluster $URL` ; if the host is local, check that
`mcp-server` is listening on the expected port (default 8081).

### `invalid input: tenant slug 'Acme!' rejects pattern …`

Tenant slugs must match `^[a-z0-9-]{3,32}$`. Lowercase, no leading or
trailing hyphen.

### Confirmation prompt is missing

The CLI auto-bypasses the prompt when (a) `--yes` is passed, (b) the
config sets `confirm = "always_yes"`, or (c) stdin is not a TTY (e.g.
piped). For unattended automation, prefer the explicit `--yes` flag —
it documents intent in the audit log.

---

## Sprint provenance

| Sprint | Scope | Doc |
|---|---|---|
| **GLOSS A (D8)** | CLI v0.1 (tenant / catalog / compliance / keys / quotas / audit / smoke / health) | [`docs/EFE_GO_LIVE_ONBOARDING_ARCHITECTURE.md`](../../docs/EFE_GO_LIVE_ONBOARDING_ARCHITECTURE.md) |
| M23 | `efe-admin curation …` subtree | [`docs/EFE_M23_TAXONOMY_CURATION_ARCHITECTURE.md`](../../docs/EFE_M23_TAXONOMY_CURATION_ARCHITECTURE.md) |
| M24 | `efe-admin learning …` subtree | [`docs/EFE_M24_ACTIVE_LEARNING_ARCHITECTURE.md`](../../docs/EFE_M24_ACTIVE_LEARNING_ARCHITECTURE.md) |
| M25 | `efe-admin agent …` subtree (EXPERIMENTAL) | [`docs/EFE_M25_AGENT_RUNTIME_ARCHITECTURE.md`](../../docs/EFE_M25_AGENT_RUNTIME_ARCHITECTURE.md) |
| Closure | GLOSS A sign-off | [`docs/audits/gloss_sprint_a_closure.md`](../../docs/audits/gloss_sprint_a_closure.md) |

---

## Development

### Build

```bash
# Native build (debug)
cargo build --package efe-admin

# Release binary
cargo build --release --package efe-admin

# Linux static (musl)
cargo build --release --target x86_64-unknown-linux-musl --package efe-admin
```

### Test

```bash
cargo test --package efe-admin
```

54 tests are currently green (39 unit + 10 CLI integration + 5
mcp roundtrip). See `tests/` and inline `#[cfg(test)] mod tests`.

### Lints

```bash
cargo clippy --package efe-admin --all-targets -- -D warnings
```

The crate is pinned to `pedantic + nursery` clippy levels with a small
allowlist (`missing_docs`, `module_name_repetitions`,
`missing_errors_doc`, `missing_panics_doc`, `must_use_candidate`) —
documented in `Cargo.toml [lints]`.

---

## License

UNLICENSED — internal EFE distribution. See
[`Cargo.toml`](Cargo.toml) workspace metadata.
