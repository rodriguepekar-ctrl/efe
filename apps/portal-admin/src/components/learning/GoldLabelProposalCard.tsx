"use client";

import { useState } from "react";
import { ConfirmEditDialog } from "./ConfirmEditDialog";

type Role =
  | "viewer" | "curation_proposer" | "curation_reviewer"
  | "curation_promoter" | "taxonomy_admin" | "tenant_admin";

type QueueItem = {
  id: string;
  catalog_id: string;
  status: string;
  proposed_class_id?: string;
  observed_confidence?: number;
  enqueued_at: string;
  failure_count?: number;
  llm_proposed_class_id?: string;
  llm_confidence?: number;
  llm_rationale?: string;
};

const STATUS_COLORS: Record<string, string> = {
  ENQUEUED:        "bg-slate-100 text-slate-700",
  LABELING:        "bg-blue-100 text-blue-800",
  LLM_LABELED:     "bg-emerald-100 text-emerald-800",
  AWAITING_REVIEW: "bg-amber-100 text-amber-900",
  CONFIRMED:       "bg-green-100 text-green-800",
  REJECTED:        "bg-red-100 text-red-800",
  FAILED:          "bg-red-100 text-red-900 animate-pulse",
};

export function GoldLabelProposalCard({
  item, tenantSlug, userRole,
}: { item: QueueItem; tenantSlug: string; userRole: Role }) {
  const [dialog, setDialog] = useState<"confirm" | "edit" | "reject" | null>(null);
  const canReview = ["curation_reviewer", "taxonomy_admin", "tenant_admin"].includes(userRole);
  const ageMin = Math.floor((Date.now() - new Date(item.enqueued_at).getTime()) / 60000);
  const isReviewable = item.status === "LLM_LABELED" || item.status === "AWAITING_REVIEW";

  return (
    <article className="rounded-lg border border-slate-200 bg-white p-4 hover:shadow-sm transition-shadow">
      <header className="flex items-start justify-between gap-4 mb-2">
        <div className="flex items-center gap-2 flex-wrap">
          <span className={`px-2 py-0.5 rounded text-xs font-medium ${STATUS_COLORS[item.status] ?? "bg-slate-100 text-slate-700"}`}>
            {item.status}
          </span>
          <code className="text-xs text-slate-500">#{item.id.slice(0, 8)}</code>
          <span className="text-xs text-slate-500">·</span>
          <code className="text-xs">{item.catalog_id}</code>
        </div>
        <div className="text-xs text-slate-500 whitespace-nowrap">
          {ageMin < 60 ? `${ageMin}m ago` : `${Math.floor(ageMin / 60)}h ago`}
        </div>
      </header>

      <div className="space-y-1 text-sm">
        {item.observed_confidence !== undefined && (
          <div className="text-slate-600">
            <span className="text-slate-500">prod conf :</span>{" "}
            <span className="tabular-nums">{item.observed_confidence.toFixed(2)}</span>
            {item.proposed_class_id && (
              <>
                <span className="text-slate-500"> · prod predicted :</span>{" "}
                <code>{item.proposed_class_id}</code>
              </>
            )}
          </div>
        )}
        {item.llm_proposed_class_id && (
          <div className="text-emerald-700">
            <span className="text-slate-500">LLM proposed :</span>{" "}
            <code className="text-emerald-700 font-medium">{item.llm_proposed_class_id}</code>
            {item.llm_confidence !== undefined && (
              <span className="text-slate-500 ml-2">
                @ {item.llm_confidence.toFixed(2)} confidence
              </span>
            )}
          </div>
        )}
        {item.llm_rationale && (
          <div className="text-slate-600 italic line-clamp-2">{item.llm_rationale}</div>
        )}
        {item.failure_count && item.failure_count > 0 && (
          <div className="text-red-700 text-xs">
            ⚠ {item.failure_count} failures
          </div>
        )}
      </div>

      {canReview && isReviewable && (
        <footer className="mt-3 flex gap-2">
          <button
            onClick={() => setDialog("confirm")}
            className="px-3 py-1 rounded bg-emerald-600 text-white text-xs hover:bg-emerald-700"
          >
            ✓ Confirm
          </button>
          <button
            onClick={() => setDialog("edit")}
            className="px-3 py-1 rounded border border-slate-300 text-slate-700 text-xs hover:bg-slate-50"
          >
            ✎ Edit class
          </button>
          <button
            onClick={() => setDialog("reject")}
            className="px-3 py-1 rounded border border-red-300 text-red-700 text-xs hover:bg-red-50"
          >
            ✗ Reject
          </button>
        </footer>
      )}

      {dialog && (
        <ConfirmEditDialog
          itemId={item.id}
          mode={dialog}
          currentClass={item.llm_proposed_class_id ?? item.proposed_class_id ?? ""}
          onClose={() => setDialog(null)}
        />
      )}
    </article>
  );
}
