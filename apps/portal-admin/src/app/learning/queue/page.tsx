// portal-admin active learning queue page — M24.

import { Suspense } from "react";

import { GoldLabelProposalCard } from "@/components/learning/GoldLabelProposalCard";
import { QueueStatusBanner } from "@/components/learning/QueueStatusBanner";
import { fetchAlQueue, fetchAlStats } from "@/lib/learning/api";
import { requireFeature, requireRole } from "@/lib/auth/guards";

export const metadata = { title: "Active Learning Queue · EFE Portal" };

export default async function ActiveLearningQueuePage({
  params, searchParams,
}: {
  params: { tenant_slug: string };
  searchParams: { catalog?: string; status?: string };
}) {
  await requireFeature(params.tenant_slug, "m24_active_learning_enabled");
  const session = await requireRole(params.tenant_slug, [
    "viewer", "curation_reviewer", "curation_promoter", "taxonomy_admin", "tenant_admin",
  ]);

  const [items, stats] = await Promise.all([
    fetchAlQueue({
      tenantSlug: params.tenant_slug,
      catalogId: searchParams.catalog,
      status: searchParams.status,
    }),
    fetchAlStats({ tenantSlug: params.tenant_slug, catalogId: searchParams.catalog }),
  ]);

  const isAtCapacity = stats.queue_enqueued + stats.queue_labeling > 8000;

  return (
    <main className="px-6 py-8 max-w-7xl mx-auto">
      <header className="mb-6">
        <h1 className="text-2xl font-semibold text-slate-900">
          Active Learning Queue · {params.tenant_slug}
        </h1>
        <p className="text-sm text-slate-600 mt-1">
          Production samples awaiting LLM labeling + human review.
          Confirmations populate the gold dataset for the next bundle retune.
        </p>
      </header>

      {isAtCapacity && <QueueStatusBanner stats={stats} />}

      <section className="grid grid-cols-2 md:grid-cols-4 gap-3 mb-6">
        <Stat label="Enqueued" value={stats.queue_enqueued} />
        <Stat label="LLM labeling" value={stats.queue_labeling} />
        <Stat label="Awaiting review" value={stats.queue_awaiting_review} highlight={stats.queue_awaiting_review > 0} />
        <Stat label="Failed" value={stats.queue_failed} />
      </section>

      <Suspense fallback={<QueueSkeleton />}>
        <div className="space-y-3">
          {items.length === 0 ? (
            <EmptyQueue />
          ) : (
            items.map((it) => (
              <GoldLabelProposalCard
                key={it.id}
                item={it}
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

function Stat({ label, value, highlight }: { label: string; value: number; highlight?: boolean }) {
  return (
    <div className={`rounded-lg border p-4 ${highlight ? "border-amber-400 bg-amber-50" : "border-slate-200 bg-white"}`}>
      <div className="text-xs text-slate-500 uppercase tracking-wide">{label}</div>
      <div className="text-2xl font-semibold text-slate-900 mt-1">{value.toLocaleString()}</div>
    </div>
  );
}

function QueueSkeleton() {
  return (
    <div className="space-y-2">
      {[1, 2, 3].map((i) => (
        <div key={i} className="h-20 rounded-lg bg-slate-100 animate-pulse" />
      ))}
    </div>
  );
}

function EmptyQueue() {
  return (
    <div className="rounded-lg border border-slate-200 bg-white p-8 text-center">
      <div className="text-4xl mb-2">✨</div>
      <div className="text-slate-700 font-medium">Queue is empty</div>
      <div className="text-sm text-slate-500 mt-1">
        Low-confidence production classifies will appear here for labeling.
      </div>
    </div>
  );
}
