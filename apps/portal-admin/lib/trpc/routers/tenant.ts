import { z } from "zod";
import { authedProcedure, router } from "../server";

const TenantHealthOutput = z.object({
  tenant_id: z.string(),
  status: z.enum(["healthy", "degraded", "down", "suspended", "unknown"]),
  checks: z.object({
    db: z.object({ status: z.string(), details: z.string().nullable().optional() }),
    redis: z.object({ status: z.string(), details: z.string().nullable().optional() }),
    llm_breaker: z.object({ status: z.string(), details: z.string().nullable().optional() }),
    audit_chain: z.object({ status: z.string(), details: z.string().nullable().optional() }),
  }),
  metrics: z.object({
    rps_last_5m: z.number().nullable().optional(),
    p99_ms_last_5m: z.number().nullable().optional(),
    error_rate_last_5m: z.number().nullable().optional(),
  }),
  compliance: z.object({
    ai_act_art14_acknowledged: z.boolean(),
    gdpr_art22_acknowledged: z.boolean(),
    expires_in_days: z.number().nullable().optional(),
  }),
  pending_reviews: z.number(),
  cost_today_usd: z.number().nullable().optional(),
  daily_cost_cap_usd: z.number().nullable().optional(),
  last_classify_at: z.string().nullable().optional(),
});

export const tenantRouter = router({
  health: authedProcedure.query(async ({ ctx }) => {
    const raw = await ctx.mcp.callTool("efe_get_tenant_health", {
      tenant_id: ctx.tenant,
    });
    return TenantHealthOutput.parse(raw);
  }),

  get: authedProcedure.query(async ({ ctx }) => {
    return ctx.mcp.callTool("efe_get_tenant", { tenant_id: ctx.tenant });
  }),

  list: authedProcedure.query(async ({ ctx }) => {
    return ctx.mcp.callTool("efe_list_tenants", {});
  }),
});
