// portal-admin curation drift events page — M23.

import Link from "next/link";
import { fetchDriftEvents } from "@/lib/curation/api";
import { requireFeature, requireRole } from "@/lib/auth/guards";
import { AcknowledgeDriftButton } from "@/components/curation/AcknowledgeDriftButton";

export const metadata = { title: "Drift events · EFE Portal" };

const SEV_COLORS: Record<string, string> = {
  HIGH:   "bg-red-100 text-red-800",
  MEDIUM: "bg-amber-100 text-amber-800",
  LOW:    "bg-slate-100 text-slate-700",
};

export default async function DriftPage({
  params, searchParams,
}: {
  params: { tenant_slug: string };
  searchParams: { only_open?: string; min_severity?: string };
}) {
  await requireFeature(params.tenant_slug, "m23_drift_detection_enabled");
  const session = await requireRole(params.tenant_slug, [
    "viewer", "curation_reviewer", "curation_promoter", "taxonomy_admin", "tenant_admin",
  ]);

  const events = await fetchDriftEvents({
    tenantSlug: params.tenant_slug,
    onlyOpen: searchParams.only_open !== "false",
    minSeverity: searchParams.min_severity ?? "LOW",
  });

  return (
    <main className="px-6 py-8 max-w-7xl mx-auto">
      <header className="mb-6">
        <h1 className="text-2xl font-semibold text-slate-900">Drift events</h1>
        <p className="text-sm text-slate-600 mt-1">
          Statistical drift detected on classification confidence distributions
          (KL divergence ≥ tenant threshold).
        </p>
      </header>

      <section className="rounded-lg border border-slate-200 bg-white overflow-hidden">
        <table className="w-full text-sm">
          <thead className="bg-slate-50 text-left text-xs text-slate-500 uppercase">
            <tr>
              <th className="py-2 px-3">Severity</th>
              <th className="py-2 px-3">Catalog / Class</th>
              <th className="py-2 px-3 text-right">KL</th>
              <th className="py-2 px-3 text-right">Conf Δ</th>
              <th className="py-2 px-3">Detected</th>
              <th className="py-2 px-3">Status</th>
              <th className="py-2 px-3"></th>
            </tr>
          </thead>
          <tbody>
            {events.map((e) => (
              <tr key={e.id} className="border-t border-slate-100">
                <td className="py-2 px-3">
                  <span className={`px-2 py-0.5 rounded text-xs font-medium ${SEV_COLORS[e.severity]}`}>
                    {e.severity}
                  </span>
                </td>
                <td className="py-2 px-3">
                  <code className="text-xs">{e.catalog_id} / {e.class_id}</code>
                </td>
                <td className="py-2 px-3 text-right tabular-nums">{e.kl_divergence.toFixed(3)}</td>
                <td className="py-2 px-3 text-right tabular-nums">
                  {(e.mean_confidence_old - e.mean_confidence_new).toFixed(3)}
                </td>
                <td className="py-2 px-3 text-slate-500 text-xs">
                  {new Date(e.detected_at).toLocaleString()}
                </td>
                <td className="py-2 px-3">
                  {e.acknowledged_at ? (
                    <span className="text-emerald-700 text-xs">acknowledged</span>
                  ) : (
                    <span className="text-amber-700 text-xs">open</span>
                  )}
                </td>
                <td className="py-2 px-3 text-right">
                  {e.triggered_proposal_id ? (
                    <Link
                      className="text-xs text-blue-600 hover:underline"
                      href={`/${params.tenant_slug}/curation/proposal/${e.triggered_proposal_id}`}
                    >
                      View proposal →
                    </Link>
                  ) : !e.acknowledged_at ? (
                    <AcknowledgeDriftButton eventId={e.id} />
                  ) : null}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {events.length === 0 && (
          <div className="py-12 text-center text-sm text-slate-500">
            No drift events match the current filter.
          </div>
        )}
      </section>
    </main>
  );
}
