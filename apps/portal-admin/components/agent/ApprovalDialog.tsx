"use client";

import { useState, useTransition } from "react";

import { approveAgentStep, rejectAgentStep } from "@/lib/agent/actions";

interface Props {
  approvalId: string;
  stepIndex: number;
  toolName: string;
  toolArgs: unknown;
  justification: string | null;
  expiresAt: string;
}

export default function ApprovalDialog(props: Props) {
  const [reason, setReason] = useState("");
  const [open, setOpen] = useState(false);
  const [isPending, startTransition] = useTransition();
  const [error, setError] = useState<string | null>(null);

  const onApprove = () => {
    if (reason.trim().length < 10) {
      setError("rationale must be ≥ 10 chars");
      return;
    }
    setError(null);
    startTransition(async () => {
      try {
        await approveAgentStep({ approvalId: props.approvalId, reason });
        setOpen(false);
      } catch (e) {
        setError(String(e));
      }
    });
  };

  const onReject = () => {
    if (reason.trim().length < 10) {
      setError("rationale must be ≥ 10 chars");
      return;
    }
    setError(null);
    startTransition(async () => {
      try {
        await rejectAgentStep({ approvalId: props.approvalId, reason });
        setOpen(false);
      } catch (e) {
        setError(String(e));
      }
    });
  };

  return (
    <div className="border-t pt-2 mt-2 first:border-0 first:pt-0 first:mt-0">
      <div className="flex items-center justify-between">
        <div>
          <p className="text-sm font-semibold">
            Step {props.stepIndex} : <code>{props.toolName}</code>
          </p>
          {props.justification && (
            <p className="text-xs text-orange-700 mt-1">
              <strong>Agent justification :</strong> {props.justification}
            </p>
          )}
          <p className="text-xs text-slate-500">
            Expires {new Date(props.expiresAt).toLocaleString()}
          </p>
        </div>
        <button
          onClick={() => setOpen((o) => !o)}
          className="text-xs underline text-orange-800"
        >
          {open ? "Cancel" : "Review"}
        </button>
      </div>

      {open && (
        <div className="mt-2 space-y-2">
          <details className="text-xs">
            <summary>Tool arguments</summary>
            <pre className="bg-slate-100 p-2 rounded text-xs overflow-x-auto">
              {JSON.stringify(props.toolArgs, null, 2)}
            </pre>
          </details>
          <textarea
            value={reason}
            onChange={(e) => setReason(e.target.value)}
            rows={3}
            placeholder="Rationale (≥ 10 chars, audited)"
            className="w-full border rounded p-2 text-sm"
          />
          {error && <p className="text-xs text-red-700">{error}</p>}
          <div className="flex gap-2">
            <button
              onClick={onApprove}
              disabled={isPending}
              className="px-3 py-1 bg-emerald-600 text-white rounded text-sm hover:bg-emerald-700 disabled:opacity-50"
            >
              Approve
            </button>
            <button
              onClick={onReject}
              disabled={isPending}
              className="px-3 py-1 bg-red-600 text-white rounded text-sm hover:bg-red-700 disabled:opacity-50"
            >
              Reject
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
