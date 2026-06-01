/**
 * Readiness probe — checks downstream dependencies are reachable
 * before accepting traffic. Reports `503` if mcp-server is unreachable.
 */
export const dynamic = "force-dynamic";

const TIMEOUT_MS = 3000;

export async function GET() {
  const checks: Record<string, "ok" | "fail" | "skipped"> = {};

  // mcp-server reachability.
  const mcpUrl = process.env.EFE_MCP_HTTP_URL;
  if (mcpUrl) {
    try {
      const ctrl = new AbortController();
      const t = setTimeout(() => ctrl.abort(), TIMEOUT_MS);
      const res = await fetch(`${mcpUrl}/healthz`, { signal: ctrl.signal });
      clearTimeout(t);
      checks.mcp_server = res.ok ? "ok" : "fail";
    } catch {
      checks.mcp_server = "fail";
    }
  } else {
    checks.mcp_server = "skipped";
  }

  const allOk = Object.values(checks).every((v) => v === "ok" || v === "skipped");
  return new Response(JSON.stringify({ checks, ready: allOk }), {
    status: allOk ? 200 : 503,
    headers: { "content-type": "application/json" },
  });
}
