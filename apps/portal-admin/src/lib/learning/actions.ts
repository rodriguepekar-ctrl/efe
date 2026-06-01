"use server";

// Server actions for active learning UI.

import { revalidatePath } from "next/cache";
import { getCurrentSession, refreshOidcToken } from "@/lib/auth/session";
import { ActiveLearningGrpcClient } from "@/lib/learning/grpc";

const MIN_RATIONALE_LEN = 10;
const OIDC_FRESHNESS_SECONDS = 600;

function canReview(role: string): boolean {
  return ["curation_reviewer", "taxonomy_admin", "tenant_admin"].includes(role);
}
function canPromote(role: string): boolean {
  return ["curation_promoter", "taxonomy_admin", "tenant_admin"].includes(role);
}
function isTenantAdmin(role: string): boolean {
  return role === "tenant_admin";
}

async function freshSession() {
  const s = await getCurrentSession();
  if (!s) throw new Error("not authenticated");
  const age = Math.floor((Date.now() - s.iat * 1000) / 1000);
  return age > OIDC_FRESHNESS_SECONDS ? await refreshOidcToken() : s;
}

export async function confirmGoldLabel({
  proposalId, rationale,
}: { proposalId: string; rationale: string }) {
  if (rationale.trim().length < MIN_RATIONALE_LEN) {
    throw new Error(`rationale must be ≥ ${MIN_RATIONALE_LEN} chars`);
  }
  const session = await freshSession();
  if (!canReview(session.role)) throw new Error("permission denied : reviewer required");
  const client = new ActiveLearningGrpcClient(session.bearer);
  await client.confirmGoldLabel({ proposalId, rationale: rationale.trim() });
  revalidatePath(`/${session.tenantSlug}/learning/queue`);
  return { ok: true };
}

export async function editAndConfirmGoldLabel({
  proposalId, finalClassId, rationale,
}: { proposalId: string; finalClassId: string; rationale: string }) {
  if (rationale.trim().length < MIN_RATIONALE_LEN) {
    throw new Error(`rationale must be ≥ ${MIN_RATIONALE_LEN} chars`);
  }
  const session = await freshSession();
  if (!canReview(session.role)) throw new Error("permission denied");
  const client = new ActiveLearningGrpcClient(session.bearer);
  await client.editAndConfirm({ proposalId, finalClassId, rationale: rationale.trim() });
  revalidatePath(`/${session.tenantSlug}/learning/queue`);
  return { ok: true };
}

export async function rejectGoldLabel({
  proposalId, rationale,
}: { proposalId: string; rationale: string }) {
  if (rationale.trim().length < MIN_RATIONALE_LEN) {
    throw new Error(`rationale must be ≥ ${MIN_RATIONALE_LEN} chars`);
  }
  const session = await freshSession();
  if (!canReview(session.role)) throw new Error("permission denied");
  const client = new ActiveLearningGrpcClient(session.bearer);
  await client.rejectGoldLabel({ proposalId, rationale: rationale.trim() });
  revalidatePath(`/${session.tenantSlug}/learning/queue`);
  return { ok: true };
}

export async function triggerBundleRetune({
  catalogId, forceBelowThreshold,
}: { catalogId: string; forceBelowThreshold?: boolean }) {
  const session = await freshSession();
  if (!canPromote(session.role)) throw new Error("permission denied");
  if (forceBelowThreshold && !isTenantAdmin(session.role)) {
    throw new Error("force_below_threshold requires tenant_admin role");
  }
  const client = new ActiveLearningGrpcClient(session.bearer);
  const r = await client.triggerRetune({
    catalogId, forceBelowThreshold: !!forceBelowThreshold,
  });
  revalidatePath(`/${session.tenantSlug}/learning/retunes`);
  return r;
}

export async function promoteRetuneRun({
  runId, skipCanary,
}: { runId: string; skipCanary?: boolean }) {
  const session = await freshSession();
  if (!canPromote(session.role)) throw new Error("permission denied");
  if (skipCanary && !isTenantAdmin(session.role)) {
    throw new Error("skip_canary requires tenant_admin");
  }
  const client = new ActiveLearningGrpcClient(session.bearer);
  const r = await client.promote({ runId, skipCanary: !!skipCanary });
  revalidatePath(`/${session.tenantSlug}/learning/retunes`);
  return r;
}

export async function rollbackRetune({
  runId, reason,
}: { runId: string; reason: string }) {
  if (reason.trim().length < MIN_RATIONALE_LEN) {
    throw new Error(`reason must be ≥ ${MIN_RATIONALE_LEN} chars`);
  }
  const session = await freshSession();
  if (!canPromote(session.role)) throw new Error("permission denied");
  const client = new ActiveLearningGrpcClient(session.bearer);
  const r = await client.rollback({ runId, reason: reason.trim() });
  revalidatePath(`/${session.tenantSlug}/learning/retunes`);
  return r;
}

export async function submitGdprErasure(req: {
  catalogId: string; urgency: string;
  contentHashes: string[];
  subjectId?: string; reason: string;
}) {
  if (req.reason.trim().length < MIN_RATIONALE_LEN) {
    throw new Error(`reason must be ≥ ${MIN_RATIONALE_LEN} chars`);
  }
  const session = await freshSession();
  if (!isTenantAdmin(session.role)) {
    throw new Error("GDPR Art. 17 erasure requires tenant_admin");
  }
  const client = new ActiveLearningGrpcClient(session.bearer);
  const r = await client.submitGdprErasure(req);
  revalidatePath(`/${session.tenantSlug}/learning`);
  return r;
}
