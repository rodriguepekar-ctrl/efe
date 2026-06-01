/** @type {import('next').NextConfig} */

const csp = [
  "default-src 'self'",
  "script-src 'self' 'nonce-PLACEHOLDER' 'strict-dynamic'",
  "style-src 'self' 'unsafe-inline'",
  "img-src 'self' data:",
  // OIDC issuer + Prometheus + mcp-server allowed connect-src.
  // Each tenant adds its IdP via runtime injection at deploy.
  "connect-src 'self' https://*.example.com",
  "frame-ancestors 'none'",
  "base-uri 'self'",
  "form-action 'self'",
  "object-src 'none'",
  "upgrade-insecure-requests",
].join("; ");

const securityHeaders = [
  { key: "Content-Security-Policy", value: csp },
  { key: "X-Frame-Options", value: "DENY" },
  { key: "X-Content-Type-Options", value: "nosniff" },
  { key: "Referrer-Policy", value: "strict-origin-when-cross-origin" },
  {
    key: "Permissions-Policy",
    value: "camera=(), microphone=(), geolocation=(), payment=(), usb=()",
  },
  {
    key: "Strict-Transport-Security",
    value: "max-age=63072000; includeSubDomains; preload",
  },
];

const nextConfig = {
  output: "standalone",
  reactStrictMode: true,
  poweredByHeader: false,

  experimental: {
    serverActions: {
      allowedOrigins: process.env.NEXTAUTH_URL ? [process.env.NEXTAUTH_URL] : [],
    },
    optimizePackageImports: ["lucide-react", "recharts", "@radix-ui/react-icons"],
  },

  async headers() {
    return [
      {
        source: "/:path*",
        headers: securityHeaders,
      },
      {
        source: "/api/:path*",
        headers: [
          ...securityHeaders,
          { key: "Cache-Control", value: "no-store, no-cache, must-revalidate" },
        ],
      },
    ];
  },

  async redirects() {
    return [
      {
        source: "/",
        destination: "/dashboard",
        permanent: false,
      },
    ];
  },

  // Bundle size guards.
  webpack: (config, { isServer, dev }) => {
    if (!dev && !isServer) {
      // Prevent accidental import of server-only libs in client.
      config.resolve.fallback = {
        ...config.resolve.fallback,
        fs: false,
        net: false,
        tls: false,
        child_process: false,
      };
    }
    return config;
  },
};

module.exports = nextConfig;
