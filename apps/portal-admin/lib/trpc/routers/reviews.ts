import { z } from "zod";
import { authedProcedure, dpoProcedure, router } from "../server";

const ReviewSummary = z.object({
  review_token: z.string().uuid(),
  tenant_id: z.string(),
  catalog: z.string(),
  bundle_version: z.string(),
  sentence_hash: z.string(),
  proposed_label: z.string(),
  proposed_confidence: z.number(),
  status: z.enum(["pending", "approved", "rejected", "expired"]),
  requested_at: z.string(),
  expires_at: z.string(),
});

export const reviewsRouter = router({
  listPending: authedProcedure.query(async ({ ctx }) => {
    const result = (await ctx.mcp.callTool("efe_get_mode3_reviews", {
      tenant_id: ctx.tenant,
      status: "pending",
      limit: 100,
    })) as { reviews?: unknown[] };
    const reviews = z.array(ReviewSummary).parse(result.reviews ?? []);
    return { reviews };
  }),

  get: authedProcedure
    .input(z.object({ token: z.string().uuid() }))
    .query(async ({ ctx, input }) => {
      // Fetch review detail (with concept_stack via separate tool).
      const stack = await ctx.mcp.callTool("efe_get_concept_stack", {
        review_token: input.token,
      });
      const summary = (await ctx.mcp.callTool("efe_get_mode3_reviews", {
        tenant_id: ctx.tenant,
        review_token: input.token,
        limit: 1,
      })) as { reviews?: any[] };
      const review = summary.reviews?.[0];
      if (!review) throw new Error(`review ${input.token} not found`);
      return { ...review, concept_stack: stack };
    }),

  approve: dpoProcedure
    .input(
      z.object({
        token: z.string().uuid(),
        finalLabel: z.string().min(1).max(100).optional(),
        notes: z.string().max(2048).optional(),
      }),
    )
    .mutation(async ({ ctx, input }) => {
      return ctx.mcp.callTool("efe_approve_mode3_review", {
        review_token: input.token,
        final_label: input.finalLabel,
        review_notes: input.notes,
        confirm: true,
      });
    }),

  reject: dpoProcedure
    .input(
      z.object({
        token: z.string().uuid(),
        finalLabel: z.string().min(1).max(100).optional(),
        reason: z.string().min(10, "Rejection reason must be at least 10 characters"),
      }),
    )
    .mutation(async ({ ctx, input }) => {
      return ctx.mcp.callTool("efe_reject_mode3_review", {
        review_token: input.token,
        reason: input.reason,
        final_label: input.finalLabel,
        confirm: true,
      });
    }),
});
