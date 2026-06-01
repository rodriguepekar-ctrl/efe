import { Button } from "@/components/ui/button";
import { signOut } from "@/lib/auth/config";
import type { RbacUser } from "@/lib/auth/rbac";
import Link from "next/link";
import { TenantSwitcher } from "./tenant-switcher";

const NAV_ITEMS = [
  { href: "/dashboard", label: "Dashboard" },
  { href: "/audit", label: "Audit" },
  { href: "/reviews", label: "Reviews" },
  { href: "/compliance", label: "Compliance" },
  { href: "/keys", label: "Keys" },
  { href: "/quotas", label: "Quotas" },
  // M25 — Agent runtime EXPERIMENTAL (visible only when feature flag ON
  // ; the page itself shows a DPIA-required banner if not signed).
  { href: "/agent", label: "Agent" },
  { href: "/settings", label: "Settings" },
];

export function Header({ user }: { user: RbacUser }) {
  return (
    <header className="border-b bg-background">
      <div className="container flex h-14 items-center justify-between gap-4">
        <Link href="/dashboard" className="flex items-center gap-2 font-semibold">
          <span aria-hidden>⬢</span>
          <span>EFE</span>
        </Link>
        <nav className="flex flex-1 items-center gap-1">
          {NAV_ITEMS.map((it) => (
            <Link
              key={it.href}
              href={it.href}
              className="rounded-md px-3 py-1.5 text-sm hover:bg-accent"
            >
              {it.label}
            </Link>
          ))}
        </nav>
        <div className="flex items-center gap-3">
          <TenantSwitcher tenants={user.tenants} />
          <span className="text-xs text-muted-foreground">{user.email}</span>
          <form
            action={async () => {
              "use server";
              await signOut({ redirectTo: "/login" });
            }}
          >
            <Button type="submit" variant="ghost" size="sm">
              Sign out
            </Button>
          </form>
        </div>
      </div>
    </header>
  );
}
