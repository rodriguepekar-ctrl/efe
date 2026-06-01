import { Header } from "@/components/nav/header";
import { getRbacUser } from "@/lib/auth/session";
import { redirect } from "next/navigation";

export default async function AuthenticatedLayout({ children }: { children: React.ReactNode }) {
  const user = await getRbacUser();
  if (!user) redirect("/login");
  if (user.tenants.length === 0) {
    return (
      <main className="flex min-h-screen items-center justify-center">
        <div className="max-w-md space-y-4 text-center">
          <h2 className="text-2xl font-bold">No tenants assigned</h2>
          <p className="text-muted-foreground">
            Your account has no tenant memberships. Contact your EFE administrator.
          </p>
        </div>
      </main>
    );
  }
  return (
    <div className="flex min-h-screen flex-col">
      <Header user={user} />
      <main className="flex-1">{children}</main>
    </div>
  );
}
