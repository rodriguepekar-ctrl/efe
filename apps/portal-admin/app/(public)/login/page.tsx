import { signIn } from "@/lib/auth/config";

export default function LoginPage({
  searchParams,
}: {
  searchParams: { next?: string; error?: string };
}) {
  const callbackUrl = searchParams.next ?? "/dashboard";

  return (
    <main className="flex min-h-screen items-center justify-center bg-muted">
      <div className="w-full max-w-sm space-y-8 rounded-lg bg-background p-10 shadow">
        <div className="space-y-2 text-center">
          <h1 className="text-2xl font-bold">EFE Portal Admin</h1>
          <p className="text-sm text-muted-foreground">Sign in with your organization SSO</p>
        </div>
        <form
          action={async () => {
            "use server";
            await signIn("oidc", { redirectTo: callbackUrl });
          }}
        >
          <button
            type="submit"
            className="w-full rounded-md bg-primary py-2 text-sm font-medium text-primary-foreground hover:opacity-90"
          >
            Sign in with SSO
          </button>
        </form>
        {searchParams.error && (
          <p className="text-sm text-destructive">
            Authentication failed. Please contact your administrator.
          </p>
        )}
      </div>
    </main>
  );
}
