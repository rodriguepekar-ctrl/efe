// SSE streaming endpoint for agent run steps. Proxies to the
// efe-server `/v1/agent/runs/:id/steps/stream` SSE endpoint.

import { NextRequest } from "next/server";
import { getRbacUser, getActiveTenant } from "@/lib/auth/session";

export const dynamic = "force-dynamic";
export const runtime = "nodejs"; // SSE needs Node, not Edge.

const API_BASE = process.env.EFE_API_BASE ?? "http://localhost:8081";
const BACKEND_TOKEN = process.env.EFE_BACKEND_TOKEN ?? "";

export async function GET(
  req: NextRequest,
  { params }: { params: { id: string } },
) {
  const user = await getRbacUser();
  if (!user) {
    return new Response("unauthorized", { status: 401 });
  }
  const tenant = await getActiveTenant();
  if (!tenant) {
    return new Response("no active tenant", { status: 401 });
  }

  // Validate the run id is a UUID.
  if (!/^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(params.id)) {
    return new Response("invalid run id", { status: 400 });
  }

  const upstream = await fetch(
    `${API_BASE}/v1/agent/runs/${params.id}/steps/stream`,
    {
      headers: {
        Authorization: `Bearer ${BACKEND_TOKEN}`,
        Accept: "text/event-stream",
        "X-EFE-Tenant": tenant,
        "X-EFE-User-Email": user.email ?? "",
      },
      signal: req.signal,
    },
  );

  if (!upstream.ok || !upstream.body) {
    return new Response(
      `upstream error: ${upstream.status}`,
      { status: upstream.status },
    );
  }

  return new Response(upstream.body, {
    status: 200,
    headers: {
      "Content-Type": "text/event-stream",
      "Cache-Control": "no-cache, no-transform",
      "Connection": "keep-alive",
      "X-Accel-Buffering": "no",
    },
  });
}
