import { fetchTenantHealth } from "@/lib/trpc/server-rsc";

export default async function SettingsPage() {
  const health = await fetchTenantHealth();

  return (
    <div className="space-y-6 p-6">
      <header>
        <h1 className="text-2xl font-bold">Settings</h1>
        <p className="text-sm text-muted-foreground">
          Tenant configuration. Most fields are read-only and managed via the
          <code className="mx-1 font-mono">efe-admin</code> CLI.
        </p>
      </header>

      <section className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <Card label="Tenant ID" value={health.tenant_id} />
        <Card label="Status" value={health.status} />
        <Card
          label="Compliance"
          value={
            health.compliance.ai_act_art14_acknowledged && health.compliance.gdpr_art22_acknowledged
              ? "Fully attested"
              : "Incomplete"
          }
        />
        <Card label="Pending Mode-3 reviews" value={String(health.pending_reviews)} />
        <Card label="Cost today (USD)" value={String(health.cost_today_usd ?? "—")} />
        <Card label="Daily cap (USD)" value={String(health.daily_cost_cap_usd ?? "—")} />
      </section>
    </div>
  );
}

function Card({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-md border p-4">
      <p className="text-xs text-muted-foreground">{label}</p>
      <p className="mt-1 break-all">{value}</p>
    </div>
  );
}
