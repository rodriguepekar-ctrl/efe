"use server";

// Server actions for the curation UI. These are the entry points called by
// the client components — they enforce server-side auth + freshness checks
// before forwarding to the gRPC CurationService.

import { revalidatePath } from "next/cache";
import { getCurrentSession, refreshOidcToken } from "@/lib/auth/session";
import { CurationGrpcClient } from "@/lib/curation/grpc";

const MIN_RATIONALE_LEN = 10;
const OIDC_FRESHNESS_SECONDS = 600; // 10 minutes

export async function approveProposal({
  proposalId, rationale,
}: { proposalId: string; rationale: string }) {
  if (rationale.trim().length < MIN_RATIONALE_LEN) {
    throw new Error(`rationale must be ≥ ${MIN_RATIONALE_LEN} chars`);
  }
  const session = await getCurrentSession();
  if (!session) throw new Error("not authenticated");
  if (!canReview(session.role)) throw new Error("permission denied : reviewer role required");

  // Freshness check : iat must be within 10 min ; refresh otherwise.
  const ageSec = Math.floor((Date.now() - session.iat * 1000) / 1000);
  const freshSession = ageSec > OIDC_FRESHNESS_SECONDS
    ? await refreshOidcToken()
    : session;

  const client = new CurationGrpcClient(freshSession.bearer);
  await client.approveProposal({
    proposalId,
    rationale: rationale.trim(),
    oidcIdTokenHash: freshSession.idTokenHash,
    oidcTokenIat: new Date(freshSession.iat * 1000).toISOString(),
  });

  revalidatePath(`/${session.tenantSlug}/curation/queue`);
  return { ok: true };
}

export async function rejectProposal({
  proposalId, rationale,
}: { proposalId: string; rationale: string }) {
  if (rationale.trim().length < MIN_RATIONALE_LEN) {
    throw new Error(`rationale must be ≥ ${MIN_RATIONALE_LEN} chars`);
  }
  const session = await getCurrentSession();
  if (!session) throw new Error("not authenticated");
  if (!canReview(session.role)) throw new Error("permission denied");

  const client = new CurationGrpcClient(session.bearer);
  await client.rejectProposal({ proposalId, rationale: rationale.trim() });
  revalidatePath(`/${session.tenantSlug}/curation/queue`);
  return { ok: true };
}

export async function acknowledgeDriftEvent({
  eventId, note,
}: { eventId: string; note?: string }) {
  const session = await getCurrentSession();
  if (!session) throw new Error("not authenticated");
  const client = new CurationGrpcClient(session.bearer);
  await client.acknowledgeDriftEvent({ eventId, note });
  revalidatePath(`/${session.tenantSlug}/curation/drift`);
  return { ok: true };
}

export async function promoteProposal({
  proposalId, skipCanary,
}: { proposalId: string; skipCanary?: boolean }) {
  const session = await getCurrentSession();
  if (!session) throw new Error("not authenticated");
  if (!canPromote(session.role)) throw new Error("permission denied : promoter role required");
  if (skipCanary && session.role !== "tenant_admin") {
    throw new Error("skip_canary requires tenant_admin");
  }
  const client = new CurationGrpcClient(session.bearer);
  const resp = await client.promoteProposal({ proposalId, skipCanary: !!skipCanary });
  revalidatePath(`/${session.tenantSlug}/curation`);
  return resp;
}

export async function rollbackVersion({
  versionId, reason, catalogId,
}: { versionId: string; reason: string; catalogId: string }) {
  if (reason.trim().length < MIN_RATIONALE_LEN) {
    throw new Error(`reason must be ≥ ${MIN_RATIONALE_LEN} chars`);
  }
  const session = await getCurrentSession();
  if (!session) throw new Error("not authenticated");
  if (!canPromote(session.role)) throw new Error("permission denied");
  const client = new CurationGrpcClient(session.bearer);
  const resp = await client.rollbackTaxonomyVersion({
    targetVersionId: versionId, catalogId, reason: reason.trim(),
  });
  revalidatePath(`/${session.tenantSlug}/curation`);
  return resp;
}

function canReview(role: string): boolean {
  return ["curation_reviewer", "taxonomy_admin", "tenant_admin"].includes(role);
}

function canPromote(role: string): boolean {
  return ["curation_promoter", "taxonomy_admin", "tenant_admin"].includes(role);
}
