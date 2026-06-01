import { ConceptStackViewer } from "@/components/audit/concept-stack-viewer";
import { fetchAuditDetail } from "@/lib/trpc/server-rsc";
import { formatTimestamp } from "@/lib/utils/format";
import Link from "next/link";

export default async function AuditDetailPage({
  params,
}: {
  params: { auditId: string };
}) {
  const auditId = Number.parseInt(params.auditId, 10);
  if (Number.isNaN(auditId)) {
    return (
      <div className="p-6">
        <p className="text-destructive">Invalid audit ID.</p>
      </div>
    );
  }

  const audit = await fetchAuditDetail(auditId);
  if (!audit) {
    return (
      <div className="p-6">
        <p className="text-destructive">Audit row #{auditId} not found.</p>
      </div>
    );
  }

  const isMode3 = audit.mode === "MODE_3";

  return (
    <div className="space-y-6 p-6">
      <header className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">Audit detail #{audit.audit_id}</h1>
          <p className="text-sm text-muted-foreground">
            Mode {audit.mode} · {formatTimestamp(audit.ts)}
          </p>
        </div>
        <a
          href={`/api/audit/${audit.audit_id}/export.pdf`}
          className="rounded-md border px-4 py-2 text-sm hover:bg-muted"
          download
        >
          Export PDF (right-to-explanation)
        </a>
      </header>

      <section className="grid grid-cols-2 gap-4 lg:grid-cols-4">
        <Field label="Tenant" value={audit.tenant_id} />
        <Field label="Mode" value={audit.mode} />
        <Field label="Sentence hash" value={audit.sentence_hash} mono />
        <Field
          label="Final label"
          value={`${audit.label ?? "(none)"} (${(audit.confidence ?? 0).toFixed(3)})`}
        />
        <Field label="Duration" value={`${audit.duration_ms}ms`} />
        <Field label="Provider" value={audit.provider_used ?? "—"} />
        <Field label="Tokens" value={String(audit.total_tokens ?? "—")} />
        <Field
          label="HMAC chain"
          value={audit.chain_intact ? "✓ intact" : "✗ broken"}
          intent={audit.chain_intact ? "ok" : "alert"}
        />
      </section>

      {isMode3 && audit.concept_stack && (
        <section aria-labelledby="concept-stack-heading" className="space-y-3">
          <h2 id="concept-stack-heading" className="text-xl font-semibold">
            Concept stack — multi-turn reasoning trace
          </h2>
          <p className="text-sm text-muted-foreground">
            GDPR Art. 22 right-to-explanation : the full multi-turn reasoning trace that produced
            this classification.
          </p>
          <ConceptStackViewer stack={audit.concept_stack} />
        </section>
      )}

      <p className="text-xs text-muted-foreground">
        <Link href="/audit" className="hover:underline">
          ← Back to audit log
        </Link>
      </p>
    </div>
  );
}

function Field({
  label,
  value,
  mono,
  intent,
}: {
  label: string;
  value: string;
  mono?: boolean;
  intent?: "ok" | "alert";
}) {
  const cls = `${mono ? "font-mono text-xs" : ""} ${
    intent === "alert" ? "text-destructive" : intent === "ok" ? "text-green-600" : ""
  }`;
  return (
    <div className="rounded-md border p-3">
      <p className="text-xs text-muted-foreground">{label}</p>
      <p className={`mt-1 break-all ${cls}`}>{value}</p>
    </div>
  );
}
