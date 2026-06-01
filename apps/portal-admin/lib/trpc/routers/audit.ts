import { z } from "zod";
import { authedProcedure, router } from "../server";

const AuditQueryInput = z.object({
  cursor: z.string().optional(),
  mode: z.enum(["MODE_2", "MODE_AUTO_ESC", "MODE_3"]).optional(),
  since: z.string().datetime().optional(),
  limit: z.number().int().min(1).max(1000).default(50),
});

const AuditRow = z.object({
  audit_id: z.number().int(),
  ts: z.string(),
  tenant_id: z.string(),
  mode: z.string(),
  sentence_hash: z.string(),
  label: z.string().nullable().optional(),
  confidence: z.number().nullable().optional(),
  duration_ms: z.number().int().nonnegative(),
  converged: z.boolean().nullable().optional(),
});

export const auditRouter = router({
  query: authedProcedure.input(AuditQueryInput).query(async ({ ctx, input }) => {
    const result = (await ctx.mcp.callTool("efe_get_audit_log", {
      tenant_id: ctx.tenant,
      ...input,
    })) as { rows?: unknown[]; next_cursor?: string };
    const rows = z.array(AuditRow).parse(result.rows ?? []);
    return { rows, nextCursor: result.next_cursor ?? null };
  }),

  get: authedProcedure
    .input(z.object({ auditId: z.number().int() }))
    .query(async ({ ctx, input }) => {
      // The mcp-server doesn't have a single-row tool ; we fetch with
      // tight cursor and pick by id. In Sprint D a dedicated tool
      // `efe_get_audit_row` would be added.
      const result = (await ctx.mcp.callTool("efe_get_audit_log", {
        tenant_id: ctx.tenant,
        audit_id_min: input.auditId,
        audit_id_max: input.auditId,
        limit: 1,
        include_concept_stack: true,
      })) as { rows?: any[] };
      const row = result.rows?.[0];
      if (!row) {
        throw new Error(`audit_id ${input.auditId} not found`);
      }
      return row;
    }),

  exportPdf: authedProcedure
    .input(z.object({ auditId: z.number().int() }))
    .mutation(async ({ ctx }) => {
      // Defer to the dedicated export tool. The audit_id is read by the
      // /api/audit/[auditId]/export.pdf route from URL params, not from
      // the tRPC input — this mutation is a no-op trigger that lets the
      // mutation pipeline (audit log + RBAC) run.
      return ctx.mcp.callTool("efe_export_audit_log", {
        tenant_id: ctx.tenant,
        since: new Date(Date.now() - 86400_000).toISOString(),
        max_rows: 1,
        format: "jsonl",
      });
    }),
});
