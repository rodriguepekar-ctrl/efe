"use client";

import { httpBatchLink } from "@trpc/client";
import { createTRPCReact } from "@trpc/react-query";
import superjson from "superjson";
import type { AppRouter } from "./routers/_index";

export const trpc = createTRPCReact<AppRouter>();

export const trpcLinks = [
  httpBatchLink({
    url: "/api/trpc",
    transformer: superjson,
    headers() {
      return {
        "X-EFE-Tenant": typeof document !== "undefined" ? readActiveTenantCookie() : "",
      };
    },
  }),
];

function readActiveTenantCookie(): string {
  const m = document.cookie.match(/(?:^|;\s*)efe_active_tenant=([^;]+)/);
  return m ? decodeURIComponent(m[1]!) : "";
}
