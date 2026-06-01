import { expect, test } from "@playwright/test";

test.describe("Health endpoints", () => {
  test("/api/healthz returns 200 ok", async ({ request }) => {
    const res = await request.get("/api/healthz");
    expect(res.status()).toBe(200);
    expect(await res.text()).toBe("ok");
  });

  test("/api/readyz returns json with checks", async ({ request }) => {
    const res = await request.get("/api/readyz");
    expect([200, 503]).toContain(res.status());
    const body = await res.json();
    expect(body).toHaveProperty("checks");
    expect(body).toHaveProperty("ready");
  });
});
