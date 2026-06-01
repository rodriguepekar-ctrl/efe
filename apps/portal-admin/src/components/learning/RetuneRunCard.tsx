"use client";

import { useState, useTransition } from "react";
import Link from "next/link";
import { promoteRetuneRun, rollbackRetune } from "@/lib/learning/actions";

type Run = {
  id: string;
  catalog_id: string;
  new_bundle_semver: string;
  training_examples_count: number;
  holdout_examples_count: number;
  status: string;
  flagged_for_promotion: boolean;
  started_at: string;
  completed_at?: string;
};

type Role =
  | "viewer" | "curation_proposer" | "curation_reviewer"
  | "curation_promoter" | "taxonomy_admin" | "tenant_admin";

const STATUS_COLORS: Record<string, string> = {
  TUNING:       "bg-blue-100 text-blue-800",
  EVAL_RUNNING: "bg-blue-200 text-blue-900",
  EVAL_PASS:    "bg-emerald-100 text-emerald-800",
  EVAL_FAIL:    "bg-slate-100 text-slate-700",
  PROMOTED:     "bg-green-200 text-green-900",
  ROLLED_BACK:  "bg-amber-100 text-amber-900",
  ABORTED:      "bg-red-100 text-red-900",
};

export function RetuneRunCard({
  run, tenantSlug, userRole,
}: { run: Run; tenantSlug: string; userRole: Role }) {
  const [error, setError] = useState<string | null>(null);
  const [isPending, startTransition] = useTransition();
  const canPromote = ["curation_promoter", "taxonomy_admin", "tenant_admin"].includes(userRole);
  const canPromoteThisRun = canPromote && run.status === "EVAL_PASS" && run.flagged_for_promotion;

  const onPromote = () => {
    startTransition(async () => {
      try {
        await promoteRetuneRun({ runId: run.id, skipCanary: false });
        window.location.reload();
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    });
  };

  const onRollback = () => {
    const reason = window.prompt("Rollback reason (≥ 10 chars) :");
    if (!reason || reason.trim().length < 10) {
      setError("rationale ≥ 10 chars required");
      return;
    }
    startTransition(async () => {
      try {
        await rollbackRetune({ runId: run.id, reason });
        window.location.reload();
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    });
  };

  return (
    <article className="rounded-lg border border-slate-200 bg-white p-4">
      <header className="flex items-start justify-between gap-4 mb-2">
        <div className="flex items-center gap-2 flex-wrap">
          <span className={`px-2 py-0.5 rounded text-xs font-medium ${STATUS_COLORS[run.status] ?? "bg-slate-100"}`}>
            {run.status}
          </span>
          {run.flagged_for_promotion && (
            <span className="px-2 py-0.5 rounded text-xs font-medium bg-amber-100 text-amber-900">
              ★ flagged for promotion
            </span>
          )}
          <code className="text-xs text-slate-500">#{run.id.slice(0, 8)}</code>
          <span className="text-xs text-slate-500">·</span>
          <code className="text-xs">{run.catalog_id}</code>
          <code className="text-xs ml-2">v{run.new_bundle_semver}</code>
        </div>
      </header>

      <div className="grid grid-cols-2 md:grid-cols-4 gap-3 text-sm mb-3">
        <Stat label="Training" value={run.training_examples_count.toLocaleString()} />
        <Stat label="Holdout" value={run.holdout_examples_count.toLocaleString()} />
        <Stat label="Started" value={new Date(run.started_at).toLocaleString()} />
        <Stat label="Completed" value={run.completed_at ? new Date(run.completed_at).toLocaleString() : "in progress"} />
      </div>

      <div className="flex gap-2 items-center">
        <Link href={`/${tenantSlug}/learning/retunes/${run.id}`}
          className="px-3 py-1 rounded border border-slate-300 text-xs text-slate-700 hover:bg-slate-50">
          View detail
        </Link>
        {canPromoteThisRun && (
          <button onClick={onPromote} disabled={isPending}
            className="px-3 py-1 rounded bg-emerald-600 text-white text-xs hover:bg-emerald-700 disabled:opacity-50">
            {isPending ? "..." : "✓ Promote"}
          </button>
        )}
        {canPromote && run.status === "PROMOTED" && (
          <button onClick={onRollback} disabled={isPending}
            className="px-3 py-1 rounded border border-amber-300 text-amber-800 text-xs hover:bg-amber-50 disabled:opacity-50">
            ↺ Rollback
          </button>
        )}
        {error && <span className="text-xs text-red-600">{error}</span>}
      </div>
    </article>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <div className="text-xs text-slate-500 uppercase tracking-wide">{label}</div>
      <div className="text-slate-900 tabular-nums truncate">{value}</div>
    </div>
  );
}
