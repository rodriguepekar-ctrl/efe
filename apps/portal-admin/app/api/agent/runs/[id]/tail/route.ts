// Polling fallback for environments where SSE is blocked by an
// intermediary (some corporate proxies). Returns a JSON snapshot of
// the latest steps + a `terminal` flag.

import { NextRequest, NextResponse } from "next/server";
import { getRbacUser, getActiveTenant } from "@/lib/auth/session";

export const dynamic = "force-dynamic";

const API_BASE = process.env.EFE_API_BASE ?? "http://localhost:8081";
const BACKEND_TOKEN = process.env.EFE_BACKEND_TOKEN ?? "";

export async function GET(
  req: NextRequest,
  { params }: { params: { id: string } },
) {
  const user = await getRbacUser();
  if (!user) {
    return NextResponse.json({ error: "unauthorized" }, { status: 401 });
  }
  const tenant = await getActiveTenant();
  if (!tenant) {
    return NextResponse.json({ error: "no_active_tenant" }, { status: 401 });
  }

  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(params.id)) {
    return NextResponse.json({ error: "invalid_id" }, { status: 400 });
  }

  const since = Number(req.nextUrl.searchParams.get("since") ?? "0");
  const limit = Math.min(
    Number(req.nextUrl.searchParams.get("limit") ?? "50"),
    200,
  );

  const r = await fetch(
    `${API_BASE}/v1/agent/runs/${params.id}/steps?since=${since}&limit=${limit}`,
    {
      headers: {
        Authorization: `Bearer ${BACKEND_TOKEN}`,
        "X-EFE-Tenant": tenant,
      },
      cache: "no-store",
    },
  );
  if (!r.ok) {
    return NextResponse.json(
      { error: "upstream", status: r.status },
      { status: r.status },
    );
  }
  return NextResponse.json(await r.json());
}
