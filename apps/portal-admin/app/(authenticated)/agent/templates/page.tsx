// AgentRunTemplate list (M25).

import { fetchAgentTemplates } from "@/lib/agent/api";
import { getRbacUser } from "@/lib/auth/session";

export const dynamic = "force-dynamic";

export default async function AgentTemplatesPage() {
  const user = await getRbacUser();
  if (!user) return <p className="p-6">Sign in.</p>;
  const tmpls = await fetchAgentTemplates();
  return (
    <div className="p-6 space-y-4">
      <header>
        <h1 className="text-2xl font-semibold">Agent run templates</h1>
        <p className="text-sm text-slate-600">
          Reusable parameterized agent configs (profile + tool whitelist
          + LLM defaults). Edit via{" "}
          <code className="text-xs">efe-admin agent template upsert --file=</code>.
        </p>
      </header>
      {tmpls.length === 0 ? (
        <p className="text-sm text-slate-600">No templates yet.</p>
      ) : (
        <table className="w-full text-sm border-collapse">
          <thead>
            <tr className="text-left">
              <th className="py-2 border-b">Slug</th>
              <th className="py-2 border-b">Version</th>
              <th className="py-2 border-b">Profile</th>
              <th className="py-2 border-b">Provider/Model</th>
              <th className="py-2 border-b">Max steps</th>
            </tr>
          </thead>
          <tbody>
            {tmpls.map((t: any) => (
              <tr key={`${t.slug}-${t.version}`}>
                <td className="py-2 border-b font-mono">{t.slug}</td>
                <td className="py-2 border-b">
                  v{t.version}{t.deprecated ? " (deprecated)" : ""}
                </td>
                <td className="py-2 border-b">{t.profile}</td>
                <td className="py-2 border-b">
                  {t.default_provider}/{t.default_model}
                </td>
                <td className="py-2 border-b">{t.max_steps}</td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
