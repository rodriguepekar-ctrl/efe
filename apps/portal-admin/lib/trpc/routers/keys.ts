import { z } from "zod";
import { adminProcedure, authedProcedure, router } from "../server";

const KeyRow = z.object({
  key_id: z.string().uuid(),
  suffix: z.string().length(4),
  scope: z.enum(["read_only", "classify", "admin"]),
  created_at: z.string(),
  expires_at: z.string().nullable().optional(),
  revoked_at: z.string().nullable().optional(),
  last_used_at: z.string().nullable().optional(),
  use_count: z.number().int().nonnegative(),
});

export const keysRouter = router({
  list: authedProcedure
    .input(z.object({ includeRevoked: z.boolean().default(false) }))
    .query(async ({ ctx, input }) => {
      const result = (await ctx.mcp.callTool("efe_list_api_keys", {
        tenant_id: ctx.tenant,
        include_revoked: input.includeRevoked,
      })) as { keys?: unknown[] };
      return { keys: z.array(KeyRow).parse(result.keys ?? []) };
    }),

  create: adminProcedure
    .input(
      z.object({
        scope: z.enum(["read_only", "classify", "admin"]),
        expiresInDays: z.number().int().min(1).max(3650).nullable(),
      }),
    )
    .mutation(async ({ ctx, input }) => {
      const expiresAt =
        input.expiresInDays === null
          ? null
          : new Date(Date.now() + input.expiresInDays * 86400_000).toISOString();
      return ctx.mcp.callTool("efe_create_api_key", {
        tenant_id: ctx.tenant,
        scope: input.scope,
        expires_at: expiresAt,
        confirm: true,
      });
    }),

  rotate: adminProcedure
    .input(z.object({ keyId: z.string().uuid() }))
    .mutation(async ({ ctx, input }) => {
      return ctx.mcp.callTool("efe_rotate_api_key", {
        tenant_id: ctx.tenant,
        key_id: input.keyId,
        confirm: true,
      });
    }),

  revoke: adminProcedure
    .input(
      z.object({
        keyId: z.string().uuid(),
        reason: z.string().min(10, "Reason must be ≥ 10 chars"),
      }),
    )
    .mutation(async ({ ctx, input }) => {
      return ctx.mcp.callTool("efe_revoke_api_key", {
        tenant_id: ctx.tenant,
        key_id: input.keyId,
        reason: input.reason,
        confirm: true,
      });
    }),
});
