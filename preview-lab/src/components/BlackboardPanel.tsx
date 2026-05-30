// SPDX-License-Identifier: UNLICENSED
import type { MissionState } from "@/lib/reduce";

export function BlackboardPanel({ state }: { state: MissionState }) {
  const items = state.pheromones;
  return (
    <div className="flex flex-col gap-2 rounded-xl border border-efe-border bg-efe-panel p-4">
      <h3 className="text-sm font-semibold text-efe-text">Blackboard</h3>
      {items.length === 0 ? (
        <p className="text-[11px] text-efe-muted">Aucune phéromone déposée.</p>
      ) : (
        <ul className="flex max-h-48 flex-col gap-1 overflow-auto">
          {items.map((p) => (
            <li
              key={p.entry_id}
              className="flex items-center justify-between gap-2 rounded-md border border-efe-border/60 bg-efe-bg/50 px-2 py-1"
            >
              <span className="font-mono text-[11px] text-efe-accent">{p.kind}</span>
              <span className="truncate font-mono text-[11px] text-efe-muted">{p.author}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
