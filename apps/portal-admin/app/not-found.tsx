import Link from "next/link";

export default function NotFound() {
  return (
    <main className="flex min-h-screen items-center justify-center p-8">
      <div className="space-y-4 text-center">
        <h2 className="text-3xl font-bold">404</h2>
        <p className="text-muted-foreground">Page not found.</p>
        <Link href="/dashboard" className="text-blue-600 hover:underline">
          Return to dashboard
        </Link>
      </div>
    </main>
  );
}
