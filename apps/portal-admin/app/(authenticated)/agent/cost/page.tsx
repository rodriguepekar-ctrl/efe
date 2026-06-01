// Cost dashboard (M25).

import { fetchAgentCostDaily } from "@/lib/agent/api";
import { getRbacUser } from "@/lib/auth/session";

export const dynamic = "force-dynamic";

export default async function AgentCostPage() {
  const user = await getRbacUser();
  if (!user) return <p className="p-6">Sign in.</p>;
  const rows = await fetchAgentCostDaily({ days: 30 });
  const total = rows.reduce((acc: number, r: any) => acc + r.cents, 0);
  return (
    <div className="p-6 space-y-4">
      <header>
        <h1 className="text-2xl font-semibold">Agent cost</h1>
        <p className="text-sm text-slate-600">
          Last 30 days. Total : ${(total / 100).toFixed(2)}.
        </p>
      </header>
      {rows.length === 0 ? (
        <p className="text-sm text-slate-600">No cost data yet.</p>
      ) : (
        <table className="w-full text-sm border-collapse">
          <thead>
            <tr className="text-left">
              <th className="py-2 border-b">Day</th>
              <th className="py-2 border-b">Runs</th>
              <th className="py-2 border-b">Cost</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r: any) => (
              <tr key={r.day}>
                <td className="py-2 border-b">{r.day}</td>
                <td className="py-2 border-b">{r.runs}</td>
                <td className="py-2 border-b">${(r.cents / 100).toFixed(2)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
