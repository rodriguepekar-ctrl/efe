// portal-admin retune history page — M24.

import Link from "next/link";
import { fetchAlRetunes } from "@/lib/learning/api";
import { requireFeature, requireRole } from "@/lib/auth/guards";
import { RetuneRunCard } from "@/components/learning/RetuneRunCard";

export const metadata = { title: "Retune Runs · EFE Portal" };

export default async function RetunesPage({
  params, searchParams,
}: {
  params: { tenant_slug: string };
  searchParams: { catalog?: string };
}) {
  await requireFeature(params.tenant_slug, "m24_active_learning_enabled");
  const session = await requireRole(params.tenant_slug, [
    "viewer", "curation_promoter", "taxonomy_admin", "tenant_admin",
  ]);
  const runs = await fetchAlRetunes({
    tenantSlug: params.tenant_slug,
    catalogId: searchParams.catalog,
    limit: 100,
  });

  return (
    <main className="px-6 py-8 max-w-7xl mx-auto">
      <header className="mb-6 flex items-start justify-between gap-4">
        <div>
          <h1 className="text-2xl font-semibold text-slate-900">Retune Runs</h1>
          <p className="text-sm text-slate-600 mt-1">
            History of bundle retune runs. Flagged-for-promotion runs await operator approval.
          </p>
        </div>
        <Link href={`/${params.tenant_slug}/learning/queue`}
          className="text-sm text-blue-600 hover:underline self-end whitespace-nowrap">
          → Queue
        </Link>
      </header>

      <section className="space-y-3">
        {runs.length === 0 ? (
          <div className="rounded-lg border border-slate-200 bg-white p-8 text-center">
            <div className="text-4xl mb-2">📊</div>
            <div className="text-slate-700 font-medium">No retune runs yet</div>
            <div className="text-sm text-slate-500 mt-1">
              Trigger one : <code>efe-admin learning retune trigger --catalog &lt;id&gt;</code>
            </div>
          </div>
        ) : (
          runs.map((r: any) => (
            <RetuneRunCard
              key={r.id}
              run={r}
              tenantSlug={params.tenant_slug}
              userRole={session.role}
            />
          ))
        )}
      </section>
    </main>
  );
}
