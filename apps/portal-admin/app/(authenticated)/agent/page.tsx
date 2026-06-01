// Agent runtime overview page (M25).

import Link from "next/link";

import { fetchAgentDpia } from "@/lib/agent/api";
import { getRbacUser } from "@/lib/auth/session";

export const dynamic = "force-dynamic";

export default async function AgentOverviewPage() {
  const user = await getRbacUser();
  if (!user) return <p className="p-6">Sign in to view this page.</p>;

  const dpia = await fetchAgentDpia();

  return (
    <div className="p-6 space-y-6">
      <header>
        <h1 className="text-2xl font-semibold">Agent runtime (M25)</h1>
        <p className="text-sm text-slate-600">
          EXPERIMENTAL : Firecracker-sandboxed LLM-driven agents,
          DPIA-gated, cost-capped, step-approved.
        </p>
      </header>

      {!dpia.is_valid && (
        <div className="border border-orange-300 bg-orange-50 rounded p-4">
          <h2 className="font-semibold text-orange-800">DPIA required</h2>
          <p className="text-sm text-orange-700">
            Before creating agent runs, your DPO must sign the M25 DPIA
            addendum. Status :{" "}
            {dpia.signed_at ? `expired ${dpia.expires_at}` : "not signed"}.
          </p>
          <Link
            href="/agent/dpia"
            className="mt-2 inline-block text-orange-800 underline"
          >
            → Open DPIA workflow
          </Link>
        </div>
      )}

      <nav className="grid grid-cols-2 md:grid-cols-4 gap-3">
        <Link href="/agent/runs" className="block border rounded p-4 hover:bg-slate-50">
          <h3 className="font-semibold">Runs</h3>
          <p className="text-xs text-slate-600">Active &amp; recent runs</p>
        </Link>
        <Link href="/agent/templates" className="block border rounded p-4 hover:bg-slate-50">
          <h3 className="font-semibold">Templates</h3>
          <p className="text-xs text-slate-600">AgentRunTemplate definitions</p>
        </Link>
        <Link href="/agent/cost" className="block border rounded p-4 hover:bg-slate-50">
          <h3 className="font-semibold">Cost</h3>
          <p className="text-xs text-slate-600">Per-run + daily totals</p>
        </Link>
        <Link href="/agent/dpia" className="block border rounded p-4 hover:bg-slate-50">
          <h3 className="font-semibold">DPIA</h3>
          <p className="text-xs text-slate-600">Compliance + addendum</p>
        </Link>
      </nav>
    </div>
  );
}
