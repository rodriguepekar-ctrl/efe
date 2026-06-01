"use server";

// Server actions for the M25 agent runtime UI.

import { revalidatePath } from "next/cache";

import { getActiveTenant, getRbacUser } from "@/lib/auth/session";

const API_BASE = process.env.EFE_API_BASE ?? "http://localhost:8081";
const BACKEND_TOKEN = process.env.EFE_BACKEND_TOKEN ?? "";

async function postJson(path: string, body: unknown) {
  const user = await getRbacUser();
  if (!user) throw new Error("not_authenticated");
  const tenant = await getActiveTenant();
  if (!tenant) throw new Error("no_active_tenant");

  const headers: Record<string, string> = {
    "Content-Type": "application/json",
    "X-EFE-Tenant": tenant,
    "X-EFE-User-Email": user.email ?? "",
  };
  if (BACKEND_TOKEN) headers.Authorization = `Bearer ${BACKEND_TOKEN}`;

  const r = await fetch(`${API_BASE}/v1/agent${path}`, {
    method: "POST",
    headers,
    body: JSON.stringify(body),
  });
  if (!r.ok) {
    const text = await r.text();
    throw new Error(`http_${r.status}: ${text}`);
  }
  return r.json();
}

export async function approveAgentStep(input: {
  approvalId: string; reason: string;
}) {
  if (!input.reason || input.reason.trim().length < 10) {
    throw new Error("rationale_too_short");
  }
  const res = await postJson("/approvals/approve", {
    approval_id: input.approvalId,
    reason: input.reason,
  });
  revalidatePath("/agent");
  return res;
}

export async function rejectAgentStep(input: {
  approvalId: string; reason: string;
}) {
  if (!input.reason || input.reason.trim().length < 10) {
    throw new Error("rationale_too_short");
  }
  const res = await postJson("/approvals/reject", {
    approval_id: input.approvalId,
    reason: input.reason,
  });
  revalidatePath("/agent");
  return res;
}

export async function pauseAgentRun(input: { runId: string; reason?: string }) {
  const res = await postJson(`/runs/${input.runId}/pause`, { reason: input.reason });
  revalidatePath(`/agent/runs/${input.runId}`);
  return res;
}

export async function resumeAgentRun(input: { runId: string }) {
  const res = await postJson(`/runs/${input.runId}/resume`, {});
  revalidatePath(`/agent/runs/${input.runId}`);
  return res;
}

export async function terminateAgentRun(input: { runId: string; reason: string }) {
  if (!input.reason || input.reason.trim().length < 10) {
    throw new Error("rationale_too_short");
  }
  const res = await postJson(`/runs/${input.runId}/terminate`, { reason: input.reason });
  revalidatePath(`/agent/runs/${input.runId}`);
  return res;
}

export async function createAgentRun(input: {
  templateSlug: string;
  initialPrompt: unknown;
  toolWhitelist?: string[];
  timeoutSeconds?: number;
  maxCostUsd?: number;
}) {
  const tenant = await getActiveTenant();
  if (!tenant) throw new Error("no_active_tenant");
  const res = await postJson("/runs", {
    tenant_id: tenant,
    template_slug: input.templateSlug,
    initial_prompt: input.initialPrompt,
    tool_whitelist: input.toolWhitelist,
    timeout_seconds: input.timeoutSeconds,
    max_cost_usd: input.maxCostUsd,
  });
  revalidatePath("/agent");
  return res;
}
