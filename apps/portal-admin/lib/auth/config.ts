/**
 * NextAuth v5 (Auth.js) configuration.
 *
 * Single OIDC provider — issuer + client id/secret injected via env.
 * Custom claims `tenants` + `roles` are propagated from the IdP into
 * the session JWT (signed with NEXTAUTH_SECRET).
 *
 * Cf. `docs/EFE_GLOSS_SPRINT_C_ARCHITECTURE.md` § 3.5.
 */
import NextAuth, { type DefaultSession } from "next-auth";

interface EfeProfile {
  sub: string;
  email?: string;
  email_verified?: boolean;
  preferred_username?: string;
  /** Custom claim — list of tenant slugs the user belongs to. */
  tenants?: string[];
  /** Custom claim — RBAC roles : "dpo" | "admin" | "auditor". */
  roles?: string[];
}

declare module "next-auth" {
  interface Session {
    user: {
      tenants: string[];
      roles: string[];
    } & DefaultSession["user"];
    accessToken?: string;
    expires: string;
  }
}

// JWT augmentation is omitted intentionally — the JWT type from
// `@auth/core/jwt` is not always resolvable from a Next.js project's
// node_modules layout (depends on hoisting). Token fields are accessed
// with casts within callbacks, which is type-safe at the boundary.

import type { NextAuthConfig } from "next-auth";

/**
 * Build NextAuth config. Lazy-loaded so module imports stay safe in
 * environments without `OIDC_*` env vars (e.g. unit tests).
 */
function buildConfig(): NextAuthConfig {
  return {
    providers: [
      {
        id: "oidc",
        name: "OIDC",
        type: "oidc",
        issuer: process.env.OIDC_ISSUER,
        clientId: process.env.OIDC_CLIENT_ID,
        clientSecret: process.env.OIDC_CLIENT_SECRET,
        authorization: {
          params: {
            scope: "openid email profile efe:tenants",
          },
        },
        idToken: true,
        // Mutable array (NextAuth v5 requires `("pkce" | "state" | ...)[]`).
        checks: ["pkce", "state"],
      },
    ],
    session: {
      strategy: "jwt",
      maxAge: 60 * 60 * 8, // 8 hours
    },
    callbacks: {
      async jwt({ token, profile, account }) {
        if (profile) {
          const p = profile as EfeProfile;
          token.tenants = p.tenants ?? [];
          token.roles = p.roles ?? [];
        }
        if (account?.access_token) {
          token.accessToken = account.access_token as string;
        }
        return token;
      },
      async session({ session, token }) {
        session.user.tenants = (token.tenants as string[]) ?? [];
        session.user.roles = (token.roles as string[]) ?? [];
        session.accessToken = token.accessToken as string | undefined;
        return session;
      },
    },
    pages: {
      signIn: "/login",
      error: "/error",
    },
    cookies: {
      sessionToken: {
        name: "__Secure-portal-session",
        options: {
          httpOnly: true,
          sameSite: "strict",
          secure: true,
          path: "/",
        },
      },
    },
    trustHost: true,
  };
}

export const authConfig = buildConfig();

export const { auth, handlers, signIn, signOut } = NextAuth(authConfig);
// Re-export for convenience in route handlers.
export const { GET, POST } = handlers;
