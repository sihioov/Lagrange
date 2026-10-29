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
    if (op.requestBody && meta.idempotency?.note !== "read-only" && !(meta.idempotency && (meta.idempotency.required || meta.idempotency.natural))) {
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

// Keep Rust-shaped v2 target rows separate from sparse positive holdings in
// both strategy and benchmark ledgers.
const v2ETF11 = [
  "069500.KRX",
  "102110.KRX",
  "114260.KRX",
  "132030.KRX",
  "133690.KRX",
  "143850.KRX",
  "148070.KRX",
  "153130.KRX",
  "192090.KRX",
  "195930.KRX",
  "229200.KRX",
];
const v2LedgerPaths = {
  strategy: ["components", "schemas", "SimulationResultV2", "properties", "ledger", "properties"],
  benchmark: ["components", "schemas", "SimulationResultV2", "properties", "benchmark", "properties"],
};
const schemaAt = (path) => path.reduce((value, key) => value?.[key], spec);
for (const [ledgerName, ledgerPath] of Object.entries(v2LedgerPaths)) {
  const targetPath = [...ledgerPath, "rebalances", "items", "properties", "target_quantities"];
  const targets = schemaAt(targetPath);
  if (targets?.type !== "array" || targets.minItems !== 11 || targets.maxItems !== 11) {
    fail(`${targetPath.join(".")}: target quantities must contain exactly ETF11`);
  }
  const targetQuantity = targets?.items?.properties?.quantity;
  if (targetQuantity?.type !== "integer" || targetQuantity.minimum !== 0) {
    fail(`${targetPath.join(".")}: target quantities must be nonnegative integers`);
  }
  const targetInstrumentIds = targets?.items?.properties?.instrument_id?.enum;
  if (JSON.stringify(targetInstrumentIds) !== JSON.stringify(v2ETF11)) {
    fail(`${targetPath.join(".")}: target quantity instruments must be the fixed ETF11 inventory`);
  }

  const holdingPaths = [
    [...ledgerPath, "rebalances", "items", "properties", "positions_after"],
    [...ledgerPath, "daily_records", "items", "properties", "opening_positions"],
    [...ledgerPath, "daily_records", "items", "properties", "closing_positions"],
    [...ledgerPath, "metrics", "properties", "ending_holdings"],
  ];
  for (const holdingPath of holdingPaths) {
    const holdingQuantity = schemaAt(holdingPath)?.items?.properties?.quantity;
    if (holdingQuantity?.type !== "integer" || holdingQuantity.minimum !== 1) {
      fail(`${holdingPath.join(".")}: holdings must remain positive integers (${ledgerName})`);
    }
  }
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
