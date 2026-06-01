// Read-side API for the M25 agent runtime (server components).
//
// The portal-admin REST gateway (`/v1/agent/*` on `efe-server`) is the
// transport. Bearer auth comes from a service-account token mounted
// into the portal pod (`EFE_BACKEND_TOKEN`) — distinct from the user's
// OIDC session, which only proves authorisation to USE the portal.

import { getRbacUser, getActiveTenant } from "@/lib/auth/session";

const API_BASE = process.env.EFE_API_BASE ?? "http://localhost:8081";
const BACKEND_TOKEN = process.env.EFE_BACKEND_TOKEN ?? "";

export type AgentRunStatus =
  | "PENDING" | "PROVISIONING" | "RUNNING" | "AWAITING_APPROVAL"
  | "PAUSED" | "RESUMING"
  | "COMPLETED" | "FAILED" | "TERMINATED" | "TIMED_OUT" | "COST_CAPPED";

export interface AgentRun {
  id: string;
  tenant_id: string;
  template_id: string;
  profile: string;
  provider: string;
  model: string;
  status: AgentRunStatus;
  current_step: number;
  max_steps: number;
  current_cost_cents: number;
  max_cost_cents: number;
  created_at: string;
  started_at: string | null;
  completed_at: string | null;
  failure_reason: string | null;
  vm_id: string | null;
}

export interface AgentStep {
  id: string;
  step_index: number;
  kind: string;
  cost_cents: number;
  payload?: unknown;
  created_at: string;
}

export interface AgentApproval {
  id: string;
  agent_run_id: string;
  step_index: number;
  tool_name: string;
  tool_args: unknown;
  justification: string | null;
  status: string;
  expires_at: string;
  requested_at: string;
}

export interface AgentRunDetail {
  run: AgentRun;
  recent_steps: AgentStep[];
  pending_approvals: AgentApproval[];
}

export interface DpiaStatus {
  signed_at: string | null;
  expires_at: string | null;
  addendum_hash: string | null;
  is_valid: boolean;
}

/**
 * Compose tenant slug + bearer token for backend calls. Returns null if
 * the user isn't signed in or has no tenant — caller must short-circuit.
 */
async function authContext(): Promise<{ tenant: string; bearer: string } | null> {
  const user = await getRbacUser();
  if (!user) return null;
  const tenant = await getActiveTenant();
  if (!tenant) return null;
  return { tenant, bearer: BACKEND_TOKEN };
}

function authHeaders(bearer: string): Record<string, string> {
  return bearer ? { Authorization: `Bearer ${bearer}` } : {};
}

export async function fetchAgentRuns(opts: {
  includeCompleted?: boolean; limit?: number;
}): Promise<AgentRun[]> {
  const ctx = await authContext();
  if (!ctx) return [];
  const params = new URLSearchParams({
    tenant: ctx.tenant,
    include_completed: String(opts.includeCompleted ?? false),
    limit: String(opts.limit ?? 50),
  });
  const r = await fetch(`${API_BASE}/v1/agent/runs?${params}`, {
    headers: authHeaders(ctx.bearer),
    cache: "no-store",
  });
  if (!r.ok) return [];
  return r.json();
}

export async function fetchAgentRun(runId: string): Promise<AgentRunDetail | null> {
  const ctx = await authContext();
  if (!ctx) return null;
  const r = await fetch(`${API_BASE}/v1/agent/runs/${runId}`, {
    headers: authHeaders(ctx.bearer),
    cache: "no-store",
  });
  if (!r.ok) return null;
  return r.json();
}

export async function fetchAgentTemplates(): Promise<Array<Record<string, unknown>>> {
  const ctx = await authContext();
  if (!ctx) return [];
  const r = await fetch(
    `${API_BASE}/v1/agent/templates?tenant=${ctx.tenant}`,
    { headers: authHeaders(ctx.bearer), cache: "no-store" }
  );
  if (!r.ok) return [];
  return r.json();
}

export async function fetchAgentDpia(): Promise<DpiaStatus> {
  const stub: DpiaStatus = {
    signed_at: null, expires_at: null, addendum_hash: null, is_valid: false,
  };
  const ctx = await authContext();
  if (!ctx) return stub;
  const r = await fetch(
    `${API_BASE}/v1/agent/dpia/status?tenant=${ctx.tenant}`,
    { headers: authHeaders(ctx.bearer), cache: "no-store" }
  );
  if (!r.ok) return stub;
  return r.json();
}

export async function fetchAgentCostDaily(opts: { days: number }) {
  const ctx = await authContext();
  if (!ctx) return [];
  const r = await fetch(
    `${API_BASE}/v1/agent/cost/daily?tenant=${ctx.tenant}&days=${opts.days}`,
    { headers: authHeaders(ctx.bearer), cache: "no-store" }
  );
  if (!r.ok) return [];
  return r.json();
}

/** Convenience for components that only need the active tenant slug. */
export async function getActiveTenantSlug(): Promise<string | null> {
  return getActiveTenant();
}
