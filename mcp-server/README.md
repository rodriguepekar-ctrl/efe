# EFE MCP Server (M16)

Exposes the EFE 1.0 GA classification API as **Model Context Protocol** tools so any MCP-compatible LLM client (Claude Desktop, Cursor, Zed, Continue, Cline, …) can call EFE without a custom plugin.

> **Compliance posture preserved** — the LLM is the user's choice, in their own MCP client. The MCP server itself adds **zero new sub-processors** to the EFE deployment. See `docs/EFE_MCP_SERVER_EXTENSION_ARCHITECTURE.md` for the full design.

## Tools exposed

| Tool                  | Description                                                                                       | Write? |
| --------------------- | ------------------------------------------------------------------------------------------------- | :----: |
| `efe_classify`        | Classify a text against a catalog. Returns top-k classes with cosine similarity scores.           |   No   |
| `efe_register_catalog`| Create a new catalog (or new version) from a list of classes + example sentences.                 |  Yes\* |
| `efe_evolve_catalog`  | Add example sentences to existing classes (or new classes) of an existing catalog.                |  Yes\* |
| `efe_list_catalogs`   | List catalogs visible to the caller's tenant.                                                     |   No   |
| `efe_get_audit_log`   | Return the last N audit-log entries for the caller's tenant.                                      |   No   |
| `efe_get_admin_quotas`| Return the current quota usage (D12) for the caller's tenant.                                     |   No   |

\* Write tools require an explicit `confirm: true` argument from the LLM client (M16 confirm-required pattern, mirrors STRIDE TM-M16-T-01).

## Quick start (Claude Desktop)

```bash
pip install efe-mcp-server  # or: pip install -e mcp-server/  from this repo
```

Then add to `~/Library/Application Support/Claude/claude_desktop_config.json` :

```json
{
  "mcpServers": {
    "efe": {
      "command": "efe-mcp-server",
      "env": {
        "EFE_OIDC_TOKEN": "eyJhbGciOi...",
        "EFE_ENDPOINT":   "https://efe.example.com"
      }
    }
  }
}
```

Restart Claude Desktop. The 6 EFE tools will appear in the tool palette.

## Quick start (offline against playground)

The MCP server can talk to the M15 playground REST backend with **no extra config** — perfect for trying it before pointing at production.

```bash
# Terminal 1 : start the playground
bash playground/scripts/quickstart.sh
# → http://localhost:8765

# Terminal 2 : grab a demo token
TOKEN=$(curl -s -X POST http://localhost:8765/auth/token \
  -H 'content-type: application/json' \
  -d '{"tenant_id":"acme","role":"admin"}' | python -c 'import json,sys; print(json.load(sys.stdin)["token"])')

# Terminal 3 : run the MCP server pointed at the playground
EFE_OIDC_TOKEN="$TOKEN" \
EFE_ENDPOINT="http://localhost:8765" \
efe-mcp-server
```

## Configuration (env vars)

| Var                          | Required | Default | Description                                                              |
| ---------------------------- | :------: | ------- | ------------------------------------------------------------------------ |
| `EFE_OIDC_TOKEN`             |    yes   | —       | Bearer token (D15 OIDC). Tenant\_id is derived from the JWT — never from arguments. |
| `EFE_ENDPOINT`               |    yes   | —       | Backend URL. `http(s)://…` → REST/gRPC ; `unix:///…` → local socket (M19). |
| `EFE_MCP_TRANSPORT`          |    no    | `stdio` | `stdio` (default, for desktop clients) or `http` (HTTP+SSE on 8800).     |
| `EFE_MCP_RATE_LIMIT_PER_MIN` |    no    | `100`   | Per-(token, tool) rate limit, requests per minute.                       |
| `EFE_MCP_AUDIT_EMIT`         |    no    | `true`  | Emit a D14-bis audit event per tool call. Set `false` to disable.        |
| `EFE_MCP_LOG_LEVEL`          |    no    | `INFO`  | `DEBUG` / `INFO` / `WARNING` / `ERROR`.                                  |

## Development

```bash
pip install -e ".[test]"
pytest                    # 40 pin tests
pytest --cov=mcp_server   # with coverage
```

## Architecture

See [`docs/EFE_MCP_SERVER_EXTENSION_ARCHITECTURE.md`](../docs/EFE_MCP_SERVER_EXTENSION_ARCHITECTURE.md).
