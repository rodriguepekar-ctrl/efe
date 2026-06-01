import { auth } from "@/lib/auth/config";
import { createMcpClient } from "@/lib/mcp/client";
import { isValidTenantSlug } from "@/lib/utils/tenant";
/**
 * RSC-friendly direct tRPC caller.
 *
 * Allows Server Components to invoke tRPC procedures without going
 * through HTTP (no fetch overhead). Builds context from headers and
 * cookies of the current request via Next.js `headers()` / `cookies()`.
 */
import { cookies, headers } from "next/headers";

const ACTIVE_TENANT_COOKIE = "efe_active_tenant";

async function buildRscContext() {
  const session = await auth();
  const cookieStore = cookies();
  const activeCookie = cookieStore.get(ACTIVE_TENANT_COOKIE)?.value;
  const tenants = session?.user.tenants ?? [];
  const tenant = activeCookie && tenants.includes(activeCookie) ? activeCookie : (tenants[0] ?? "");
  return {
    user: session?.user
      ? {
          email: session.user.email,
          tenants: session.user.tenants ?? [],
          roles: session.user.roles ?? [],
        }
      : null,
    tenant,
    mcp: createMcpClient(tenant, session?.accessToken),
  };
}

export async function fetchTenantHealth() {
  const ctx = await buildRscContext();
  const raw = await ctx.mcp.callTool("efe_get_tenant_health", {
    tenant_id: ctx.tenant,
  });
  return raw as any;
}

export async function fetchAuditList(args: {
  cursor?: string;
  mode?: string;
  since?: string;
  limit?: number;
}) {
  const ctx = await buildRscContext();
  return ctx.mcp.callTool("efe_get_audit_log", {
    tenant_id: ctx.tenant,
    ...args,
  }) as any;
}

export async function fetchAuditDetail(auditId: number) {
  const ctx = await buildRscContext();
  const result = (await ctx.mcp.callTool("efe_get_audit_log", {
    tenant_id: ctx.tenant,
    audit_id_min: auditId,
    audit_id_max: auditId,
    limit: 1,
    include_concept_stack: true,
  })) as { rows?: any[] };
  return result.rows?.[0] ?? null;
}

export async function fetchPendingReviews() {
  const ctx = await buildRscContext();
  return ctx.mcp.callTool("efe_get_mode3_reviews", {
    tenant_id: ctx.tenant,
    status: "pending",
    limit: 100,
  }) as any;
}

export async function fetchReviewDetail(token: string) {
  const ctx = await buildRscContext();
  if (!isValidTenantSlug(ctx.tenant)) throw new Error("invalid tenant in context");
  const stack = await ctx.mcp.callTool("efe_get_concept_stack", { review_token: token });
  const summary = (await ctx.mcp.callTool("efe_get_mode3_reviews", {
    tenant_id: ctx.tenant,
    review_token: token,
    limit: 1,
  })) as { reviews?: any[] };
  const review = summary.reviews?.[0];
  if (!review) throw new Error(`review ${token} not found`);
  return { ...review, concept_stack: stack };
}

export async function fetchComplianceStatus() {
  const ctx = await buildRscContext();
  return ctx.mcp.callTool("efe_verify_compliance", { tenant_id: ctx.tenant }) as any;
}

export async function fetchKeysList() {
  const ctx = await buildRscContext();
  return ctx.mcp.callTool("efe_list_api_keys", {
    tenant_id: ctx.tenant,
    include_revoked: false,
  }) as any;
}

export async function fetchQuotas() {
  const ctx = await buildRscContext();
  return ctx.mcp.callTool("efe_get_admin_quotas", { tenant_id: ctx.tenant }) as any;
}
