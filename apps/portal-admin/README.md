# EFE Portal Admin

Tenant DPO/admin web UI for the EFE classification platform (Sprint GLOSS-C).

Built with Next.js 14 (App Router + RSC), tRPC v11, NextAuth v5, Tailwind CSS, shadcn/ui.

## Local development

```bash
pnpm install
cp .env.local.example .env.local
# Edit .env.local with your IdP + backend URLs.
pnpm dev
# → http://localhost:3000
```

## Tests

```bash
pnpm test:unit          # Vitest unit tests (~110)
pnpm test:e2e           # Playwright E2E (~145 specs × 3 browsers)
pnpm typecheck          # TypeScript strict
pnpm lint               # Biome lint
```

## Build

```bash
pnpm build              # Next.js standalone output
docker build -t portal-admin:dev .
```

Image budget : **≤ 200 MB** (distroless + standalone). Verified by CI.

## Deployment

The portal is deployed via the `efe-stack` Helm chart sub-chart `portal-admin`.

```bash
helm upgrade efe-stack ./charts/efe-stack \
  --set portal-admin.enabled=true \
  --set portal-admin.image.tag=1.0.0 \
  --reuse-values
```

Cf. `docs/EFE_GLOSS_SPRINT_C_ARCHITECTURE.md` § 3 for full details.

## Pages

| Path | Persona | Purpose |
|------|---------|---------|
| `/login` | all | OIDC SSO redirect |
| `/dashboard` | all | KPIs RPS / latency / cost / reviews |
| `/audit` | all | Search + paginate audit log |
| `/audit/[id]` | all | Single audit detail + concept_stack viewer (Mode-3 GDPR Art. 22) |
| `/reviews` | all | Pending Mode-3 reviews list |
| `/reviews/[token]` | DPO | Approve / reject review |
| `/compliance` | DPO | AI Act Art. 14 + GDPR Art. 22 attestations |
| `/keys` | Admin | API keys CRUD |
| `/quotas` | Admin | Quotas view + edit |
| `/settings` | all (read) | Tenant settings (residency, BYOK, DPO contact) |

## Environment variables

See `.env.local.example` for the complete list. Production values are injected
via Kubernetes Secrets through the Helm chart.

## Related docs

- Architecture : `docs/EFE_GLOSS_SPRINT_C_ARCHITECTURE.md`
- Helm chart : `charts/efe-stack/charts/portal-admin/`
- Backend tools : `mcp-server/mcp_server/tools/m23_*.py` (Sprint A D8)
