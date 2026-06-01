import { auth } from "@/lib/auth/config";
/**
 * Edge middleware — runs on every request before routing.
 *
 * Responsibilities :
 *   1. Bounce unauthenticated requests to /login (except public paths)
 *   2. Inject CSP nonce per-request (XSS hardening)
 *   3. Set request id header for tracing
 */
import { type NextRequest, NextResponse } from "next/server";

const PUBLIC_PATHS = [
  "/login",
  "/error",
  "/api/auth",
  "/api/healthz",
  "/api/readyz",
  "/_next",
  "/favicon.ico",
];

export default auth((req: NextRequest & { auth: any }) => {
  const { nextUrl } = req;
  const pathname = nextUrl.pathname;
  const isAuthed = !!req.auth;
  const isPublic = PUBLIC_PATHS.some((p) => pathname.startsWith(p));

  // Generate per-request CSP nonce.
  const nonce = generateNonce();
  const requestId = generateRequestId();

  if (!isAuthed && !isPublic) {
    const url = nextUrl.clone();
    url.pathname = "/login";
    url.searchParams.set("next", pathname);
    return NextResponse.redirect(url);
  }

  const response = NextResponse.next({
    request: {
      headers: new Headers({
        ...Object.fromEntries(req.headers),
        "x-csp-nonce": nonce,
        "x-request-id": requestId,
      }),
    },
  });
  response.headers.set("x-csp-nonce", nonce);
  response.headers.set("x-request-id", requestId);
  return response;
});

export const config = {
  matcher: ["/((?!_next/static|_next/image|favicon.ico|public/).*)"],
};

function generateNonce(): string {
  // Simple base64 of crypto-random bytes (Edge runtime).
  const arr = new Uint8Array(16);
  crypto.getRandomValues(arr);
  return btoa(String.fromCharCode(...arr));
}

function generateRequestId(): string {
  return crypto.randomUUID();
}
