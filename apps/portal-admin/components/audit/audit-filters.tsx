"use client";

import { useRouter, useSearchParams } from "next/navigation";

export function AuditFilters() {
  const router = useRouter();
  const sp = useSearchParams();

  const updateFilter = (key: string, value: string | null) => {
    const next = new URLSearchParams(sp);
    if (value) next.set(key, value);
    else next.delete(key);
    next.delete("cursor"); // reset pagination
    router.push(`/audit?${next.toString()}`);
  };

  return (
    <search aria-label="Audit filters">
      <form className="flex flex-wrap items-center gap-2 rounded-md border bg-muted/30 p-3 text-sm">
        <label className="flex items-center gap-1">
          Mode:
          <select
            defaultValue={sp.get("mode") ?? ""}
            onChange={(e) => updateFilter("mode", e.target.value || null)}
            className="rounded border bg-background px-2 py-1"
          >
            <option value="">all</option>
            <option value="MODE_2">Mode-2</option>
            <option value="MODE_AUTO_ESC">Mode-Auto-Esc</option>
            <option value="MODE_3">Mode-3</option>
          </select>
        </label>
        <label className="flex items-center gap-1">
          Since:
          <input
            type="datetime-local"
            defaultValue={sp.get("since") ?? ""}
            onChange={(e) =>
              updateFilter("since", e.target.value ? new Date(e.target.value).toISOString() : null)
            }
            className="rounded border bg-background px-2 py-1"
          />
        </label>
      </form>
    </search>
  );
}
