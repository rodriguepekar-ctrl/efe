"use client";

import { useEffect } from "react";

export default function ErrorBoundary({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  useEffect(() => {
    // Production : send to observability backend.
    // eslint-disable-next-line no-console
    console.error("Portal error:", error);
  }, [error]);

  return (
    <main className="flex min-h-screen items-center justify-center p-8">
      <div className="max-w-md space-y-4 text-center">
        <h2 className="text-2xl font-bold">Something went wrong</h2>
        <p className="text-muted-foreground">
          An unexpected error occurred. The incident has been recorded.
        </p>
        {error.digest && (
          <p className="font-mono text-xs text-muted-foreground">Reference: {error.digest}</p>
        )}
        <button
          type="button"
          onClick={reset}
          className="rounded-md bg-primary px-4 py-2 text-sm text-primary-foreground hover:opacity-90"
        >
          Try again
        </button>
      </div>
    </main>
  );
}
