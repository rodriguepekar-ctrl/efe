/**
 * RBAC helpers — pure functions for role + tenant checks.
 *
 * Used by tRPC procedure middleware AND by component conditionals to
 * grey out forbidden actions.
 */

export type EfeRole = "dpo" | "admin" | "auditor";

export interface RbacUser {
  email?: string | null;
  tenants: string[];
  roles: string[];
}

export interface RbacContext {
  user: RbacUser | null;
  tenant: string;
}

/** Caller has at least one of the required roles. */
export function hasAnyRole(user: RbacUser | null, required: readonly EfeRole[]): boolean {
  if (!user) return false;
  return user.roles.some((r) => (required as readonly string[]).includes(r));
}

/** Caller has all of the required roles. */
export function hasAllRoles(user: RbacUser | null, required: readonly EfeRole[]): boolean {
  if (!user) return false;
  return required.every((r) => user.roles.includes(r));
}

/** Caller belongs to the tenant. */
export function belongsToTenant(user: RbacUser | null, tenant: string): boolean {
  if (!user) return false;
  return user.tenants.includes(tenant);
}

/** Throw a `code` if user is not authenticated. */
export function requireAuth(ctx: RbacContext): asserts ctx is RbacContext & { user: RbacUser } {
  if (!ctx.user) {
    throw new RbacError("UNAUTHORIZED", "Authentication required");
  }
}

/** Throw if user is not in tenant OR not in role. */
export function requireRole(ctx: RbacContext, required: readonly EfeRole[]): void {
  requireAuth(ctx);
  if (!belongsToTenant(ctx.user, ctx.tenant)) {
    throw new RbacError("FORBIDDEN", `User not in tenant '${ctx.tenant}'`);
  }
  if (!hasAnyRole(ctx.user, required)) {
    throw new RbacError("FORBIDDEN", `Requires one of roles: ${required.join(", ")}`);
  }
}

export class RbacError extends Error {
  public readonly code: "UNAUTHORIZED" | "FORBIDDEN";
  constructor(code: "UNAUTHORIZED" | "FORBIDDEN", message: string) {
    super(message);
    this.name = "RbacError";
    this.code = code;
  }
}
