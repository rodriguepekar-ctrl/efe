"use client";

import { useState, useTransition } from "react";
import { acknowledgeDriftEvent } from "@/lib/curation/actions";

export function AcknowledgeDriftButton({ eventId }: { eventId: string }) {
  const [open, setOpen] = useState(false);
  const [note, setNote] = useState("");
  const [isPending, startTransition] = useTransition();
  const [error, setError] = useState<string | null>(null);

  const submit = () => {
    startTransition(async () => {
      try {
        await acknowledgeDriftEvent({ eventId, note: note.trim() || undefined });
        setOpen(false);
        window.location.reload();
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    });
  };

  if (!open) {
    return (
      <button
        onClick={() => setOpen(true)}
        className="text-xs px-2 py-1 rounded border border-slate-300 hover:bg-slate-50"
      >
        Acknowledge
      </button>
    );
  }

  return (
    <div className="inline-flex flex-col gap-1 items-end">
      <input
        type="text"
        value={note}
        onChange={(e) => setNote(e.target.value)}
        placeholder="Note (optional)"
        className="text-xs rounded border border-slate-300 px-2 py-1 w-40"
        autoFocus
      />
      <div className="flex gap-1">
        <button
          onClick={() => setOpen(false)}
          disabled={isPending}
          className="text-xs px-2 py-0.5 rounded border border-slate-300 hover:bg-slate-50"
        >Cancel</button>
        <button
          onClick={submit}
          disabled={isPending}
          className="text-xs px-2 py-0.5 rounded bg-emerald-600 text-white hover:bg-emerald-700"
        >
          {isPending ? "..." : "Confirm"}
        </button>
      </div>
      {error && <div className="text-xs text-red-600">{error}</div>}
    </div>
  );
}
