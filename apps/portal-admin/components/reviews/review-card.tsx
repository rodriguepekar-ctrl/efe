interface Review {
  review_token: string;
  catalog: string;
  bundle_version: string;
  proposed_label: string;
  proposed_confidence: number;
  expires_at: string;
  requested_at: string;
}

export function ReviewCard({ review }: { review: Review }) {
  const expiresIn = Math.max(
    0,
    Math.round((new Date(review.expires_at).getTime() - Date.now()) / 86400_000),
  );

  return (
    <article className="rounded-md border p-4 transition-colors hover:bg-accent/40">
      <header className="flex items-baseline justify-between">
        <h3 className="font-mono text-xs">{review.review_token.slice(0, 8)}…</h3>
        <span className={`text-xs ${expiresIn < 2 ? "text-destructive" : "text-muted-foreground"}`}>
          expires in {expiresIn}d
        </span>
      </header>
      <p className="mt-2 text-sm">
        Catalog <code className="font-mono">{review.catalog}</code> ·{" "}
        <code className="font-mono">{review.bundle_version}</code>
      </p>
      <p className="mt-2 text-sm">
        Proposed: <strong>{review.proposed_label}</strong>{" "}
        <span className="text-muted-foreground">({review.proposed_confidence.toFixed(3)})</span>
      </p>
    </article>
  );
}
