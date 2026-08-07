import type { NextConfig } from "next";

function apiOrigin(): string {
  const configured = process.env["API_INTERNAL_URL"] ?? "http://127.0.0.1:8080";
  if (!URL.canParse(configured)) {
    throw new Error("API_INTERNAL_URL must be an absolute URL");
  }
  return new URL(configured).origin;
}

const nextConfig: NextConfig = {
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
