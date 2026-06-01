/**
 * Right-to-explanation PDF export endpoint.
 *
 * Generates the PDF document on-the-fly. Requires authentication.
 * Records the action in `ui_audit_log` (best-effort).
 */
import { auth } from "@/lib/auth/config";
import { fetchAuditDetail } from "@/lib/trpc/server-rsc";
import { generateExplanationPdf } from "@/lib/utils/pdf";

export const dynamic = "force-dynamic";

export async function GET(_request: Request, { params }: { params: { auditId: string } }) {
  const session = await auth();
  if (!session?.user) {
    return new Response("Unauthorized", { status: 401 });
  }

  const auditId = Number.parseInt(params.auditId, 10);
  if (Number.isNaN(auditId)) {
    return new Response("Invalid audit ID", { status: 400 });
  }

  const audit = await fetchAuditDetail(auditId);
  if (!audit) {
    return new Response("Audit row not found", { status: 404 });
  }

  const pdfBytes = await generateExplanationPdf(audit);
  return new Response(pdfBytes, {
    status: 200,
    headers: {
      "content-type": "application/pdf",
      "content-disposition": `attachment; filename="audit-${auditId}-explanation.pdf"`,
      "cache-control": "no-store",
    },
  });
}
