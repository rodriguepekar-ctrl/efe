import { ConceptStackViewer } from "@/components/audit/concept-stack-viewer";
import { ReviewActionPanel } from "@/components/reviews/review-action-panel";
import { fetchReviewDetail } from "@/lib/trpc/server-rsc";
import Link from "next/link";

export default async function ReviewDetailPage({
  params,
}: {
  params: { token: string };
}) {
  const review = await fetchReviewDetail(params.token);

  return (
    <div className="space-y-6 p-6">
      <header>
        <h1 className="text-2xl font-bold">Review {params.token.slice(0, 8)}…</h1>
        <p className="text-sm text-muted-foreground">
          Catalog <code className="font-mono">{review.catalog}</code> · Bundle{" "}
          <code className="font-mono">{review.bundle_version}</code> · Expires{" "}
          {new Date(review.expires_at).toLocaleString()}
        </p>
      </header>

      <section className="grid grid-cols-2 gap-4 lg:grid-cols-3">
        <Field label="Proposed label" value={review.proposed_label} />
        <Field label="Confidence" value={review.proposed_confidence.toFixed(3)} />
        <Field label="Sentence hash" value={review.sentence_hash} mono />
      </section>

      <section className="space-y-3">
        <h2 className="text-xl font-semibold">Concept stack</h2>
        <ConceptStackViewer stack={review.concept_stack} />
      </section>

      <ReviewActionPanel token={params.token} proposedLabel={review.proposed_label} />

      <p className="text-xs text-muted-foreground">
        <Link href="/reviews" className="hover:underline">
          ← Back to pending reviews
        </Link>
      </p>
    </div>
  );
}

function Field({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="rounded-md border p-3">
      <p className="text-xs text-muted-foreground">{label}</p>
      <p className={`mt-1 ${mono ? "font-mono text-xs break-all" : ""}`}>{value}</p>
    </div>
  );
}
