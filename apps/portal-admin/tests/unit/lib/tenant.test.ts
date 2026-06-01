import { assertValidTenantSlug, isValidTenantSlug } from "@/lib/utils/tenant";
import { describe, expect, it } from "vitest";

describe("isValidTenantSlug", () => {
  it("accepts valid slugs", () => {
    for (const s of ["acme", "acme-corp", "tenant-1", "ab1", "a1b2c3-x9"]) {
      expect(isValidTenantSlug(s)).toBe(true);
    }
  });

  it("rejects too short", () => {
    expect(isValidTenantSlug("ab")).toBe(false);
  });

  it("rejects uppercase", () => {
    expect(isValidTenantSlug("Acme")).toBe(false);
  });

  it("rejects leading/trailing hyphen", () => {
    expect(isValidTenantSlug("-acme")).toBe(false);
    expect(isValidTenantSlug("acme-")).toBe(false);
  });

  it("rejects too long", () => {
    expect(isValidTenantSlug("a".repeat(33))).toBe(false);
  });

  it("rejects empty", () => {
    expect(isValidTenantSlug("")).toBe(false);
  });

  it("rejects non-string input gracefully", () => {
    expect(isValidTenantSlug(undefined as unknown as string)).toBe(false);
    expect(isValidTenantSlug(null as unknown as string)).toBe(false);
  });
});

describe("assertValidTenantSlug", () => {
  it("throws on invalid", () => {
    expect(() => assertValidTenantSlug("BadCase")).toThrow();
  });
  it("does not throw on valid", () => {
    expect(() => assertValidTenantSlug("acme")).not.toThrow();
  });
});
