# efe-agent-runner

EFE M25 — **EXPERIMENTAL** Agent runtime Helm chart. In-cluster
Firecracker microVM sandbox for LLM-driven autonomous agents.

> **WARNING** : default OFF. Single design-partner pilot. Sunset clause
> at 12 months if < 2 active tenants. Read
> [`../../docs/EFE_M25_AGENT_RUNTIME_ARCHITECTURE.md`](../../docs/EFE_M25_AGENT_RUNTIME_ARCHITECTURE.md)
> in full before installing in production.

## TL;DR

```bash
helm repo add efe-ai https://charts.efe.example.com
helm install efe-agent-runner efe-ai/efe-agent-runner \
  --namespace efe-system \
  --create-namespace \
  --values prod-values.yaml
```

## Hard prerequisites

1. **EFE 2.0** (`efe-server` must be at 2.0.0+).
2. **M21 LLMRouter** ON, deployed and healthy.
3. **DPIA signed** for each tenant that will create AgentRuns.
4. **KVM** available on the worker nodes (`/dev/kvm` accessible).
5. **Image whitelist** populated with cosign-verified pinned digests.
6. **OIDC + roles** : the bearer-token interceptor in `efe-server`
   must be configured to mint `operator` / `admin` roles.

## Required values

| Key | Description |
|---|---|
| `agentRuntimeEnabled` | Master switch (set `true` only after DPIA) |
| `imageWhitelist` | `repo@sha256:digest,…` list (cosign-verified) |
| `agentCore.image.digest` | Pinned digest of agent-core image |
| `agentCore.cosign.keyRef` | KMS key ref for cosign verification |
| `database.existingSecret` | Secret holding DSN to Postgres |
| `audit.hmacKeySecret` | Secret holding `EFE_AUDIT_HMAC_KEY` |
| `approval.hmacKeySecret` | Secret holding `EFE_APPROVAL_HMAC_KEY` |

## Onboarding a tenant

1. Have your DPO sign the M25 addendum (see
   `docs/release/DPIA_M25_TEMPLATE.md`).
2. `efe-admin tenant enable-m25 <slug> --dpia-hash <sha256>`
3. Create at least one `AgentRunTemplate`.
4. Have an operator submit the first run (manual approval recommended
   for the first 10 runs, per the runbook).

## Sunset

The chart is wired to refuse upgrade past 12 months if `< 2` tenants
have non-zero AgentRun counts in the trailing 90 days. The hook is in
the controller — no automatic uninstall, but the alert escalates
during the steering committee review.

## See also

- Runbook : [`../../docs/runbooks/M25_AGENT_RUNTIME_RUNBOOK.md`](../../docs/runbooks/M25_AGENT_RUNTIME_RUNBOOK.md)
- DPIA template : `docs/release/DPIA_M25_TEMPLATE.md`
- Profiles : `config/agent-profiles.yaml`
