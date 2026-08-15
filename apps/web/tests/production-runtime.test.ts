import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { GET as health } from "@/app/healthz/route";

function source(relativePath: string): string {
  return readFileSync(resolve(process.cwd(), relativePath), "utf8");
}

describe("production runtime contract", () => {
  it("builds a standalone Next.js server", () => {
    // Given
    const nextConfig = source("next.config.ts");

    // Then
    expect(nextConfig).toContain('output: "standalone"');
  });

  it("keeps the web image immutable, digest-pinned, and non-root", () => {
    // Given
    const dockerfile = source("Dockerfile");
    const fromLines = dockerfile.split("\n").filter((line) => /^FROM\s+/i.test(line));

    // Then
    expect(fromLines).toHaveLength(3);
    expect(fromLines.every((line) => /@sha256:[0-9a-f]{64}/i.test(line))).toBe(true);
    expect(dockerfile).toContain(
      "node@sha256:4f696fbf39f383c1e486030ba6b289a5d9af541642fc78ab197e584a113b9c03",
    );
    expect(dockerfile).toContain("USER 10001:10001");
    expect(dockerfile).toContain('CMD ["node", "apps/web/server.js"]');
    expect(dockerfile).toContain("HEALTHCHECK");
    expect(dockerfile).not.toMatch(/NEXT_PUBLIC_[A-Z0-9_]+/);
  });

  it("serves an uncached liveness response without configuration or credentials", async () => {
    // When
    const response = health();

    // Then
    expect(response.status).toBe(200);
    expect(response.headers.get("cache-control")).toBe("no-store");
    expect(response.headers.get("content-type")).toBe("text/plain; charset=utf-8");
    await expect(response.text()).resolves.toBe("ok\n");
  });
});
