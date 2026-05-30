// SPDX-License-Identifier: UNLICENSED
import type { MissionState, OverlayKind } from "@/lib/reduce";

const TONE: Record<OverlayKind, string> = {
  fallback: "border-efe-cost text-efe-cost",
  circuit: "border-efe-danger text-efe-danger",
  noise: "border-efe-cost text-efe-cost",
  bypass: "border-efe-latency text-efe-latency",
  llm_failed: "border-efe-danger text-efe-danger",
};

const ICON: Record<OverlayKind, string> = {
  fallback: "↪",
  circuit: "⚡",
  noise: "✂",
  bypass: "⤳",
  llm_failed: "✕",
};

export function OverlayToasts({ state }: { state: MissionState }) {
  if (state.overlays.length === 0) return null;
  return (
    <div className="flex flex-col gap-1">
      {state.overlays.map((o, i) => (
        <div
          key={`${o.kind}-${i}`}
          className={`flex items-center gap-2 rounded-md border bg-efe-panel px-3 py-1.5 text-[12px] ${TONE[o.kind]}`}
        >
          <span aria-hidden>{ICON[o.kind]}</span>
          <span className="font-mono">{o.message}</span>
        </div>
      ))}
    </div>
  );
}
