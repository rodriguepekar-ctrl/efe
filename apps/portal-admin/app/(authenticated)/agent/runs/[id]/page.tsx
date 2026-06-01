// Single agent run detail page with live tail + approval dialog.

import { notFound } from "next/navigation";

import { fetchAgentRun } from "@/lib/agent/api";
import { STATUS_BADGE } from "@/lib/agent/grpc";

import LiveTail from "@/components/agent/LiveTail";
import ApprovalDialog from "@/components/agent/ApprovalDialog";
import CostMeter from "@/components/agent/CostMeter";

export const dynamic = "force-dynamic";

export default async function AgentRunDetailPage({
  params,
}: {
  params: { id: string };
}) {
  const detail = await fetchAgentRun(params.id);
  if (!detail) notFound();

  const { run, recent_steps, pending_approvals } = detail;

  return (
    <div className="p-6 space-y-6">
      <header>
        <h1 className="text-2xl font-semibold">
          Agent run {run.id.slice(0, 8)}…
        </h1>
        <div className="flex items-center gap-2 text-sm">
          <span className={`px-2 py-0.5 rounded ${STATUS_BADGE[run.status] ?? ""}`}>
            {run.status}
          </span>
          <span className="text-slate-600">
            {run.profile} · {run.provider}/{run.model}
          </span>
        </div>
      </header>

      <section className="grid grid-cols-2 md:grid-cols-4 gap-3">
        <div className="border rounded p-3 text-sm">
          <p className="text-slate-500 uppercase text-xs">Step</p>
          <p className="font-mono">{run.current_step}/{run.max_steps}</p>
        </div>
        <div className="border rounded p-3 text-sm">
          <p className="text-slate-500 uppercase text-xs">Cost</p>
          <CostMeter
            spentCents={run.current_cost_cents}
            capCents={run.max_cost_cents}
          />
        </div>
        <div className="border rounded p-3 text-sm">
          <p className="text-slate-500 uppercase text-xs">VM</p>
          <p className="font-mono text-xs">{run.vm_id ?? "—"}</p>
        </div>
        <div className="border rounded p-3 text-sm">
          <p className="text-slate-500 uppercase text-xs">Created</p>
          <p>{new Date(run.created_at).toLocaleString()}</p>
        </div>
      </section>

      {pending_approvals.length > 0 && (
        <section className="border border-orange-300 bg-orange-50 rounded p-4">
          <h2 className="font-semibold text-orange-800 mb-2">
            Pending approvals ({pending_approvals.length})
          </h2>
          {pending_approvals.map((a) => (
            <ApprovalDialog
              key={a.id}
              approvalId={a.id}
              stepIndex={a.step_index}
              toolName={a.tool_name}
              toolArgs={a.tool_args}
              justification={a.justification}
              expiresAt={a.expires_at}
            />
          ))}
        </section>
      )}

      <section>
        <h2 className="font-semibold mb-2">Steps (live)</h2>
        <LiveTail runId={run.id} initialSteps={recent_steps} />
      </section>
    </div>
  );
}
