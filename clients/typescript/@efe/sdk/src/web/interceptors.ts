// Sprint D24 (Φ-B) — Browser-specific interceptors.
//
// The browser SDK has NO retry by default — caller's responsibility
// (TanStack Query / SWR have richer policies than what an SDK can
// reasonably embed). The browser SDK has NO Prometheus collectors
// either (no scrape endpoint in browser).
//
// This file is small by design ; expanded in D26+ if user demand.

// Currently a placeholder for future browser-specific interceptors
// (e.g. CSRF token injection, fetch-level rate-limit handling).
// Empty default export signals "no extra interceptors" without
// breaking import callers.
export {};
