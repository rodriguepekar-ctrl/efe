"use client";

import { useState, useTransition } from "react";
import { approveProposal, rejectProposal } from "@/lib/curation/actions";

type Props = {
  proposalId: string;
  decision: "approve" | "reject";
  onClose: () => void;
};

export function ApprovalDialog({ proposalId, decision, onClose }: Props) {
  const [rationale, setRationale] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [isPending, startTransition] = useTransition();

  const minLen = 10;
  const valid = rationale.trim().length >= minLen;

  const submit = () => {
    if (!valid) {
      setError(`Rationale must be at least ${minLen} characters (Art. 14 audit requirement).`);
      return;
    }
    startTransition(async () => {
      try {
        // Re-acquire fresh OIDC ID token via prompt=none before submission.
        // (Implementation in @/lib/auth/refresh ; throws if user signed out.)
        const action = decision === "approve" ? approveProposal : rejectProposal;
        await action({ proposalId, rationale: rationale.trim() });
        onClose();
        // Refresh server data on parent.
        window.location.reload();
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    });
  };

  const isApprove = decision === "approve";

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-slate-900/50" role="dialog" aria-modal="true">
      <div className="bg-white rounded-lg shadow-xl max-w-lg w-full p-6">
        <h2 className="text-lg font-semibold text-slate-900 mb-1">
          {isApprove ? "Approve" : "Reject"} Proposal
        </h2>
        <p className="text-sm text-slate-600 mb-4">
          <code className="text-xs">{proposalId}</code>
        </p>

        <label className="block text-sm font-medium text-slate-700 mb-1">
          Rationale (required, ≥ {minLen} chars)
        </label>
        <textarea
          value={rationale}
          onChange={(e) => { setRationale(e.target.value); setError(null); }}
          rows={4}
          className="w-full rounded border border-slate-300 px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
          placeholder={isApprove
            ? "Explain why you approve this change…"
            : "Explain why you reject this change…"}
          autoFocus
        />
        <div className="text-xs text-slate-500 mt-1">
          {rationale.trim().length} / {minLen} chars min
        </div>

        {error && (
          <div className="mt-3 rounded bg-red-50 border border-red-200 p-3 text-sm text-red-800">
            {error}
          </div>
        )}

        <div className="mt-4 rounded bg-amber-50 border border-amber-200 p-3 text-xs text-amber-900">
          ⚠ This action will be cryptographically logged in the audit chain.
          The decision cannot be undone (a separate rollback action is required).
        </div>

        <div className="mt-5 flex gap-2 justify-end">
          <button
            onClick={onClose}
            disabled={isPending}
            className="px-4 py-2 rounded border border-slate-300 text-sm text-slate-700 hover:bg-slate-50 disabled:opacity-50"
          >
            Cancel
          </button>
          <button
            onClick={submit}
            disabled={isPending || !valid}
            className={`px-4 py-2 rounded text-sm text-white disabled:opacity-50 ${
              isApprove ? "bg-emerald-600 hover:bg-emerald-700" : "bg-red-600 hover:bg-red-700"
            }`}
          >
            {isPending ? "Submitting…" : (isApprove ? "✓ Confirm Approval" : "✗ Confirm Rejection")}
          </button>
        </div>
      </div>
    </div>
  );
}
