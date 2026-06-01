/** Tenant slug validation — mirrors the backend pattern. */

const SLUG_RE = /^[a-z0-9][a-z0-9-]{1,30}[a-z0-9]$/;

export function isValidTenantSlug(s: string): boolean {
  return typeof s === "string" && SLUG_RE.test(s);
}

export function assertValidTenantSlug(s: string): asserts s is string {
  if (!isValidTenantSlug(s)) {
    throw new Error(`Invalid tenant slug "${s}" — must match [a-z0-9][a-z0-9-]{1,30}[a-z0-9]`);
  }
}
