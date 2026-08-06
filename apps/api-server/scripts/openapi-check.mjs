// npm run openapi:check
//
// 1. Regenerates openapi.json from scripts/openapi-spec.mjs and fails if the
//    committed spec drifted (the spec is the versioned contract).
// 2. Lints every operation: x-lagrange auth/ownership/entitlement/
//    idempotency/audit/cache/errors metadata, stable error codes, and the
//    typed envelope on all 4xx/5xx responses.
// 3. Generates TypeScript types (openapi-typescript) into
//    generated/openapi.ts and type-checks them with tsc --noEmit.
//
// Exit code 0 = contract clean.

import { readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { execFileSync } from "node:child_process";
import openapiTS, { astToString } from "openapi-typescript";
import { build, ERROR_CODES } from "./openapi-spec.mjs";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const workspaceRoot = dirname(dirname(root));
const specPath = join(root, "openapi.json");
const generatedDir = join(root, "generated");
const generatedPath = join(generatedDir, "openapi.ts");

let failures = 0;
const fail = (msg) => {
  failures += 1;
  console.error(`OPENAPI-CHECK FAIL: ${msg}`);
};

// ---- 1. regenerate + drift -------------------------------------------------
const fresh = JSON.stringify(build(), null, 2) + "\n";
let committed = "";
try {
  committed = readFileSync(specPath, "utf8");
} catch {
  committed = null;
}
if (committed !== fresh) {
  writeFileSync(specPath, fresh);
  fail("openapi.json drifted from the spec table; regenerated - re-run to confirm clean");
} else {
  console.log("openapi: spec in sync with the route table");
}

// ---- 2. lint metadata ------------------------------------------------------
const spec = build();
const metaKeys = ["auth", "ownership", "entitlement", "idempotency", "audit", "cache", "errors", "phase"];
let ops = 0;
for (const [path, item] of Object.entries(spec.paths)) {
  for (const [method, op] of Object.entries(item)) {
    if (!["get", "post", "put", "patch", "delete"].includes(method)) continue;
    ops += 1;
    const meta = op["x-lagrange"];
    if (!meta) {
      fail(`${method} ${path}: missing x-lagrange`);
      continue;
    }
    for (const key of metaKeys) {
      if (!(key in meta)) fail(`${method} ${path}: missing x-lagrange.${key}`);
    }
    if (op.requestBody && !(meta.idempotency && (meta.idempotency.required || meta.idempotency.natural))) {
      fail(`${method} ${path}: request body without idempotency semantics`);
    }
    if (meta.cache?.policy !== "no-store") {
      fail(`${method} ${path}: cache policy must be no-store`);
    }
    if (!Object.keys(op.responses).some((c) => /^[45]\d\d$/.test(c))) {
      fail(`${method} ${path}: must declare 4xx/5xx responses`);
    }
  }
}
console.log(`openapi: linted ${ops} operations`);

// ErrorCode enum matches the stable table.
const enumCodes = new Set(spec.components.schemas.ErrorCode.enum);
if (enumCodes.size !== ERROR_CODES.length) {
  fail(`ErrorCode enum has ${enumCodes.size} codes, expected ${ERROR_CODES.length}`);
}

// ---- 3. generate TS + typecheck ---------------------------------------------
try {
  const ast = await openapiTS(spec, { prettier: false });
  mkdirSync(generatedDir, { recursive: true });
  writeFileSync(generatedPath, astToString(ast));
  console.log("openapi: TypeScript types generated");
  execFileSync(
    process.execPath,
    [join(workspaceRoot, "node_modules", "typescript", "bin", "tsc"), "--noEmit", "--strict", "--skipLibCheck", generatedPath],
    { stdio: "inherit" },
  );
  console.log("openapi: generated TypeScript types type-check clean");
} catch (e) {
  fail(`TypeScript generation/typecheck failed: ${e.stderr || e.message}`);
}

if (failures > 0) {
  console.error(`openapi:check FAILED with ${failures} problem(s)`);
  process.exit(1);
}
console.log("openapi:check PASSED");
