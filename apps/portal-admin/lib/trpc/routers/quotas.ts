import { z } from "zod";
import { adminProcedure, authedProcedure, router } from "../server";

const Quotas = z.object({
  tenant_id: z.string(),
  max_rps: z.number().int().nonnegative(),
  max_tokens_per_day: z.number().int().nonnegative(),
  monthly_cap_usd: z.number().nullable().optional(),
  used_today_tokens: z.number().nullable().optional(),
  used_today_rps_p99: z.number().nullable().optional(),
});

export const quotasRouter = router({
  get: authedProcedure.query(async ({ ctx }) => {
    const raw = await ctx.mcp.callTool("efe_get_admin_quotas", {
      tenant_id: ctx.tenant,
    });
    return Quotas.parse(raw);
  }),

  set: adminProcedure
    .input(
      z
        .object({
          maxRps: z.number().int().min(0).max(100_000).optional(),
          maxTokensPerDay: z.number().int().min(0).max(1_000_000_000).optional(),
          monthlyCapUsd: z.number().min(0).max(1_000_000).optional(),
        })
        .refine(
          (d) =>
            d.maxRps !== undefined ||
            d.maxTokensPerDay !== undefined ||
            d.monthlyCapUsd !== undefined,
          {
            message: "Supply at least one quota to set",
          },
        ),
    )
    .mutation(async ({ ctx, input }) => {
      return ctx.mcp.callTool("efe_set_quotas", {
        tenant_id: ctx.tenant,
        max_rps: input.maxRps,
        max_tokens_per_day: input.maxTokensPerDay,
        monthly_cap_usd: input.monthlyCapUsd,
        confirm: true,
      });
    }),
});
