import type { NextConfig } from "next";

function apiOrigin(): string {
  const configured = process.env["API_INTERNAL_URL"] ?? "http://127.0.0.1:8080";
  if (!URL.canParse(configured)) {
    throw new Error("API_INTERNAL_URL must be an absolute URL");
  }
  return new URL(configured).origin;
}

const nextConfig: NextConfig = {
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
    ];
  },
};

export default nextConfig;
