// DPIA workflow page (M25).

import { fetchAgentDpia, getActiveTenantSlug } from "@/lib/agent/api";
import { getRbacUser } from "@/lib/auth/session";

export const dynamic = "force-dynamic";

export default async function AgentDpiaPage() {
  const user = await getRbacUser();
  if (!user) return <p className="p-6">Sign in.</p>;
  const tenant = (await getActiveTenantSlug()) ?? "<tenant>";
  const dpia = await fetchAgentDpia();

  return (
    <div className="p-6 space-y-4">
      <header>
        <h1 className="text-2xl font-semibold">DPIA — Agent runtime</h1>
        <p className="text-sm text-slate-600">
          The M25 agent runtime is a high-risk autonomous AI system
          (EU AI Act Art. 9). A DPIA addendum must be signed by your
          DPO before any agent run can be created. Re-signing required
          every 12 months.
        </p>
      </header>

      <section className="border rounded p-4 space-y-2">
        <h2 className="font-semibold">Current status</h2>
        <p className="text-sm">
          <strong>Valid :</strong> {dpia.is_valid ? "yes" : "no"}<br />
          <strong>Signed at :</strong> {dpia.signed_at ?? "(never)"}<br />
          <strong>Expires at :</strong> {dpia.expires_at ?? "(n/a)"}<br />
          <strong>Addendum hash :</strong>{" "}
          <code className="text-xs">{dpia.addendum_hash ?? "(none)"}</code>
        </p>
      </section>

      <section className="border rounded p-4 space-y-2">
        <h2 className="font-semibold">How to sign</h2>
        <ol className="text-sm list-decimal pl-5 space-y-1">
          <li>
            Download the template :{" "}
            <a className="underline" href="/docs/release/DPIA_M25_TEMPLATE.md">
              DPIA_M25_TEMPLATE.md
            </a>
          </li>
          <li>Have your DPO review + sign + compute the SHA-256 of the signed PDF.</li>
          <li>
            Run :
            <pre className="bg-slate-100 p-2 mt-1 text-xs rounded">
{`efe-admin agent dpia sign \\
  --tenant ${tenant} \\
  --addendum-hash sha256:<hex> \\
  --valid-days 365 \\
  --confirm`}
            </pre>
          </li>
          <li>
            Keep the signed PDF in your compliance vault — auditors
            will request it.
          </li>
        </ol>
      </section>
    </div>
  );
}
