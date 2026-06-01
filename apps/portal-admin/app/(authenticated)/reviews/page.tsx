import { ReviewCard } from "@/components/reviews/review-card";
import { fetchPendingReviews } from "@/lib/trpc/server-rsc";
import Link from "next/link";

export const revalidate = 10;

export default async function ReviewsPage() {
  const result = await fetchPendingReviews();
  const reviews = result.reviews ?? [];

  if (reviews.length === 0) {
    return (
      <div className="space-y-4 p-6">
        <h1 className="text-2xl font-bold">Mode-3 pending reviews</h1>
        <p className="rounded-md border bg-green-50 p-6 text-center text-muted-foreground dark:bg-green-950">
          No reviews pending. ✨
        </p>
      </div>
    );
  }

  return (
    <div className="space-y-4 p-6">
      <header className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">Mode-3 pending reviews</h1>
          <p className="text-sm text-muted-foreground">
            {reviews.length} review{reviews.length === 1 ? "" : "s"} awaiting your decision.
          </p>
        </div>
      </header>
      <ul className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        {reviews.map((r: any) => (
          <li key={r.review_token}>
            <Link href={`/reviews/${r.review_token}`} className="block hover:opacity-90">
              <ReviewCard review={r} />
            </Link>
          </li>
        ))}
      </ul>
    </div>
  );
}
