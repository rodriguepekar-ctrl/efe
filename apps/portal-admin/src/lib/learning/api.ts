// Read-side API for active learning (server components).

import { getCurrentSession } from "@/lib/auth/session";

const API_BASE = process.env.EFE_API_BASE ?? "http://localhost:8081";

export async function fetchAlQueue(opts: {
  tenantSlug: string; catalogId?: string; status?: string;
}): Promise<any[]> {
  const session = await getCurrentSession();
  if (!session) return [];
  const params = new URLSearchParams({ limit: "50" });
  if (opts.catalogId) params.set("catalog", opts.catalogId);
  if (opts.status)    params.set("status", opts.status);
  const r = await fetch(`${API_BASE}/v1/learning/queue?${params}`, {
    headers: { Authorization: `Bearer ${session.bearer}` },
    cache: "no-store",
  });
  if (!r.ok) return [];
  return r.json();
}

export async function fetchAlStats(opts: {
  tenantSlug: string; catalogId?: string;
}): Promise<{
  queue_enqueued: number; queue_labeling: number;
  queue_awaiting_review: number; queue_failed: number;
  oldest_open_age_seconds: number;
  total_gold_examples: number; total_holdout_examples: number;
  retune_runs_last_30d: number; promotions_last_30d: number;
  cost_cents_today: number; cost_cents_last_30d: number;
}> {
  const session = await getCurrentSession();
  const stub = {
    queue_enqueued: 0, queue_labeling: 0, queue_awaiting_review: 0, queue_failed: 0,
    oldest_open_age_seconds: 0,
    total_gold_examples: 0, total_holdout_examples: 0,
    retune_runs_last_30d: 0, promotions_last_30d: 0,
    cost_cents_today: 0, cost_cents_last_30d: 0,
  };
  if (!session) return stub;
  const params = new URLSearchParams();
  if (opts.catalogId) params.set("catalog", opts.catalogId);
  const r = await fetch(`${API_BASE}/v1/learning/stats?${params}`, {
    headers: { Authorization: `Bearer ${session.bearer}` },
    cache: "no-store",
  });
  if (!r.ok) return stub;
  return r.json();
}

export async function fetchAlRetunes(opts: {
  tenantSlug: string; catalogId?: string; limit?: number;
}): Promise<any[]> {
  const session = await getCurrentSession();
  if (!session) return [];
  const params = new URLSearchParams({ limit: String(opts.limit ?? 50) });
  if (opts.catalogId) params.set("catalog", opts.catalogId);
  const r = await fetch(`${API_BASE}/v1/learning/retune?${params}`, {
    headers: { Authorization: `Bearer ${session.bearer}` },
    cache: "no-store",
  });
  if (!r.ok) return [];
  return r.json();
}

export async function fetchAlRetuneDetail(
  tenantSlug: string, runId: string,
): Promise<any | null> {
  const session = await getCurrentSession();
  if (!session) return null;
  const r = await fetch(`${API_BASE}/v1/learning/retune/${encodeURIComponent(runId)}`, {
    headers: { Authorization: `Bearer ${session.bearer}` },
    cache: "no-store",
  });
  if (r.status === 404) return null;
  if (!r.ok) throw new Error(`fetchAlRetuneDetail : ${r.status}`);
  return r.json();
}

export async function fetchAlGdprPending(opts: {
  tenantSlug: string;
}): Promise<any[]> {
  const session = await getCurrentSession();
  if (!session) return [];
  const r = await fetch(`${API_BASE}/v1/learning/gdpr/erasure?status=pending`, {
    headers: { Authorization: `Bearer ${session.bearer}` },
    cache: "no-store",
  });
  if (!r.ok) return [];
  return r.json();
}
