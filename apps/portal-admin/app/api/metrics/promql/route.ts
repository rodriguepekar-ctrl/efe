/**
 * Prometheus PromQL proxy endpoint.
 *
 * The portal calls this from client charts ; the server-side handler
 * bridges to the Prometheus API URL configured via env. This avoids
 * leaking PROMETHEUS_URL to the browser AND lets us enforce auth +
 * tenant scoping (selector injection).
 */
import { auth } from "@/lib/auth/config";
import { cookies } from "next/headers";

export const dynamic = "force-dynamic";

const ACTIVE_TENANT_COOKIE = "efe_active_tenant";

export async function GET(request: Request) {
  const session = await auth();
  if (!session?.user) {
    return new Response("Unauthorized", { status: 401 });
  }

  const url = new URL(request.url);
  const query = url.searchParams.get("query");
  const rangeSeconds = Number.parseInt(url.searchParams.get("range_seconds") ?? "3600", 10);
  const step = Number.parseInt(url.searchParams.get("step") ?? "60", 10);

  if (!query) return new Response("Missing 'query'", { status: 400 });
  if (rangeSeconds <= 0 || rangeSeconds > 86_400)
    return new Response("Invalid 'range_seconds'", { status: 400 });
  if (step <= 0 || step > 3600) return new Response("Invalid 'step'", { status: 400 });

  const tenant = cookies().get(ACTIVE_TENANT_COOKIE)?.value ?? session.user.tenants?.[0] ?? "";
  if (!session.user.tenants?.includes(tenant)) {
    return new Response("Forbidden : tenant scope mismatch", { status: 403 });
  }

  // Inject tenant selector into the PromQL — prevents cross-tenant peek.
  const scopedQuery = injectTenantSelector(query, tenant);

  const promUrl = process.env.PROMETHEUS_URL;
  if (!promUrl) return new Response("Prometheus not configured", { status: 503 });

  const params = new URLSearchParams({
    query: scopedQuery,
    start: `${Math.floor(Date.now() / 1000) - rangeSeconds}`,
    end: `${Math.floor(Date.now() / 1000)}`,
    step: `${step}`,
  });

  try {
    const res = await fetch(`${promUrl}/api/v1/query_range?${params}`, {
      headers: { Accept: "application/json" },
      cache: "no-store",
    });
    if (!res.ok) {
      return new Response(`Prometheus error : ${res.status}`, { status: 502 });
    }
    const data = (await res.json()) as { data: { result: Array<{ values: [number, string][] }> } };
    const values = data.data.result?.[0]?.values ?? [];
    return new Response(JSON.stringify({ values }), {
      headers: { "content-type": "application/json", "cache-control": "no-store" },
    });
  } catch (e) {
    return new Response(`Upstream error : ${(e as Error).message}`, { status: 502 });
  }
}

/**
 * Naive but safe selector injection : if the metric brackets `{...}` are
 * present, append `,tenant="<tenant>"`. Otherwise add `{tenant="..."}`
 * after the metric name. Properly escapes the tenant slug.
 */
function injectTenantSelector(query: string, tenant: string): string {
  // Tenant slug is already validated by NextAuth + cookie middleware.
  const escaped = tenant.replace(/"/g, '\\"');
  // Replace each metric reference. Conservative — match `metric{` pattern.
  return query.replace(/(\b[a-zA-Z_][a-zA-Z0-9_]*)({?)/g, (m, metric, open) => {
    if (open === "{") {
      return `${metric}{tenant="${escaped}",`;
    }
    return m;
  });
}
