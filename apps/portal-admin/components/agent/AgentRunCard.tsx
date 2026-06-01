"use client";

import Link from "next/link";

import type { AgentRun } from "@/lib/agent/api";
import { STATUS_BADGE } from "@/lib/agent/grpc";

import CostMeter from "./CostMeter";

interface Props {
  run: AgentRun;
}

export default function AgentRunCard({ run }: Props) {
  return (
    <Link
      href={`/agent/runs/${run.id}`}
      className="block border rounded p-4 hover:bg-slate-50 transition"
    >
      <header className="flex items-center justify-between mb-2">
        <div>
          <p className="font-mono text-xs text-slate-500">{run.id.slice(0, 8)}…</p>
          <h3 className="font-semibold text-sm">{run.template_slug}</h3>
        </div>
        <span className={`px-2 py-0.5 rounded text-xs ${STATUS_BADGE[run.status] ?? ""}`}>
          {run.status}
        </span>
      </header>
      <p className="text-xs text-slate-600">
        {run.profile} · {run.provider}/{run.model}
      </p>
      <p className="text-xs text-slate-600 mt-1">
        Step {run.current_step}/{run.max_steps}
      </p>
      <div className="mt-2">
        <CostMeter spentCents={run.current_cost_cents} capCents={run.max_cost_cents} />
      </div>
    </Link>
  );
}
