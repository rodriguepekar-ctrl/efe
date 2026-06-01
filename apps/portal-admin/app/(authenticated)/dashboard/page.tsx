import { KpiCard } from "@/components/dashboard/kpi-card";
import { fetchTenantHealth } from "@/lib/trpc/server-rsc";
import { formatCostUsd, formatLatencyMs, formatPercent, formatRps } from "@/lib/utils/format";
import { Suspense } from "react";

export const revalidate = 30;

export default async function DashboardPage() {
  const health = await fetchTenantHealth();

  return (
    <div className="space-y-6 p-6">
      <header>
        <h1 className="text-2xl font-bold">Dashboard</h1>
        <p className="text-sm text-muted-foreground">
          Tenant: <span className="font-mono">{health.tenant_id}</span> · Status:{" "}
          <span className={health.status === "healthy" ? "text-green-600" : "text-amber-600"}>
            {health.status}
          </span>
        </p>
      </header>

      <section
        className="grid grid-cols-1 gap-4 md:grid-cols-3 lg:grid-cols-6"
        aria-label="Key performance indicators"
      >
        <KpiCard label="RPS (5m)" value={formatRps(health.metrics.rps_last_5m)} />
        <KpiCard label="p99 latency" value={formatLatencyMs(health.metrics.p99_ms_last_5m)} />
        <KpiCard label="Error rate" value={formatPercent(health.metrics.error_rate_last_5m)} />
        <KpiCard label="Cost today" value={formatCostUsd(health.cost_today_usd)} />
        <KpiCard label="Pending reviews" value={String(health.pending_reviews)} />
        <KpiCard
          label="Compliance"
          value={
            health.compliance.ai_act_art14_acknowledged && health.compliance.gdpr_art22_acknowledged
              ? "✓"
              : "⚠"
          }
        />
      </section>

      <section className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <Suspense fallback={<ChartFallback label="RPS" />}>
          <RpsChart />
        </Suspense>
        <Suspense fallback={<ChartFallback label="Latency p99" />}>
          <LatencyChart />
        </Suspense>
      </section>
    </div>
  );
}

function ChartFallback({ label }: { label: string }) {
  return (
    <div className="flex h-72 items-center justify-center rounded-md border bg-muted/30 text-muted-foreground">
      Loading {label}…
    </div>
  );
}

async function RpsChart() {
  // Lazy-loaded client chart kept off the critical path.
  const { RpsChart: Chart } = await import("@/components/dashboard/rps-chart");
  return <Chart />;
}

async function LatencyChart() {
  const { LatencyChart: Chart } = await import("@/components/dashboard/latency-chart");
  return <Chart />;
}
