// SPDX-License-Identifier: UNLICENSED
import type { RobotStatus } from "@/lib/reduce";

const STYLES: Record<RobotStatus, string> = {
  idle: "border-efe-border text-efe-muted",
  running: "border-efe-latency text-efe-text animate-pulseNode",
  done: "border-efe-accent text-efe-text",
  failed: "border-efe-danger text-efe-text",
};

const DOT: Record<RobotStatus, string> = {
  idle: "bg-efe-muted/40",
  running: "bg-efe-latency",
  done: "bg-efe-accent",
  failed: "bg-efe-danger",
};

export function RobotNode({
  label,
  sublabel,
  status,
  latencyMs,
}: {
  label: string;
  sublabel?: string;
  status: RobotStatus;
  latencyMs?: number;
}) {
  return (
    <div
      className={`flex min-w-[7rem] flex-col gap-1 rounded-lg border bg-efe-panel px-3 py-2 ${STYLES[status]}`}
    >
      <div className="flex items-center gap-2">
        <span className={`h-2 w-2 rounded-full ${DOT[status]}`} />
        <span className="text-sm font-semibold">{label}</span>
      </div>
      <div className="text-[11px] text-efe-muted">
        {sublabel ?? statusLabel(status)}
      </div>
      <div className="font-mono text-[11px] text-efe-muted">
        {typeof latencyMs === "number" ? `${latencyMs} ms` : "\u00a0"}
      </div>
    </div>
  );
}

function statusLabel(s: RobotStatus): string {
  switch (s) {
    case "idle":
      return "en attente";
    case "running":
      return "en cours…";
    case "done":
      return "terminé";
    case "failed":
      return "échec";
  }
}
