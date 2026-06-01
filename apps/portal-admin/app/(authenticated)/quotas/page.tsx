import { QuotasForm } from "@/components/quotas/quotas-form";
import { hasAnyRole } from "@/lib/auth/rbac";
import { getRbacUser } from "@/lib/auth/session";
import { fetchQuotas } from "@/lib/trpc/server-rsc";

export default async function QuotasPage() {
  const quotas = await fetchQuotas();
  const user = await getRbacUser();
  const canEdit = hasAnyRole(user, ["admin"]);

  return (
    <div className="space-y-6 p-6">
      <header>
        <h1 className="text-2xl font-bold">Quotas</h1>
        <p className="text-sm text-muted-foreground">
          Per-tenant rate limits and budget caps. Enforced server-side at the efe-server gateway.
        </p>
      </header>

      <QuotasForm initial={quotas} canEdit={canEdit} />
    </div>
  );
}
