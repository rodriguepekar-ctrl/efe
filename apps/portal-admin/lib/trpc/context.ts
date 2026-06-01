/**
 * tRPC context builder.
 *
 * Builds the per-request context : auth session + active tenant +
 * mcp-server client. Each tRPC procedure receives this context and
 * can use `ctx.mcp.callTool(...)` server-side.
 */
import { auth } from "@/lib/auth/config";
import { type McpClient, createMcpClient } from "@/lib/mcp/client";
import { TRPCError } from "@trpc/server";

import type { RbacUser } from "@/lib/auth/rbac";

export interface TrpcContext {
  user: RbacUser | null;
  tenant: string;
  mcp: McpClient;
}

const ACTIVE_TENANT_COOKIE = "efe_active_tenant";
const ACTIVE_TENANT_HEADER = "x-efe-tenant";

export async function createContext(req: Request): Promise<TrpcContext> {
  const session = await auth();
  const user: RbacUser | null = session?.user
    ? {
        email: session.user.email,
        tenants: session.user.tenants ?? [],
        roles: session.user.roles ?? [],
      }
    : null;

  // Resolve active tenant : header > cookie > user's first tenant.
  const headerTenant = req.headers.get(ACTIVE_TENANT_HEADER);
  const cookieHeader = req.headers.get("cookie") ?? "";
  const cookieTenant = parseCookie(cookieHeader, ACTIVE_TENANT_COOKIE);
  const candidate = headerTenant ?? cookieTenant ?? user?.tenants[0];

  if (!candidate) {
    // Anonymous + no tenant — context is still built (for public procedures).
    return {
      user,
      tenant: "",
      mcp: createMcpClient("", session?.accessToken),
    };
  }

  // Hard cross-tenant guard.
  if (user && !user.tenants.includes(candidate)) {
    throw new TRPCError({
      code: "FORBIDDEN",
      message: `Tenant '${candidate}' is not in user's tenants list`,
    });
  }

  return {
    user,
    tenant: candidate,
    mcp: createMcpClient(candidate, session?.accessToken),
  };
}

function parseCookie(cookieHeader: string, name: string): string | undefined {
  const parts = cookieHeader.split(";");
  for (const p of parts) {
    const [k, v] = p.trim().split("=");
    if (k === name && v) return decodeURIComponent(v);
  }
  return undefined;
}
