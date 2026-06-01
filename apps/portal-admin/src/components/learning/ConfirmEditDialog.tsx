"use client";

import { useState, useTransition } from "react";
import { confirmGoldLabel, rejectGoldLabel, editAndConfirmGoldLabel } from "@/lib/learning/actions";

type Props = {
  itemId: string;
  mode: "confirm" | "edit" | "reject";
  currentClass: string;
  onClose: () => void;
};

const MIN_RATIONALE = 10;

export function ConfirmEditDialog({ itemId, mode, currentClass, onClose }: Props) {
  const [rationale, setRationale] = useState("");
  const [editClass, setEditClass] = useState(currentClass);
  const [error, setError] = useState<string | null>(null);
  const [isPending, startTransition] = useTransition();

  const valid = rationale.trim().length >= MIN_RATIONALE
    && (mode !== "edit" || editClass.trim().length >= 2);

  const submit = () => {
    if (!valid) {
      setError(`Rationale ≥ ${MIN_RATIONALE} chars required`
        + (mode === "edit" ? " ; class id required" : ""));
      return;
    }
    startTransition(async () => {
      try {
        if (mode === "confirm") {
          await confirmGoldLabel({ proposalId: itemId, rationale: rationale.trim() });
        } else if (mode === "edit") {
          await editAndConfirmGoldLabel({
            proposalId: itemId, finalClassId: editClass.trim(), rationale: rationale.trim(),
          });
        } else {
          await rejectGoldLabel({ proposalId: itemId, rationale: rationale.trim() });
        }
        onClose();
        window.location.reload();
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    });
  };

  const title = mode === "confirm" ? "Confirm gold label"
              : mode === "edit"    ? "Edit and confirm gold label"
                                   : "Reject gold label";

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-slate-900/50" role="dialog" aria-modal="true">
      <div className="bg-white rounded-lg shadow-xl max-w-lg w-full p-6">
        <h2 className="text-lg font-semibold text-slate-900 mb-1">{title}</h2>
        <p className="text-xs text-slate-600 mb-4">
          <code>#{itemId.slice(0, 12)}</code> · current : <code>{currentClass}</code>
        </p>

        {mode === "edit" && (
          <div className="mb-4">
            <label className="block text-sm font-medium text-slate-700 mb-1">
              Final class ID
            </label>
            <input
              type="text"
              value={editClass}
              onChange={(e) => setEditClass(e.target.value)}
              className="w-full rounded border border-slate-300 px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
              placeholder="kebab-case-id"
            />
          </div>
        )}

        <label className="block text-sm font-medium text-slate-700 mb-1">
          Rationale (required, ≥ {MIN_RATIONALE} chars)
        </label>
        <textarea
          value={rationale}
          onChange={(e) => { setRationale(e.target.value); setError(null); }}
          rows={4}
          className="w-full rounded border border-slate-300 px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-blue-500"
          placeholder="Explain your decision…"
          autoFocus
        />
        <div className="text-xs text-slate-500 mt-1">
          {rationale.trim().length} / {MIN_RATIONALE} chars min
        </div>

        {error && (
          <div className="mt-3 rounded bg-red-50 border border-red-200 p-3 text-sm text-red-800">
            {error}
          </div>
        )}

        <div className="mt-4 rounded bg-amber-50 border border-amber-200 p-3 text-xs text-amber-900">
          ⚠ Logged in the audit chain (HMAC). Cannot be undone.
        </div>

        <div className="mt-5 flex gap-2 justify-end">
          <button onClick={onClose} disabled={isPending}
            className="px-4 py-2 rounded border border-slate-300 text-sm hover:bg-slate-50 disabled:opacity-50">
            Cancel
          </button>
          <button onClick={submit} disabled={isPending || !valid}
            className={`px-4 py-2 rounded text-sm text-white disabled:opacity-50 ${
              mode === "reject" ? "bg-red-600 hover:bg-red-700" : "bg-emerald-600 hover:bg-emerald-700"
            }`}>
            {isPending ? "Submitting…" : `${title.split(" ")[0]}`}
          </button>
        </div>
      </div>
    </div>
  );
}
