// Sprint Φ-D-LATACT D92 + L1 audit fix L1-3.
// Next.js 14 App Router configuration.

/** @type {import('next').NextConfig} */
const nextConfig = {
  reactStrictMode: true,
  poweredByHeader: false,

  // SLO impact : if any of the headers below is omitted, web_vitals
  // (LCP, INP, CLS) become unreliable for SLO-LATACT-* on portal.
  async headers() {
    return [
      {
        source: '/(.*)',
        headers: [
          { key: 'X-Frame-Options', value: 'DENY' },
          { key: 'X-Content-Type-Options', value: 'nosniff' },
          { key: 'Referrer-Policy', value: 'strict-origin-when-cross-origin' },
          // CSP : allow fonts + inline styles for Next.js + same-origin only.
          {
            key: 'Content-Security-Policy',
            value: [
              "default-src 'self'",
              "img-src 'self' data: blob:",
              "style-src 'self' 'unsafe-inline'",
              "script-src 'self' 'unsafe-inline'",
              "connect-src 'self'",
              "font-src 'self' data:",
              "object-src 'none'",
              "frame-ancestors 'none'",
              "base-uri 'self'",
              "form-action 'self'",
            ].join('; '),
          },
        ],
      },
    ];
  },

  // L2 audit forward-compat : keep telemetry compatible across builds.
  experimental: {
    instrumentationHook: false,
  },
};

export default nextConfig;
