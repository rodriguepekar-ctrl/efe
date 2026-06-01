// portal-admin curation cost dashboard — M23.

import { fetchCurationCostReport } from "@/lib/curation/api";
import { CurationCostChart } from "@/components/curation/CurationCostChart";
import { requireFeature, requireRole } from "@/lib/auth/guards";

export const metadata = { title: "Curation cost · EFE Portal" };

export default async function CostPage({ params }: { params: { tenant_slug: string } }) {
  await requireFeature(params.tenant_slug, "m23_curation_enabled");
  await requireRole(params.tenant_slug, ["taxonomy_admin", "tenant_admin"]);

  const report = await fetchCurationCostReport({
    tenantSlug: params.tenant_slug,
    days: 30,
    groupBy: ["tool_name", "provider"],
  });

  return (
    <main className="px-6 py-8 max-w-7xl mx-auto">
      <header className="mb-6">
        <h1 className="text-2xl font-semibold text-slate-900">Curation Cost · last 30 days</h1>
        <p className="text-sm text-slate-600 mt-1">
          Per-tool LLM spend ; budget tracking ; provider breakdown.
        </p>
      </header>

      <section className="grid grid-cols-3 gap-4 mb-6">
        <Stat label="Total spend" value={`$${(report.total_cost_cents / 100).toFixed(2)}`} />
        <Stat label="Tool calls" value={report.buckets.reduce((s, b) => s + b.call_count, 0).toLocaleString()} />
        <Stat label="Avg cost / call" value={
          report.buckets.length === 0 ? "—" :
          `$${(report.total_cost_cents / 100 / report.buckets.reduce((s, b) => s + b.call_count, 0)).toFixed(3)}`
        } />
      </section>

      <CurationCostChart buckets={report.buckets} />

      <section className="mt-6 rounded-lg border border-slate-200 bg-white p-5">
        <h2 className="text-sm font-semibold text-slate-700 uppercase tracking-wide mb-3">
          Detail (top 20)
        </h2>
        <table className="w-full text-sm">
          <thead>
            <tr className="text-left text-xs text-slate-500 uppercase">
              <th className="py-2 px-2">Tool</th>
              <th className="py-2 px-2">Provider</th>
              <th className="py-2 px-2 text-right">Calls</th>
              <th className="py-2 px-2 text-right">Tokens</th>
              <th className="py-2 px-2 text-right">Cost</th>
            </tr>
          </thead>
          <tbody>
            {report.buckets.slice(0, 20).map((b, i) => (
              <tr key={i} className="border-t border-slate-100">
                <td className="py-2 px-2"><code>{b.labels.tool_name ?? "—"}</code></td>
                <td className="py-2 px-2">{b.labels.provider ?? "—"}</td>
                <td className="py-2 px-2 text-right tabular-nums">{b.call_count.toLocaleString()}</td>
                <td className="py-2 px-2 text-right tabular-nums">{(b.input_tokens + b.output_tokens).toLocaleString()}</td>
                <td className="py-2 px-2 text-right tabular-nums">${(b.cost_cents / 100).toFixed(2)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>
    </main>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg border border-slate-200 bg-white p-4">
      <div className="text-xs text-slate-500 uppercase tracking-wide">{label}</div>
      <div className="text-2xl font-semibold text-slate-900 mt-1">{value}</div>
    </div>
  );
}
