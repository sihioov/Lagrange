import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repo = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const selectedFiles = ["service-status.spec.ts", "stock-beta.spec.ts", "stock-beta-intraday.spec.ts"];
const legacyFile = "stock-beta-intraday.spec.ts";
const digest = (path) => createHash("sha256").update(readFileSync(path)).digest("hex");
const read = (path) => JSON.parse(readFileSync(path, "utf8"));
const save = (path, value) => writeFileSync(path, JSON.stringify(value, null, 2) + "\n");
const key = (row) => JSON.stringify([row.project, row.file, ...row.titles]);

function rows(report) {
  assert.equal(report.errors?.length ?? 0, 0, "Reporter errors");
  const found = [];
  function walk(suite, parentTitles, fileSuite) {
    const titles = fileSuite ? parentTitles : [...parentTitles, suite.title];
    for (const spec of suite.specs ?? []) {
      let file = spec.file.replaceAll("\\", "/");
      if (file.startsWith("/")) file = relative(report.config.rootDir, file).replaceAll("\\", "/");
      for (const test of spec.tests) {
        const row = { project: test.projectName, file, titles: [...titles, spec.title], test };
        assert.equal(row.project, "chromium");
        assert(selectedFiles.includes(file), "Unexpected test file " + file);
        assert(row.titles.every((title) => !/[\r\n›]/.test(title)), "Unrepresentable exact title");
        found.push(row);
      }
    }
    for (const nested of suite.suites ?? []) walk(nested, titles, false);
  }
  for (const suite of report.suites) walk(suite, [], true);
  assert.equal(new Set(found.map(key)).size, found.length, "Duplicate inventory/result key");
  return found;
}

function batches(inventory, size = 8) {
  assert(size >= 1 && size <= 8);
  assert.equal(new Set(inventory.map(key)).size, inventory.length, "Duplicate inventory key");
  const result = [];
  for (let offset = 0; offset < inventory.length; offset += size) result.push(inventory.slice(offset, offset + size));
  assert.deepEqual(result.flat().map(key), inventory.map(key));
  return result;
}

function verify(expected, actual) {
  assert.equal(new Set(actual.map(key)).size, actual.length, "Duplicate executed result");
  assert.deepEqual(actual.map(key).sort(), expected.map(key).sort(), "Missing or extra executed result");
  let passed = 0, skipped = 0;
  for (const row of actual) {
    assert.equal(row.test.status, row.file === legacyFile ? "skipped" : "expected", "Failed/flaky/interrupted result");
    assert.equal(row.test.results.length, 1, "Retries or missing execution");
    const status = row.test.results[0].status;
    if (row.file === legacyFile) {
      assert.equal(status, "skipped", "Legacy REST-off gate must stay skipped");
      skipped++;
    } else {
      assert.equal(row.test.expectedStatus, "passed");
      assert.equal(status, "passed", "Required case failed or unexpectedly skipped");
      passed++;
    }
  }
  return { collected: expected.length, passed, skipped, failed: 0, flaky: 0 };
}

function checkSources(plan) {
  for (const [path, hash] of Object.entries(plan.source_sha256)) assert.equal(digest(path), hash, "Source changed during aggregate: " + path);
}

function selfCheck() {
  const row = (title, file = "service-status.spec.ts", status = "passed") => ({
    project: "chromium", file, titles: ["tiny", title],
    test: { status: status === "skipped" ? "skipped" : "expected", expectedStatus: status === "skipped" ? "skipped" : "passed", results: [{ status }] },
  });
  const expected = [row("a"), row("b"), row("legacy", legacyFile, "skipped")];
  assert.deepEqual(verify(expected, expected), { collected: 3, passed: 2, skipped: 1, failed: 0, flaky: 0 });
  assert.equal(batches(expected, 2).length, 2);
  const negatives = [
    () => verify(expected, expected.slice(1)),
    () => verify(expected, [...expected, expected[0]]),
    () => verify(expected, [row("extra"), ...expected.slice(1)]),
    () => verify(expected, [row("a", "service-status.spec.ts", "skipped"), ...expected.slice(1)]),
    () => verify(expected, [{ ...expected[0], test: { ...expected[0].test, status: "flaky" } }, ...expected.slice(1)]),
    () => batches([...expected, expected[0]]),
  ];
  for (const negative of negatives) assert.throws(negative);
  console.log(JSON.stringify({ positive_bookkeeping: 2, rejected_missing_duplicate_extra_skip_flaky_duplicate_inventory: 6 }));
}

const [command, planPath, value] = process.argv.slice(2);
if (command === "self-check") selfCheck();
else if (command === "plan") {
  const inventoryPath = planPath;
  const target = value;
  const artifact = process.argv[5];
  const inventory = rows(read(inventoryPath));
  const counts = Object.fromEntries(selectedFiles.map((file) => [file, inventory.filter((row) => row.file === file).length]));
  assert.deepEqual(counts, { "service-status.spec.ts": 37, "stock-beta.spec.ts": 39, "stock-beta-intraday.spec.ts": 12 }, "Unexplained inventory drift");
  const sourceFiles = [...selectedFiles.map((file) => resolve(repo, "apps/web/tests/e2e", file)),
    resolve(repo, "apps/web/tests/e2e/support/service-status-fixture.mjs"),
    resolve(repo, "apps/web/playwright.config.ts")];
  for (const source of sourceFiles) {
    const archive = resolve(artifact, "sources", relative(repo, source));
    mkdirSync(dirname(archive), { recursive: true, mode: 0o700 });
    writeFileSync(archive, readFileSync(source));
  }
  const plan = {
    inventory_path: inventoryPath, inventory_sha256: digest(inventoryPath),
    source_sha256: Object.fromEntries(sourceFiles.map((file) => [file, digest(file)])),
    collected: inventory.length, counts, batches: [],
  };
  for (const [index, group] of batches(inventory).entries()) {
    const label = String(index + 1).padStart(2, "0");
    const list = resolve(dirname(target), "batch-" + label + ".txt");
    const output = resolve(artifact, "batches", label);
    mkdirSync(output, { recursive: true, mode: 0o700 });
    writeFileSync(list, group.map((row) => "[" + row.project + "] › " + row.file + " › " + row.titles.join(" › ")).join("\n") + "\n");
    plan.batches.push({ index: index + 1, expected: group.map(({test, ...row}) => row), list_file: list,
      result_file: resolve(output, "results.json"), output_dir: resolve(output, "output"), log_file: resolve(output, "runner.log") });
  }
  save(target, plan);
  console.log(JSON.stringify({ collected: plan.collected, counts, batches: plan.batches.length, max_batch_tests: 8 }));
} else if (command === "verify") {
  const plan = read(planPath); checkSources(plan);
  const batch = plan.batches[Number(value) - 1]; assert(batch, "Unknown batch");
  const receipt = verify(batch.expected, rows(read(batch.result_file)));
  save(resolve(dirname(batch.result_file), "verified.json"), receipt);
  console.log(JSON.stringify({ batch: batch.index, ...receipt }));
} else if (command === "sources") checkSources(read(planPath));
else if (command === "merge") {
  const plan = read(planPath); checkSources(plan);
  assert.equal(digest(plan.inventory_path), plan.inventory_sha256);
  const expected = plan.batches.flatMap((batch) => batch.expected);
  const actual = plan.batches.flatMap((batch) => rows(read(batch.result_file)));
  const receipt = verify(expected, actual);
  assert.equal(receipt.skipped, 12);
  const result = { ...receipt, complete_union: true, disjoint_batches: plan.batches.length, unchanged_sources: true, inventory_sha256: plan.inventory_sha256 };
  save(resolve(dirname(planPath), "aggregate.json"), result);
  console.log(JSON.stringify(result));
} else throw new Error("Use self-check, plan, sources, verify or merge");
