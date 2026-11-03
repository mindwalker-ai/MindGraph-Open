import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  // Emit a minimal self-contained server for the Docker image (docker/web.Dockerfile).
  output: "standalone",
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
