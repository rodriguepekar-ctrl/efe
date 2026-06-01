// Auth guards used by server components for tenant-feature + RBAC checks.
//
// SCAFFOLDING : real implementation looks up tenant features in the EFE
// tenant_feature table via the backend client. This stub returns a
// permissive value in dev and a denial in production unless wired to
// the backend.

import { redirect } from "next/navigation";
import { getCurrentSession, type Role, type Session } from "@/lib/auth/session";

/// Server-side guard : ensure a tenant has the named feature flag ON.
/// In production, this calls the backend to read `tenant_feature.<flag>`.
/// On failure, redirects to a "feature not enabled" page.
export async function requireFeature(
  tenantSlug: string,
  flag: string,
): Promise<true> {
  const enabled = await isFeatureEnabled(tenantSlug, flag);
  if (!enabled) {
    redirect(`/${tenantSlug}/features/disabled?flag=${encodeURIComponent(flag)}`);
  }
  return true;
}

/// Server-side guard : ensure the caller has at least one of the given roles.
/// On failure, redirects to /forbidden. Returns the resolved Session on success.
export async function requireRole(
  tenantSlug: string,
  allowed: Role[],
): Promise<Session> {
  const session = await getCurrentSession();
  if (!session) {
    redirect(`/${tenantSlug}/login?next=${encodeURIComponent("/curation")}`);
  }
  if (session.tenantSlug !== tenantSlug) {
    redirect("/forbidden?reason=tenant_mismatch");
  }
  if (!allowed.includes(session.role)) {
    redirect(`/forbidden?role=${session.role}`);
  }
  return session;
}

// ─── Internals ─────────────────────────────────────────────────────

async function isFeatureEnabled(tenantSlug: string, flag: string): Promise<boolean> {
  // Real impl : await backend.getTenantFeatures(tenantSlug)[flag]
  // Dev fallback : env var EFE_DEV_FEATURE_<FLAG>=true|false
  const envKey = `EFE_DEV_FEATURE_${flag.toUpperCase()}`;
  if (process.env.NODE_ENV !== "production") {
    return process.env[envKey] === "true";
  }
  return false;
}
