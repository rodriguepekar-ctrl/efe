import { AuditFilters } from "@/components/audit/audit-filters";
import { AuditTable } from "@/components/audit/audit-table";
import { fetchAuditList } from "@/lib/trpc/server-rsc";

interface Props {
  searchParams: { cursor?: string; mode?: string; since?: string };
}

export default async function AuditPage({ searchParams }: Props) {
  const result = await fetchAuditList({
    cursor: searchParams.cursor,
    mode: searchParams.mode,
    since: searchParams.since,
    limit: 50,
  });

  return (
    <div className="space-y-4 p-6">
      <header>
        <h1 className="text-2xl font-bold">Audit log</h1>
        <p className="text-sm text-muted-foreground">
          Tamper-evident audit chain (HMAC-D14-bis). Mode-3 rows include the full concept_stack
          reasoning trace (right-to-explanation per GDPR Art. 22).
        </p>
      </header>

      <AuditFilters />

      <AuditTable rows={result.rows ?? []} />

      {result.next_cursor && (
        <a
          href={`/audit?cursor=${encodeURIComponent(result.next_cursor)}${
            searchParams.mode ? `&mode=${searchParams.mode}` : ""
          }`}
          className="text-sm text-blue-600 hover:underline"
        >
          Next page →
        </a>
      )}
    </div>
  );
}
