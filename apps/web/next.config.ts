import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  // Emit a minimal self-contained server for the Docker image (docker/web.Dockerfile).
  output: "standalone",
  poweredByHeader: false,
  // Baseline browser hardening for every route (M10). A Content-Security-Policy
  // with script rules is not set yet: it needs per-request nonces for the
  // inline scripts Next.js emits.
  headers() {
    return [
      {
        source: "/:path*",
        headers: [
          { key: "X-Content-Type-Options", value: "nosniff" },
          { key: "Referrer-Policy", value: "same-origin" },
          { key: "X-Frame-Options", value: "DENY" },
          { key: "Content-Security-Policy", value: "frame-ancestors 'none'" },
        ],
      },
    ];
  },
  cacheComponents: true,
  partialPrefetching: true,
  turbopack: {
    rules: {
      "*.css": {
        loaders: ["@tailwindcss/turbopack"],
        as: "*.css",
      },
    },
  },
};

export default nextConfig;
