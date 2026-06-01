# `efe-admin repl` — Letter B reference manual

> **Sprint D19 — DT-B-5** : this document is THE source of truth
> for the REPL command set. The pin test
> `scripts/tests/test_d19_dt_b5_help_doc_parity` extracts every
> `:cmd` mentioned here AND every `:cmd` registered in the live
> REPL (`registered_commands()`), and asserts that the two sets
> are **identical**. Doc drift = CI fail.

---

## 1. Overview

The REPL is a long-running process that lets an operator
interactively classify text, tune a catalog, inspect the server
state, and toggle diagnostics. Boot once, talk for hours.

**Boot** :

```bash
$ efe-admin repl --catalog ./catalogs/support_triage.yaml
EFE REPL v0.1.10 — type :help for commands, :quit to exit.
efe>
```

## 2. Command set (canonical)

| Command | Synopsis | Purpose |
|---------|----------|---------|
| `:help` | `:help [topic]` | Show this command list (or a specific topic). |
| `:quit` | `:quit` | Exit the REPL cleanly. |
| `:exit` | `:exit` | Alias for `:quit`. |
| `:catalog` | `:catalog PATH` | Load a new catalog at runtime. |
| `:reload` | `:reload` | Reload the current catalog from disk. |
| `:classify` | `:classify TEXT` | Classify TEXT against the loaded catalog. |
| `:last` | `:last` | Re-print the last classify verdict. |
| `:budget` | `:budget MS` | Set the per-request budget in milliseconds. |
| `:timeout` | `:timeout SECS` | Set the gRPC deadline in seconds. |
| `:strict` | `:strict {on\|off}` | Toggle strict mode (rejects → exit non-zero). |
| `:tenant` | `:tenant TENANT_ID` | Switch the active tenant. |
| `:endpoint` | `:endpoint HOST:PORT` | Reconfigure the gRPC endpoint. |
| `:auth` | `:auth FILE` | Reload the JWT bearer token from FILE. |
| `:log` | `:log {error\|warn\|info\|debug}` | Set the log level for stderr emission. |
| `:quiet` | `:quiet` | Shortcut for `:log error`. |
| `:verbose` | `:verbose` | Shortcut for `:log debug` (deprecated). |
| `:debug` | `:debug {on\|off}` | Toggle structured debug events on stderr. |
| `:diagnostics` | `:diagnostics {on\|off}` | Include server diagnostics in classify output. |
| `:format` | `:format {json\|text\|table}` | Switch the stdout format. |
| `:pretty` | `:pretty {on\|off}` | Toggle JSON pretty-printing. |
| `:history` | `:history [N]` | Show last N history entries (default 20). |
| `:save-history` | `:save-history` | Flush history to disk now (auto on quit). |
| `:network` | `:network {grpc\|native}` | Switch between gRPC + PyO3 native (D25). |
| `:tuning` | `:tuning {summary\|reset}` | Show / reset runtime tuning state. |
| `:session` | `:session {info\|reset}` | Show / reset session state. |
| `:output` | `:output PATH` | Tee subsequent stdout to PATH. |

## 3. Multi-line input

Use a trailing `\` to continue on the next line :

```text
efe> :classify Customer claims charge for $99 on \
... 2024-01-15 was unauthorised; wants refund.
{"label": "fraud", ...}
efe>
```

## 4. Sessions and state

The REPL maintains a `SessionState` per process. Mutable state :

* current catalog (loaded YAML)
* current tenant id + endpoint + auth token
* current budget / timeout / strict flag
* current log level + format / pretty toggles
* current network backend (`grpc` default, `native` opt-in via D25)
* last classify result (`:last`)

State is NOT persisted across processes. Boot a new REPL = fresh
session.

## 5. Secret redaction

> **Sprint D19 — DT-B-4** — the REPL automatically redacts
> sensitive tokens from `:debug` logs before they hit stderr.
> See `efe_tool/repl/redaction.py`.

Patterns redacted (deny-list) :

| Name | Example match | Replacement |
|------|---------------|-------------|
| `jwt` | `eyJhbGciOiJIUzI1...` | `<REDACTED-JWT>` |
| `bearer` | `Bearer abc123...` | `Bearer <REDACTED>` |
| `aws_access_key` | `AKIA0123456789ABCDEF` | `<REDACTED-AWS-KEY>` |
| `aws_secret_kv` | `aws_secret_access_key=wJa...` | `aws_secret_access_key=<REDACTED-AWS-SECRET>` |
| `openai_key` | `sk-ant-api03-...` | `<REDACTED-API-KEY>` |
| `stripe_key` | `sk_live_4eC39...` | `<REDACTED-STRIPE-KEY>` |
| `github_pat` | `ghp_AAAA0000...` | `<REDACTED-GITHUB-PAT>` |
| `gcp_private_key` | `"private_key":"-----BEGIN..."` | `"private_key":"<REDACTED-GCP-PRIVATE-KEY>"` |
| `x_api_key_header` | `X-API-Key: abc123...` | `X-API-Key: <REDACTED-HEADER>` |
| `env_kv_secret` | `MY_TOKEN=abc123...` | `MY_TOKEN=<REDACTED>` |

Operators MUST NOT paste secrets in the REPL. The redaction is
defence-in-depth — it catches the cases where an operator does
anyway. Pin tested.

## 6. History

* Default location : `$XDG_DATA_HOME/efe/history` (Linux/macOS) or
  `%APPDATA%/efe/history` (Windows via `pyreadline3`).
* Override : `--history PATH` flag at boot, or `EFE_HISTORY_PATH`
  env var.
* Capacity : 10 000 entries (FIFO rotation).
* Format : one line per entry, UTF-8, no timestamps.

## 7. Windows support

> **Sprint D19 — DT-B-1** — the REPL works on Windows via
> `pyreadline3`. Install via :
>
> ```bash
> pip install efe-tool[repl-win]
> ```

Without the extra, the REPL falls back to a degraded line-mode
input (no completion, no history rotation). The CLI emits a
`UserWarning` at boot to surface the degradation.

## 8. Testing the REPL

The REPL is pin-tested via `pexpect` (Linux/macOS) — see
`clients/python/tests/repl/test_pexpect_*.py` for the canonical
signal-handling tests :

* Ctrl-C during `:classify` returns to prompt cleanly.
* SIGTERM with a dirty session flushes history before exit.
* Exception in `:catalog PATH` (corrupted YAML) does NOT crash
  the session ; the operator sees a clear error + stays at prompt.
* Double Ctrl-C exits with code 130 (`InterruptedError`).
* EOF (Ctrl-D / `:quit`) exits with code 0.

## 9. References

* [`LETTER_B_ARCHITECTURE.md`](../LETTER_B_ARCHITECTURE.md) — full
  spec.
* [`efe_tool/repl/`](../clients/python/efe_tool/repl/) — source.
* [`efe_tool/repl/redaction.py`](../clients/python/efe_tool/repl/redaction.py)
  — DT-B-4 redaction module.
* [`scripts/tests/test_d19_dt_b5_help_doc_parity.py`](../scripts/tests/test_d19_dt_b5_help_doc_parity.py)
  — this doc ↔ live REPL parity test.

## 10. Pin tests guarding this document

| Test | Invariant |
|------|-----------|
| `test_d19_dt_b5_help_doc_parity` | Every `:cmd` documented = every `:cmd` registered. |
| `test_d19_dt_b1_pyreadline3_extra_declared` | `pyproject.toml` declares the `repl-win` extra. |
| `test_d19_dt_b3_history_capped` | History writer enforces the documented cap. |
| `test_d19_dt_b4_redaction_in_debug_logs` | Every pattern listed in §5 actually redacts. |
| `test_d19_dt_b2_pexpect_signals_5_cases` | The 5 signal scenarios in §8 pass on Linux/macOS. |
