import { expect, test } from "@playwright/test";

test.describe("Security headers", () => {
  test("login page sets strict CSP + frame-ancestors none", async ({ page }) => {
    const response = await page.goto("/login");
    expect(response).not.toBeNull();
    const headers = response!.headers();
    expect(headers["content-security-policy"]).toMatch(/frame-ancestors 'none'/);
    expect(headers["x-frame-options"]).toBe("DENY");
    expect(headers["x-content-type-options"]).toBe("nosniff");
    expect(headers["referrer-policy"]).toBe("strict-origin-when-cross-origin");
    expect(headers["strict-transport-security"]).toMatch(/max-age=63072000/);
  });

  test("session cookie name is __Secure-portal-session (when set)", async ({ context, page }) => {
    await page.goto("/login");
    const cookies = await context.cookies();
    // Cookie may not be set until after auth ; just assert no insecure name leakage.
    for (const c of cookies) {
      if (c.name.toLowerCase().includes("session")) {
        expect(c.name).toMatch(/^__Secure-/);
      }
    }
  });
});
