import { expect, test } from "@playwright/test";

test.describe("Login page", () => {
  test("renders the SSO sign-in form", async ({ page }) => {
    await page.goto("/login");
    await expect(page.getByRole("heading", { name: /EFE Portal Admin/i })).toBeVisible();
    await expect(page.getByRole("button", { name: /sign in with sso/i })).toBeVisible();
  });

  test("redirects unauthenticated users to /login from /dashboard", async ({ page }) => {
    await page.goto("/dashboard");
    await expect(page).toHaveURL(/\/login/);
  });

  test("preserves the next= param", async ({ page }) => {
    await page.goto("/audit");
    await expect(page).toHaveURL(/next=%2Faudit/);
  });
});
