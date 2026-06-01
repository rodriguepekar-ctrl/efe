// Active runs list (M25).

import Link from "next/link";

import { fetchAgentRuns } from "@/lib/agent/api";
import { getRbacUser } from "@/lib/auth/session";
import { STATUS_BADGE } from "@/lib/agent/grpc";

export const dynamic = "force-dynamic";

export default async function AgentRunsPage() {
  const user = await getRbacUser();
  if (!user) return <p className="p-6">Sign in to view this page.</p>;
  const runs = await fetchAgentRuns({ limit: 100 });

  return (
    <div className="p-6 space-y-4">
      <header className="flex items-center justify-between">
        <h1 className="text-2xl font-semibold">Agent runs</h1>
        <Link
          href="/agent/runs/new"
          className="rounded bg-emerald-600 px-3 py-1 text-sm text-white hover:bg-emerald-700"
        >
          New run
        </Link>
      </header>

      {runs.length === 0 ? (
        <p className="text-sm text-slate-600">No active runs.</p>
      ) : (
        <table className="w-full border-collapse text-sm">
          <thead>
            <tr className="text-left">
              <th className="py-2 border-b">ID</th>
              <th className="py-2 border-b">Status</th>
              <th className="py-2 border-b">Template</th>
              <th className="py-2 border-b">Profile</th>
              <th className="py-2 border-b">Step</th>
              <th className="py-2 border-b">Cost</th>
              <th className="py-2 border-b">Created</th>
            </tr>
          </thead>
          <tbody>
            {runs.map((r) => (
              <tr key={r.id} className="hover:bg-slate-50">
                <td className="py-2 border-b font-mono text-xs">
                  <Link href={`/agent/runs/${r.id}`} className="underline">
                    {r.id.slice(0, 8)}…
                  </Link>
                </td>
                <td className="py-2 border-b">
                  <span className={`px-2 py-0.5 rounded text-xs ${STATUS_BADGE[r.status] ?? ""}`}>
                    {r.status}
                  </span>
                </td>
                <td className="py-2 border-b">{r.template_id}</td>
                <td className="py-2 border-b">{r.profile}</td>
                <td className="py-2 border-b">
                  {r.current_step}/{r.max_steps}
                </td>
                <td className="py-2 border-b">
                  ${(r.current_cost_cents / 100).toFixed(2)} / ${(r.max_cost_cents / 100).toFixed(2)}
                </td>
                <td className="py-2 border-b text-xs">
                  {new Date(r.created_at).toLocaleString()}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
