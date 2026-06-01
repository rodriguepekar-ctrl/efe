/** Server-side session helpers (RSC). */
import { auth } from "./config";
import type { RbacUser } from "./rbac";

/**
 * Returns the current session's RbacUser, or null if anonymous.
 * MUST be called in Server Components / Server Actions only.
 */
export async function getRbacUser(): Promise<RbacUser | null> {
  const session = await auth();
  if (!session?.user) return null;
  return {
    email: session.user.email,
    tenants: session.user.tenants ?? [],
    roles: session.user.roles ?? [],
  };
}

/**
 * Returns the active tenant for the current request. Reads cookie
 * `efe_active_tenant` set by the tenant-switcher component, or falls
 * back to the user's first tenant.
 */
export async function getActiveTenant(cookieValue?: string): Promise<string | null> {
  const user = await getRbacUser();
  if (!user) return null;
  if (cookieValue && user.tenants.includes(cookieValue)) {
    return cookieValue;
  }
  return user.tenants[0] ?? null;
}
