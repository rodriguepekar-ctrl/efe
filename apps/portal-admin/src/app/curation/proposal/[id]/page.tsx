// portal-admin curation proposal detail page — M23.
//
// Displays full proposal context : LLM-generated rationale, semantic diff
// (with Sankey + confusion-matrix-delta heatmap), reviewer history, and
// approval/reject controls.

import { notFound } from "next/navigation";
import { fetchProposalDetail } from "@/lib/curation/api";
import { ProposalDiff } from "@/components/curation/ProposalDiff";
import { requireFeature, requireRole } from "@/lib/auth/guards";

export const dynamic = "force-dynamic";

export default async function ProposalDetailPage({
  params,
}: { params: { tenant_slug: string; id: string } }) {
  await requireFeature(params.tenant_slug, "m23_curation_enabled");
  const session = await requireRole(params.tenant_slug, [
    "viewer", "curation_reviewer", "curation_promoter", "taxonomy_admin", "tenant_admin",
  ]);
  const detail = await fetchProposalDetail(params.tenant_slug, params.id);
  if (!detail) notFound();

  return (
    <main className="px-6 py-8 max-w-7xl mx-auto">
      <header className="mb-6">
        <div className="text-xs text-slate-500 mb-1">
          <a href={`/${params.tenant_slug}/curation/queue`} className="hover:underline">← Back to queue</a>
        </div>
        <h1 className="text-2xl font-semibold text-slate-900">
          Proposal <code className="text-lg">#{detail.summary.id.slice(0, 8)}</code>
        </h1>
        <p className="text-sm text-slate-600 mt-1">
          {detail.summary.kind} · {detail.summary.catalog_id} · {detail.summary.status}
        </p>
      </header>

      <section className="grid grid-cols-1 lg:grid-cols-3 gap-6">
        <div className="lg:col-span-2 space-y-6">
          <Card title="Rationale">
            <div className="prose prose-sm max-w-none whitespace-pre-wrap">
              {detail.rationale_markdown}
            </div>
          </Card>

          <Card title="Semantic diff">
            <ProposalDiff diffJson={detail.diff_json} />
          </Card>
        </div>

        <aside className="space-y-6">
          <Card title="Metadata">
            <dl className="text-sm space-y-1.5">
              <Row label="Tenant"     value={detail.summary.tenant_id} />
              <Row label="Proposed"   value={detail.summary.proposed_by} />
              <Row label="Via"        value={detail.summary.proposed_via} />
              <Row label="Created"    value={new Date(detail.summary.created_at).toLocaleString()} />
              <Row label="LLM cost"   value={`$${(detail.summary.actual_cost_cents / 100).toFixed(2)}`} />
              <Row label="Expires"    value={new Date(detail.summary.expires_at).toLocaleString()} />
            </dl>
          </Card>

          <Card title={`Reviews (${detail.reviews.length})`}>
            {detail.reviews.length === 0 ? (
              <div className="text-sm text-slate-500">No reviews yet.</div>
            ) : (
              <ul className="space-y-3">
                {detail.reviews.map((r) => (
                  <li key={r.id} className="border-l-2 pl-3 border-slate-300">
                    <div className="text-sm font-medium">
                      {r.decision === "APPROVE" ? "✓" : "✗"} {r.reviewer_oidc_sub}
                      <span className="ml-2 text-xs text-slate-500">{r.reviewer_role}</span>
                    </div>
                    <div className="text-xs text-slate-500">
                      {new Date(r.reviewed_at).toLocaleString()}
                    </div>
                    <div className="text-sm mt-1 italic">"{r.rationale}"</div>
                  </li>
                ))}
              </ul>
            )}
          </Card>
        </aside>
      </section>
    </main>
  );
}

function Card({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="rounded-lg border border-slate-200 bg-white p-5">
      <h2 className="text-sm font-semibold text-slate-700 uppercase tracking-wide mb-3">{title}</h2>
      {children}
    </section>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex justify-between gap-3">
      <dt className="text-slate-500">{label}</dt>
      <dd className="text-slate-900 text-right">{value}</dd>
    </div>
  );
}
