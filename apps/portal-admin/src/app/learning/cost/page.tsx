// portal-admin active learning cost dashboard — M24.

import { fetchAlStats } from "@/lib/learning/api";
import { requireFeature, requireRole } from "@/lib/auth/guards";

export const metadata = { title: "Active Learning Cost · EFE Portal" };

export default async function AlCostPage({ params }: { params: { tenant_slug: string } }) {
  await requireFeature(params.tenant_slug, "m24_active_learning_enabled");
  await requireRole(params.tenant_slug, ["taxonomy_admin", "tenant_admin"]);
  const stats = await fetchAlStats({ tenantSlug: params.tenant_slug });

  return (
    <main className="px-6 py-8 max-w-7xl mx-auto">
      <header className="mb-6">
        <h1 className="text-2xl font-semibold text-slate-900">Active Learning Cost</h1>
        <p className="text-sm text-slate-600 mt-1">
          LLM labeling spend (budget category : <code>active_learning</code>).
        </p>
      </header>

      <section className="grid grid-cols-3 gap-4 mb-6">
        <Stat label="Today" value={`$${(stats.cost_cents_today / 100).toFixed(2)}`} />
        <Stat label="Last 30 days" value={`$${(stats.cost_cents_last_30d / 100).toFixed(2)}`} />
        <Stat label="Avg / day"
          value={`$${(stats.cost_cents_last_30d / 100 / 30).toFixed(2)}`} />
      </section>

      <section className="rounded-lg border border-slate-200 bg-white p-5">
        <h2 className="text-sm font-semibold text-slate-700 uppercase tracking-wide mb-3">
          Budget governance
        </h2>
        <ul className="space-y-2 text-sm">
          <li>
            <span className="text-slate-500">Hard cap :</span>{" "}
            <code>tenant_feature.m24_active_learning_budget_cents_per_day</code>
          </li>
          <li>
            <span className="text-slate-500">Reset :</span>{" "}
            00:00 UTC daily
          </li>
          <li>
            <span className="text-slate-500">Circuit breaker :</span>{" "}
            worker pauses if budget hits 100 % mid-batch
          </li>
        </ul>
      </section>

      <section className="mt-6 rounded-lg border border-slate-200 bg-white p-5">
        <h2 className="text-sm font-semibold text-slate-700 uppercase tracking-wide mb-3">
          Activity (last 30 days)
        </h2>
        <div className="grid grid-cols-2 gap-4 text-sm">
          <Detail label="Retune runs" value={stats.retune_runs_last_30d.toLocaleString()} />
          <Detail label="Promotions" value={stats.promotions_last_30d.toLocaleString()} />
          <Detail label="Total gold examples" value={stats.total_gold_examples.toLocaleString()} />
          <Detail label="Holdout examples" value={stats.total_holdout_examples.toLocaleString()} />
        </div>
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

function Detail({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="text-xs text-slate-500 uppercase tracking-wide">{label}</div>
      <div className="text-slate-900 tabular-nums">{value}</div>
    </div>
  );
}
