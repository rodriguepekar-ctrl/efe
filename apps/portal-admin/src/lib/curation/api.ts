// Curation read-side API client (server-side).
//
// Functions here are called from React Server Components to fetch data
// for rendering. All calls go through the backend REST gateway with the
// caller's bearer token (server-side fetch).

import { getCurrentSession } from "@/lib/auth/session";

const API_BASE = process.env.EFE_API_BASE ?? "http://localhost:8081";

type Proposal = {
  id: string;
  catalog_id: string;
  class_id_primary?: string;
  kind: "SPLIT" | "MERGE" | "EXPLAIN" | "GENERATE_SYNTH" | "AUDIT_VS_GOLD";
  status: "PENDING" | "APPROVED" | "REJECTED" | "PROMOTED" | "ROLLED_BACK" | "EXPIRED";
  proposed_by: string;
  proposed_via: "mcp_tool" | "cli" | "portal" | "auto_drift";
  llm_provider?: string;
  llm_model?: string;
  cost_cents?: number;
  created_at: string;
  rationale_markdown: string;
  severity?: "LOW" | "MEDIUM" | "HIGH";
  diff_summary?: { added: number; removed: number; classes_affected: number };
  confidence_score?: number;
};

export async function fetchPendingProposals(opts: {
  tenantSlug: string;
  catalogId?: string;
  kind?: string;
}): Promise<Proposal[]> {
  const session = await getCurrentSession();
  if (!session) return [];
  const params = new URLSearchParams({ status: "pending", limit: "50" });
  if (opts.catalogId) params.set("catalog", opts.catalogId);
  if (opts.kind) params.set("kind", opts.kind);
  const r = await fetch(`${API_BASE}/v1/curation/proposals?${params}`, {
    headers: { Authorization: `Bearer ${session.bearer}` },
    cache: "no-store",
  });
  if (!r.ok) {
    if (r.status === 404) return [];
    throw new Error(`fetchPendingProposals : ${r.status}`);
  }
  return r.json();
}

export async function fetchProposalDetail(
  tenantSlug: string, proposalId: string,
): Promise<{
  summary: any;
  rationale_markdown: string;
  diff_json: any;
  reviews: any[];
} | null> {
  const session = await getCurrentSession();
  if (!session) return null;
  const r = await fetch(
    `${API_BASE}/v1/curation/proposals/${encodeURIComponent(proposalId)}`,
    {
      headers: { Authorization: `Bearer ${session.bearer}` },
      cache: "no-store",
    },
  );
  if (r.status === 404) return null;
  if (!r.ok) throw new Error(`fetchProposalDetail : ${r.status}`);
  return r.json();
}

export async function fetchDriftEvents(opts: {
  tenantSlug: string;
  onlyOpen?: boolean;
  minSeverity?: string;
}): Promise<any[]> {
  const session = await getCurrentSession();
  if (!session) return [];
  const params = new URLSearchParams({
    min_severity: opts.minSeverity ?? "LOW",
    limit: "50",
  });
  if (opts.onlyOpen) params.set("only_unacknowledged", "true");
  const r = await fetch(`${API_BASE}/v1/curation/drift?${params}`, {
    headers: { Authorization: `Bearer ${session.bearer}` },
    cache: "no-store",
  });
  if (!r.ok) return [];
  return r.json();
}

export async function fetchCurationCostReport(opts: {
  tenantSlug: string;
  days: number;
  groupBy: string[];
}): Promise<{ total_cost_cents: number; buckets: any[] }> {
  const session = await getCurrentSession();
  if (!session) return { total_cost_cents: 0, buckets: [] };
  const to = new Date();
  const from = new Date(to.getTime() - opts.days * 86_400_000);
  const r = await fetch(`${API_BASE}/v1/curation/cost/report`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${session.bearer}`,
      "content-type": "application/json",
    },
    body: JSON.stringify({
      from: from.toISOString(),
      to: to.toISOString(),
      group_by: opts.groupBy,
    }),
    cache: "no-store",
  });
  if (!r.ok) return { total_cost_cents: 0, buckets: [] };
  return r.json();
}
