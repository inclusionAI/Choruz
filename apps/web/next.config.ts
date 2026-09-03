import type { NextConfig } from "next";
import path from "node:path";

const nextPublicApiPort =
  process.env.NEXT_PUBLIC_CHORUZ_API_PORT?.trim()
  || process.env.CHORUZ_API_PORT?.trim()
  || "3000";

const nextConfig: NextConfig = {
  reactStrictMode: true,
  output: "standalone",
  env: {
    NEXT_PUBLIC_CHORUZ_API_PORT: nextPublicApiPort,
  },
  // Next.js 16 checks hostnames here; "*" is not a valid full-domain wildcard.
  // 127.0.0.1 is needed when VS Code forwards localhost and the browser opens that host.
  allowedDevOrigins: ["127.0.0.1"],
  experimental: {
    // /dashboard bootstraps once, then the acknowledged sync stream owns live
    // state. Avoid App Router refreshes remounting that client cache.
    staleTimes: { dynamic: 3600 },
  },
  outputFileTracingRoot: path.join(__dirname, "..", ".."),
  outputFileTracingExcludes: {
    "*": [".runtime/**"],
  },
  turbopack: {
    root: path.join(__dirname, "..", ".."),
  },
  async rewrites() {
    const apiTarget =
      process.env.CHORUZ_API_BASE_URL?.trim()
      || process.env.CHORUZ_API_URL?.trim()
      || `http://127.0.0.1:${process.env.CHORUZ_API_PORT ?? "3000"}`;
    return [
      {
        source: "/api/v1/:path*",
        destination: `${apiTarget}/v1/:path*`,
      },
    ];
  },
};

export default nextConfig;
