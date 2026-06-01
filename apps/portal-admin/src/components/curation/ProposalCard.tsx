"use client";

import { useState } from "react";
import Link from "next/link";

import { ApprovalDialog } from "./ApprovalDialog";

type Role =
  | "viewer" | "curation_proposer" | "curation_reviewer"
  | "curation_promoter" | "taxonomy_admin" | "tenant_admin";

type Proposal = {
  id: string;
  catalog_id: string;
  class_id_primary?: string;
  kind: "SPLIT" | "MERGE" | "EXPLAIN" | "GENERATE_SYNTH" | "AUDIT_VS_GOLD";
  status: "PENDING" | "APPROVED" | "REJECTED" | "PROMOTED" | "ROLLED_BACK" | "EXPIRED";
  proposed_by: string;
  proposed_via: "mcp_tool" | "cli" | "portal" | "auto_drift";
  llm_provider?: string;
  llm_model?: string;
  cost_cents?: number;
  created_at: string;        // ISO
  confidence_score?: number;
  rationale_markdown: string;
  diff_summary?: { added: number; removed: number; classes_affected: number };
  severity?: "LOW" | "MEDIUM" | "HIGH";
};

const KIND_LABELS: Record<Proposal["kind"], { label: string; color: string }> = {
  SPLIT:          { label: "SPLIT",          color: "bg-blue-100 text-blue-800" },
  MERGE:          { label: "MERGE",          color: "bg-purple-100 text-purple-800" },
  EXPLAIN:        { label: "EXPLAIN",        color: "bg-slate-100 text-slate-800" },
  GENERATE_SYNTH: { label: "SYNTH",          color: "bg-emerald-100 text-emerald-800" },
  AUDIT_VS_GOLD:  { label: "AUDIT",          color: "bg-amber-100 text-amber-800" },
};

const SEVERITY_BADGE: Record<NonNullable<Proposal["severity"]>, string> = {
  LOW:    "bg-slate-100 text-slate-700",
  MEDIUM: "bg-amber-100 text-amber-800",
  HIGH:   "bg-red-100 text-red-800 animate-pulse",
};

export function ProposalCard({
  proposal, tenantSlug, userRole,
}: { proposal: Proposal; tenantSlug: string; userRole: Role }) {
  const [showApproval, setShowApproval] = useState<"approve" | "reject" | null>(null);
  const canReview = ["curation_reviewer", "taxonomy_admin", "tenant_admin"].includes(userRole);
  const ageStr = formatAge(proposal.created_at);
  const kindStyle = KIND_LABELS[proposal.kind];

  return (
    <article className="rounded-lg border border-slate-200 bg-white p-5 hover:shadow-md transition-shadow">
      <header className="flex items-start justify-between gap-4 mb-3">
        <div className="flex items-center gap-3">
          <span className={`px-2 py-0.5 rounded text-xs font-medium ${kindStyle.color}`}>
            {kindStyle.label}
          </span>
          {proposal.severity && (
            <span className={`px-2 py-0.5 rounded text-xs font-medium ${SEVERITY_BADGE[proposal.severity]}`}>
              {proposal.severity}
            </span>
          )}
          <code className="text-sm text-slate-500">#{proposal.id.slice(0, 8)}</code>
        </div>
        <div className="text-xs text-slate-500">{ageStr}</div>
      </header>

      <div className="space-y-2">
        <div className="text-sm">
          <span className="text-slate-500">catalog :</span>{" "}
          <code className="text-slate-900">{proposal.catalog_id}</code>
          {proposal.class_id_primary && (
            <>
              <span className="text-slate-500"> · class :</span>{" "}
              <code className="text-slate-900">{proposal.class_id_primary}</code>
            </>
          )}
        </div>

        <div className="text-sm text-slate-600">
          {proposal.proposed_via === "auto_drift" ? "Auto-triggered by drift" : `Proposed by ${proposal.proposed_by}`}
          {proposal.llm_provider && (
            <span className="ml-2">
              · {proposal.llm_provider} / {proposal.llm_model}
            </span>
          )}
          {typeof proposal.cost_cents === "number" && (
            <span className="ml-2">· ${(proposal.cost_cents / 100).toFixed(2)}</span>
          )}
        </div>

        {proposal.rationale_markdown && (
          <p className="text-sm text-slate-700 line-clamp-3 mt-2">
            <span className="text-slate-500">Rationale :</span>{" "}
            {proposal.rationale_markdown.slice(0, 500)}
            {proposal.rationale_markdown.length > 500 && "…"}
          </p>
        )}

        {proposal.diff_summary && (
          <div className="text-xs text-slate-500 mt-2 flex gap-3">
            <span>+ {proposal.diff_summary.added} anchors added</span>
            <span>− {proposal.diff_summary.removed} removed</span>
            <span>{proposal.diff_summary.classes_affected} classes affected</span>
          </div>
        )}
      </div>

      <footer className="mt-4 flex gap-2">
        <Link
          href={`/${tenantSlug}/curation/proposal/${proposal.id}`}
          className="px-3 py-1.5 rounded border border-slate-300 text-sm text-slate-700 hover:bg-slate-50"
        >
          View Full Diff
        </Link>
        {canReview && proposal.status === "PENDING" && (
          <>
            <button
              onClick={() => setShowApproval("approve")}
              className="px-3 py-1.5 rounded bg-emerald-600 text-white text-sm hover:bg-emerald-700"
            >
              ✓ Approve
            </button>
            <button
              onClick={() => setShowApproval("reject")}
              className="px-3 py-1.5 rounded border border-red-300 text-red-700 text-sm hover:bg-red-50"
            >
              ✗ Reject
            </button>
          </>
        )}
      </footer>

      {showApproval && (
        <ApprovalDialog
          proposalId={proposal.id}
          decision={showApproval}
          onClose={() => setShowApproval(null)}
        />
      )}
    </article>
  );
}

function formatAge(iso: string): string {
  const delta = Date.now() - new Date(iso).getTime();
  const min = Math.floor(delta / 60_000);
  if (min < 60) return `${min}m ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}h ago`;
  const day = Math.floor(hr / 24);
  return `${day}d ago`;
}
