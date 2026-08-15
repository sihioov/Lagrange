import type { NextConfig } from "next";

function apiOrigin(): string {
  const configured = process.env["API_INTERNAL_URL"] ?? "http://127.0.0.1:8080";
  if (!URL.canParse(configured)) {
    throw new Error("API_INTERNAL_URL must be an absolute URL");
  }
  return new URL(configured).origin;
}

const nextConfig: NextConfig = {
  // Keep the production image small and self-contained. The standalone
  // server is copied into the runtime image by apps/web/Dockerfile; it does
  // not need the full workspace node_modules tree.
  output: "standalone",
  // Next 16.3's CLI checker expects TypeScript 6's stream behavior; this
  // workspace intentionally pins TypeScript 5.9, so keep the stable compiler
  // API path for deterministic production builds.
  experimental: { useTypeScriptCli: false },
  // `next dev` serves /_next/static chunks only to origins it recognises. The
  // e2e lane drives the app over the loopback literal, so without this the
  // client bundle is refused and nothing ever hydrates.
  allowedDevOrigins: ["127.0.0.1"],
  async rewrites() {
    return [
      {
        destination: `${apiOrigin()}/api/v1/:path*`,
        source: "/api/v1/:path*",
      },
      {
        // The auth authority is intentionally unversioned. Keep the browser
        // URL same-origin while forwarding the request to the API service.
        destination: `${apiOrigin()}/auth/:path*`,
        source: "/auth/:path*",
      },
    ];
  },
};

export default nextConfig;
