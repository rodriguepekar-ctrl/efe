// Auth session helpers — server-side OIDC + bearer access for the
// portal-admin (Next.js App Router).
//
// SCAFFOLDING : real implementation lives in M19 (auth middleware).
// This stub exposes the surface that M23 server actions depend on so
// the curation pages compile + run in dev. Replace with the production
// auth integration before tenant rollout.

import { cookies, headers } from "next/headers";

export type Role =
  | "viewer"
  | "curation_proposer"
  | "curation_reviewer"
  | "curation_promoter"
  | "taxonomy_admin"
  | "tenant_admin"
  | "admin";

export type Session = {
  tenantSlug: string;
  oidcSub: string;
  role: Role;
  bearer: string;
  iat: number;          // unix seconds — used for freshness checks
  idTokenHash: string;  // sha256 hex of the raw id_token, persisted with reviews
};

/// Return the current session, or null if not authenticated.
///
/// In dev mode (no IdP wired), reads from `EFE_DEV_SESSION` env var.
/// In production, decodes the bearer cookie and validates against the
/// IdP introspection endpoint.
export async function getCurrentSession(): Promise<Session | null> {
  const cookieStore = cookies();
  const bearer = cookieStore.get("efe_bearer")?.value;
  if (!bearer) {
    // Dev fallback only.
    if (process.env.NODE_ENV !== "production" && process.env.EFE_DEV_SESSION) {
      return JSON.parse(process.env.EFE_DEV_SESSION) as Session;
    }
    return null;
  }
  const claims = await decodeAndValidateBearer(bearer);
  if (!claims) return null;
  return {
    tenantSlug: claims.tenant ?? "",
    oidcSub: claims.sub,
    role: (claims["x-efe-role"] as Role) ?? "viewer",
    bearer,
    iat: claims.iat,
    idTokenHash: claims["x-id-token-hash"] ?? "",
  };
}

/// Refresh the OIDC ID token via prompt=none. Throws if the user is
/// signed out (the caller — e.g. ApprovalDialog — surfaces a
/// re-authentication prompt).
export async function refreshOidcToken(): Promise<Session> {
  const current = await getCurrentSession();
  if (!current) {
    throw new Error("session expired ; please sign in again");
  }
  // Real impl : call /api/auth/refresh which talks to the IdP.
  // For now, return current (reads from cookie which is set by
  // middleware to the latest token).
  return current;
}

// ─── Internal helpers ──────────────────────────────────────────────

type RawClaims = {
  sub: string;
  iat: number;
  tenant?: string;
  "x-efe-role"?: string;
  "x-id-token-hash"?: string;
};

async function decodeAndValidateBearer(_bearer: string): Promise<RawClaims | null> {
  // In production this calls the IdP introspection endpoint or
  // verifies a signed JWT against the JWKS. In dev/stub it returns null.
  if (process.env.NODE_ENV !== "production" && process.env.EFE_DEV_SESSION) {
    return JSON.parse(process.env.EFE_DEV_SESSION) as RawClaims;
  }
  return null;
}
