// portal-admin curation queue — M23.
//
// Lists pending proposals for the active tenant + selected catalog.
// Reviewers can drill into a proposal, approve / reject, or bulk-act.
//
// Path : /[tenant_slug]/curation/queue

import { Suspense } from "react";
import { redirect } from "next/navigation";

import { ProposalCard } from "@/components/curation/ProposalCard";
import { CurationFiltersBar } from "@/components/curation/CurationFiltersBar";
import { fetchPendingProposals } from "@/lib/curation/api";
import { requireRole, requireFeature } from "@/lib/auth/guards";

export const metadata = {
  title: "Curation queue · EFE Portal",
  description: "Review and approve LLM-assisted taxonomy proposals.",
};

type SearchParams = {
  catalog?: string;
  kind?: "SPLIT" | "MERGE" | "EXPLAIN" | "GENERATE_SYNTH" | "AUDIT_VS_GOLD";
  severity?: "LOW" | "MEDIUM" | "HIGH";
};

export default async function CurationQueuePage({
  params,
  searchParams,
}: {
  params: { tenant_slug: string };
  searchParams: SearchParams;
}) {
  // Auth + feature flag guards (server-side ; UI hides actions but this is the auth boundary)
  await requireFeature(params.tenant_slug, "m23_curation_enabled");
  const session = await requireRole(params.tenant_slug, [
    "viewer", "curation_reviewer", "curation_promoter", "taxonomy_admin", "tenant_admin",
  ]);

  if (!session) {
    redirect(`/${params.tenant_slug}/login?next=/curation/queue`);
  }

  const proposals = await fetchPendingProposals({
    tenantSlug: params.tenant_slug,
    catalogId: searchParams.catalog,
    kind: searchParams.kind,
  });

  const stats = {
    pending: proposals.filter((p) => p.status === "PENDING").length,
    approved: proposals.filter((p) => p.status === "APPROVED").length,
    cost30d: proposals.reduce((sum, p) => sum + (p.cost_cents ?? 0), 0),
  };

  return (
    <main className="px-6 py-8 max-w-7xl mx-auto">
      <header className="mb-6">
        <h1 className="text-2xl font-semibold text-slate-900">
          Curation Queue · {params.tenant_slug}
        </h1>
        <p className="text-sm text-slate-600 mt-1">
          LLM-assisted taxonomy proposals awaiting human review (Art. 14 of EU AI Act).
        </p>
      </header>

      <section className="grid grid-cols-3 gap-4 mb-6">
        <Stat label="Pending" value={stats.pending} highlight={stats.pending > 0} />
        <Stat label="Approved this week" value={stats.approved} />
        <Stat label="Cost (30d)" value={`$${(stats.cost30d / 100).toFixed(2)}`} />
      </section>

      <CurationFiltersBar tenantSlug={params.tenant_slug} initial={searchParams} />

      <Suspense fallback={<QueueSkeleton />}>
        <div className="space-y-4 mt-6">
          {proposals.length === 0 ? (
            <EmptyQueue />
          ) : (
            proposals.map((p) => (
              <ProposalCard
                key={p.id}
                proposal={p}
                tenantSlug={params.tenant_slug}
                userRole={session.role}
              />
            ))
          )}
        </div>
      </Suspense>
    </main>
  );
}

function Stat({ label, value, highlight }: { label: string; value: string | number; highlight?: boolean }) {
  return (
    <div className={`rounded-lg border p-4 ${highlight ? "border-amber-400 bg-amber-50" : "border-slate-200 bg-white"}`}>
      <div className="text-xs text-slate-500 uppercase tracking-wide">{label}</div>
      <div className="text-2xl font-semibold text-slate-900 mt-1">{value}</div>
    </div>
  );
}

function QueueSkeleton() {
  return (
    <div className="space-y-3 mt-6">
      {[1, 2, 3].map((i) => (
        <div key={i} className="h-24 rounded-lg bg-slate-100 animate-pulse" />
      ))}
    </div>
  );
}

function EmptyQueue() {
  return (
    <div className="rounded-lg border border-slate-200 bg-white p-8 text-center">
      <div className="text-4xl mb-2">✨</div>
      <div className="text-slate-700 font-medium">No pending proposals</div>
      <div className="text-sm text-slate-500 mt-1">
        Proposals appear here when a curation tool runs (manually or via drift detection).
      </div>
    </div>
  );
}
