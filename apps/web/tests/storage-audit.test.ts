import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { describe, expect, it } from "vitest";

const SOURCE_ROOTS = ["app", "components", "lib"] as const;
const SOURCE_EXTENSIONS = new Set([".ts", ".tsx", ".js", ".jsx"]);
const FORBIDDEN_STORAGE = [
  { label: "localStorage", pattern: /\blocalStorage\b/u },
  { label: "sessionStorage", pattern: /\bsessionStorage\b/u },
  { label: "Auth0 browser token", pattern: /auth0_(?:access|refresh|id)_token/iu },
  { label: "OAuth refresh token", pattern: /refresh_token/iu },
  { label: "KIS browser credential", pattern: /kis_(?:app_?key|app_?secret|access_token)/iu },
  { label: "bearer token persistence", pattern: /Authorization\s*:\s*["'`]Bearer/iu },
  { label: "user identity in URL", pattern: /[?&]user_id=/iu },
] as const;

function extension(path: string): string {
  const dot = path.lastIndexOf(".");
  return dot === -1 ? "" : path.slice(dot);
}

function sourceFiles(path: string): string[] {
  if (!existsSync(path)) {
    return [];
  }
  const files: string[] = [];
  for (const entry of readdirSync(path, { withFileTypes: true })) {
    const entryPath = join(path, entry.name);
    if (entry.isDirectory()) {
      files.push(...sourceFiles(entryPath));
    } else if (SOURCE_EXTENSIONS.has(extension(entry.name))) {
      files.push(entryPath);
    }
  }
  return files;
}

describe("browser storage security audit", () => {
  it("contains no browser-held session, Auth0, or KIS credential storage", () => {
    // Given
    const files = SOURCE_ROOTS.flatMap((root) => sourceFiles(resolve(process.cwd(), root)));

    // When
    const violations = files.flatMap((file) => {
      const content = readFileSync(file, "utf8");
      return FORBIDDEN_STORAGE.filter(({ pattern }) => pattern.test(content)).map(
        ({ label }) => `${file}: ${label}`,
      );
    });

    // Then
    expect(violations).toEqual([]);
  });
});
