import { hasAnyRole } from "@/lib/auth/rbac";
/**
 * tRPC server initialization + procedure builders.
 *
 * Exposes :
 *   - `publicProcedure`     — no auth check
 *   - `authedProcedure`     — requires logged-in user
 *   - `dpoProcedure`        — requires role=dpo
 *   - `adminProcedure`      — requires role=admin
 *   - `auditedProcedure`    — wraps `authedProcedure` with action logging
 */
import { TRPCError, initTRPC } from "@trpc/server";
import superjson from "superjson";
import type { TrpcContext } from "./context";

const t = initTRPC.context<TrpcContext>().create({
  transformer: superjson,
  errorFormatter({ shape }) {
    return shape;
  },
});

export const router = t.router;
export const middleware = t.middleware;
export const publicProcedure = t.procedure;

const isAuthenticated = middleware(({ ctx, next }) => {
  if (!ctx.user) {
    throw new TRPCError({ code: "UNAUTHORIZED", message: "Sign in required" });
  }
  if (!ctx.tenant) {
    throw new TRPCError({ code: "BAD_REQUEST", message: "Active tenant missing" });
  }
  return next({ ctx: { ...ctx, user: ctx.user } });
});

const requireRole = (...required: ("dpo" | "admin" | "auditor")[]) =>
  middleware(({ ctx, next }) => {
    if (!ctx.user) {
      throw new TRPCError({ code: "UNAUTHORIZED" });
    }
    if (!hasAnyRole(ctx.user, required)) {
      throw new TRPCError({
        code: "FORBIDDEN",
        message: `Requires one of roles: ${required.join(", ")}`,
      });
    }
    return next({ ctx });
  });

export const authedProcedure = publicProcedure.use(isAuthenticated);
export const dpoProcedure = authedProcedure.use(requireRole("dpo"));
export const adminProcedure = authedProcedure.use(requireRole("admin"));

/**
 * Procedure that records the action in `ui_audit_log` (best-effort).
 * Use for any write that should be auditable for SOC 2 / GDPR Art. 30.
 */
export const auditedProcedure = (action: string) =>
  authedProcedure.use(async ({ ctx, next, getRawInput }) => {
    const result = await next({ ctx });
    // Best-effort, non-blocking.
    try {
      const input = await getRawInput();
      await ctx.mcp.callTool("efe_record_ui_audit", {
        tenant_id: ctx.tenant,
        user_email: ctx.user?.email,
        action,
        payload_jsonb: input,
      });
    } catch (e) {
      // Log to stderr ; do NOT propagate the error to caller.
      // The action itself succeeded ; logging is observability concern.
      // eslint-disable-next-line no-console
      console.error("ui_audit_log emit failed:", action, e);
    }
    return result;
  });
