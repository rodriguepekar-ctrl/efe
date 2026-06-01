import { z } from "zod";
import { authedProcedure, dpoProcedure, router } from "../server";

const ComplianceStatus = z.object({
  tenant_id: z.string(),
  ai_act_art14: z.object({
    acknowledged: z.boolean(),
    acknowledged_at: z.string().nullable().optional(),
    signed_by: z.string().nullable().optional(),
    expires_in_days: z.number().nullable().optional(),
  }),
  gdpr_art22: z.object({
    acknowledged: z.boolean(),
    acknowledged_at: z.string().nullable().optional(),
    signed_by: z.string().nullable().optional(),
    expires_in_days: z.number().nullable().optional(),
  }),
  soc2_in_scope: z.boolean(),
});

const AttestInput = z
  .object({
    ai_act_art14: z.boolean().default(false),
    gdpr_art22: z.boolean().default(false),
    signed_by: z.string().email(),
    dpo_contact: z.string().email().optional(),
  })
  .refine((d) => d.ai_act_art14 || d.gdpr_art22, {
    message: "At least one attestation must be selected",
    path: ["ai_act_art14"],
  })
  .refine((d) => !d.gdpr_art22 || !!d.dpo_contact, {
    message: "DPO contact required when GDPR Art. 22 is acknowledged",
    path: ["dpo_contact"],
  });

export const complianceRouter = router({
  verify: authedProcedure.query(async ({ ctx }) => {
    const raw = await ctx.mcp.callTool("efe_verify_compliance", {
      tenant_id: ctx.tenant,
    });
    return ComplianceStatus.parse(raw);
  }),

  attest: dpoProcedure.input(AttestInput).mutation(async ({ ctx, input }) => {
    return ctx.mcp.callTool("efe_attest_compliance", {
      tenant_id: ctx.tenant,
      ...input,
      confirm: true,
    });
  }),
});
