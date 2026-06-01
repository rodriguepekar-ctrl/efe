import { router } from "../server";
import { auditRouter } from "./audit";
import { complianceRouter } from "./compliance";
import { keysRouter } from "./keys";
import { quotasRouter } from "./quotas";
import { reviewsRouter } from "./reviews";
import { tenantRouter } from "./tenant";

export const appRouter = router({
  audit: auditRouter,
  compliance: complianceRouter,
  keys: keysRouter,
  quotas: quotasRouter,
  reviews: reviewsRouter,
  tenant: tenantRouter,
});

export type AppRouter = typeof appRouter;
