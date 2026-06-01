import {
  RbacError,
  belongsToTenant,
  hasAllRoles,
  hasAnyRole,
  requireAuth,
  requireRole,
} from "@/lib/auth/rbac";
import { describe, expect, it } from "vitest";

const dpoUser = { email: "d@a.com", tenants: ["acme"], roles: ["dpo"] };
const adminUser = { email: "a@a.com", tenants: ["acme"], roles: ["admin"] };
const cross = { email: "x@b.com", tenants: ["beta"], roles: ["dpo"] };

describe("rbac.hasAnyRole", () => {
  it("returns false for null user", () => {
    expect(hasAnyRole(null, ["dpo"])).toBe(false);
  });
  it("returns true when role matches", () => {
    expect(hasAnyRole(dpoUser, ["dpo"])).toBe(true);
  });
  it("returns false when no role matches", () => {
    expect(hasAnyRole(dpoUser, ["admin"])).toBe(false);
  });
});

describe("rbac.hasAllRoles", () => {
  it("returns true only when ALL required roles are present", () => {
    const u = { email: "x", tenants: [], roles: ["dpo", "admin"] };
    expect(hasAllRoles(u, ["dpo", "admin"])).toBe(true);
    expect(hasAllRoles(u, ["dpo", "auditor"])).toBe(false);
  });
});

describe("rbac.belongsToTenant", () => {
  it("returns false for null", () => {
    expect(belongsToTenant(null, "acme")).toBe(false);
  });
  it("returns true when tenant in user's list", () => {
    expect(belongsToTenant(dpoUser, "acme")).toBe(true);
  });
  it("returns false otherwise", () => {
    expect(belongsToTenant(dpoUser, "beta")).toBe(false);
  });
});

describe("rbac.requireAuth", () => {
  it("throws UNAUTHORIZED when user is null", () => {
    expect(() => requireAuth({ user: null, tenant: "acme" })).toThrow(RbacError);
  });
  it("does not throw with a user", () => {
    expect(() => requireAuth({ user: dpoUser, tenant: "acme" })).not.toThrow();
  });
});

describe("rbac.requireRole", () => {
  it("throws when user not in tenant", () => {
    expect(() => requireRole({ user: cross, tenant: "acme" }, ["dpo"])).toThrow(/not in tenant/);
  });
  it("throws when user lacks role", () => {
    expect(() => requireRole({ user: adminUser, tenant: "acme" }, ["dpo"])).toThrow(
      /requires one of roles/i,
    );
  });
  it("succeeds when user is in tenant + role", () => {
    expect(() => requireRole({ user: dpoUser, tenant: "acme" }, ["dpo"])).not.toThrow();
  });
});
