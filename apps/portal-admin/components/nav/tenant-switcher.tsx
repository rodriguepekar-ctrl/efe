"use client";

import { useEffect, useState } from "react";

const COOKIE = "efe_active_tenant";

export function TenantSwitcher({ tenants }: { tenants: string[] }) {
  const [active, setActive] = useState(tenants[0] ?? "");

  useEffect(() => {
    const m = document.cookie.match(/(?:^|;\s*)efe_active_tenant=([^;]+)/);
    const fromCookie = m ? decodeURIComponent(m[1]!) : "";
    if (fromCookie && tenants.includes(fromCookie)) {
      setActive(fromCookie);
    }
  }, [tenants]);

  if (tenants.length <= 1) {
    return <span className="text-sm font-mono">{active}</span>;
  }

  return (
    <select
      aria-label="Active tenant"
      value={active}
      onChange={(e) => {
        const v = e.target.value;
        setActive(v);
        const exp = new Date(Date.now() + 86_400_000 * 365).toUTCString();
        document.cookie = `${COOKIE}=${encodeURIComponent(v)}; path=/; expires=${exp}; SameSite=Strict; Secure`;
        location.reload();
      }}
      className="rounded border bg-background px-2 py-1 text-sm font-mono"
    >
      {tenants.map((t) => (
        <option key={t} value={t}>
          {t}
        </option>
      ))}
    </select>
  );
}
