# vsock-relayer

EFE M26 vsock-relayer — a privileged DaemonSet that bridges in-microVM HTTP
calls to in-cluster services (M21 LLM router + mcp-server) with mTLS,
JWT auth, per-microVM rate-limiting, and audit relay.

## Architecture

```
agent-core (microVM) ─vsock─► vsock-relayer (host pod) ─mTLS─► LLM router
                                                       └──────► mcp-server
```

See `docs/EFE_M26_AGENT_RUNTIME_PRODUCTION_ARCHITECTURE.md` § 10.

## Build

```bash
docker build -f cmd/vsock-relayer/Dockerfile -t ghcr.io/efe-ai/vsock-relayer:2.1.0 .
```

## Env vars

| Var | Default | Purpose |
|---|---|---|
| `EFE_RELAY_VSOCK_PORT` | 9999 | vsock listener port |
| `EFE_LLM_ROUTER_URL` | https://efe-llm-router.efe-system.svc:50051 | LLM router upstream |
| `EFE_MCP_SERVER_URL` | https://efe-mcp-server.efe-system.svc:8080 | mcp-server upstream |
| `EFE_LLM_CLIENT_TLS_DIR` | /var/run/tls/llm-client | mTLS client cert + key |
| `EFE_MCP_CLIENT_TLS_DIR` | /var/run/tls/mcp-client | mTLS client cert + key |
| `EFE_RELAY_CA_BUNDLE` | /etc/ssl/certs/efe-ca-bundle.pem | CA bundle for upstream verify |
| `EFE_RELAY_JWT_PUBLIC_KEY_PATH` | /var/run/relay-jwt/pub.pem | JWT validation public key |
| `EFE_RELAY_AUDIT_ENDPOINT` | http://efe-mcp-server.efe-system.svc:8080/v1/audit/append | audit relay target |
| `EFE_RELAY_HEALTH_ADDR` | 0.0.0.0:8080 | TCP health server addr |
