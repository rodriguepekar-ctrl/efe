import { createContext } from "@/lib/trpc/context";
import { appRouter } from "@/lib/trpc/routers/_index";
/**
 * tRPC fetch handler — exposed at /api/trpc/*.
 * Uses the standard fetchRequestHandler.
 */
import { fetchRequestHandler } from "@trpc/server/adapters/fetch";

const handler = (req: Request) =>
  fetchRequestHandler({
    endpoint: "/api/trpc",
    req,
    router: appRouter,
    createContext: () => createContext(req),
    onError({ path, error }) {
      // eslint-disable-next-line no-console
      console.error(`tRPC error on '${path}':`, error);
    },
  });

export { handler as GET, handler as POST };
