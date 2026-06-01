import { AttestationForm } from "@/components/compliance/attestation-form";
import { hasAnyRole } from "@/lib/auth/rbac";
import { getRbacUser } from "@/lib/auth/session";
import { fetchComplianceStatus } from "@/lib/trpc/server-rsc";

export default async function CompliancePage() {
  const status = await fetchComplianceStatus();
  const user = await getRbacUser();
  const canAttest = hasAnyRole(user, ["dpo"]);

  return (
    <div className="space-y-6 p-6">
      <header>
        <h1 className="text-2xl font-bold">Compliance</h1>
        <p className="text-sm text-muted-foreground">
          EU AI Act Art. 14 (high-risk human oversight) + GDPR Art. 22 (right not to be subject to
          solely automated decisions). Attestations are immutable once recorded.
        </p>
      </header>

      <section className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <StatusCard
          title="EU AI Act Art. 14"
          subtitle="High-risk human oversight"
          ack={status.ai_act_art14}
        />
        <StatusCard
          title="GDPR Art. 22"
          subtitle="Right not to be subject to solely automated decisions"
          ack={status.gdpr_art22}
        />
      </section>

      {canAttest ? (
        <section>
          <h2 className="mb-3 text-xl font-semibold">Record new attestation</h2>
          <AttestationForm />
        </section>
      ) : (
        <p className="text-sm text-muted-foreground">
          Only users with the <code>dpo</code> role can record new attestations.
        </p>
      )}
    </div>
  );
}

function StatusCard({
  title,
  subtitle,
  ack,
}: {
  title: string;
  subtitle: string;
  ack: { acknowledged: boolean; acknowledged_at?: string | null; signed_by?: string | null };
}) {
  return (
    <div className="rounded-md border p-4">
      <h3 className="font-semibold">{title}</h3>
      <p className="text-xs text-muted-foreground">{subtitle}</p>
      <p className="mt-3">
        {ack.acknowledged ? (
          <span className="text-green-600">
            ✓ Acknowledged on {ack.acknowledged_at} by {ack.signed_by}
          </span>
        ) : (
          <span className="text-amber-600">⚠ Not acknowledged</span>
        )}
      </p>
    </div>
  );
}
