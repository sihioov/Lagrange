import { readFileSync } from "node:fs";

export const SERVICE_STATUS_PATH = "/api/v1/admin/service-status";

const baseOutput = JSON.parse(
  readFileSync(
    new URL("../../../../../tests/fixtures/service-status/base-output.json", import.meta.url),
    "utf8",
  ),
);
const caseDocument = JSON.parse(
  readFileSync(
    new URL("../../../../../tests/fixtures/service-status/cases.json", import.meta.url),
    "utf8",
  ),
);

function clone(value) {
  return JSON.parse(JSON.stringify(value));
}

function pointerTokens(pointer) {
  if (pointer === "") return [];
  if (!pointer.startsWith("/"))
    throw new Error("Service-status fixture requires an absolute JSON pointer");
  return pointer
    .slice(1)
    .split("/")
    .map((token) => token.replaceAll("~1", "/").replaceAll("~0", "~"));
}

function child(parent, token) {
  return Array.isArray(parent) ? parent[Number(token)] : parent[token];
}

function applyPatch(document, patch) {
  const tokens = pointerTokens(patch.path);
  if (tokens.length === 0) throw new Error("Root replacement is not used by the frozen fixtures");
  const last = tokens.pop();
  let parent = document;
  for (const token of tokens) {
    parent = child(parent, token);
    if (parent === undefined || parent === null)
      throw new Error(`Missing fixture parent at ${patch.path}`);
  }

  if (Array.isArray(parent)) {
    if (patch.op === "add") {
      const index = last === "-" ? parent.length : Number(last);
      if (!Number.isInteger(index) || index < 0 || index > parent.length)
        throw new Error(`Invalid fixture array insertion at ${patch.path}`);
      parent.splice(index, 0, clone(patch.value));
      return;
    }
    const index = Number(last);
    if (!Number.isInteger(index) || index < 0 || index >= parent.length)
      throw new Error(`Missing fixture array item at ${patch.path}`);
    if (patch.op === "replace") parent[index] = clone(patch.value);
    else if (patch.op === "remove") parent.splice(index, 1);
    else throw new Error(`Unsupported fixture patch operation: ${patch.op}`);
    return;
  }

  if (typeof parent !== "object" || parent === null)
    throw new Error(`Invalid fixture parent at ${patch.path}`);
  if (patch.op === "replace") {
    if (!Object.hasOwn(parent, last))
      throw new Error(`Missing fixture replacement target at ${patch.path}`);
    parent[last] = clone(patch.value);
  } else if (patch.op === "add") {
    parent[last] = clone(patch.value);
  } else if (patch.op === "remove") {
    if (!Object.hasOwn(parent, last))
      throw new Error(`Missing fixture removal target at ${patch.path}`);
    delete parent[last];
  } else {
    throw new Error(`Unsupported fixture patch operation: ${patch.op}`);
  }
}

/** Materialize only the frozen public-output oracle; this does not run a reducer. */
export function serviceStatusCase(caseId = "baseline_idle_and_fresh") {
  const selected = caseDocument.cases.find((item) => item.id === caseId);
  if (selected === undefined) throw new Error(`Unknown frozen service-status case: ${caseId}`);
  const output = clone(baseOutput);
  for (const patch of selected.expected_patch) applyPatch(output, patch);
  return output;
}

/** Add a same-origin browser response fixture without changing the shared API fixture server. */
export async function installServiceStatusFixture(page, options = {}) {
  const state = {
    body: serviceStatusCase(options.caseId ?? "baseline_idle_and_fresh"),
    calls: 0,
    fault: null,
    pending: false,
    releasePending: null,
  };

  await page.route(`**${SERVICE_STATUS_PATH}`, async (route) => {
    if (route.request().method() !== "GET") {
      await route.fulfill({ status: 405, body: "" });
      return;
    }
    state.calls += 1;
    const fault = state.fault;
    if (fault?.kind === "network") {
      await route.abort("failed");
      return;
    }
    if (fault?.kind === "delayed") {
      state.pending = true;
      await new Promise((resolve) => {
        state.releasePending = resolve;
      });
      state.pending = false;
      state.releasePending = null;
    }

    const response = fault ?? { kind: "json", status: 200, body: state.body };
    const headers = {
      "cache-control": "no-store",
      ...(response.headers ?? {}),
    };
    const body = typeof response.body === "string" ? response.body : JSON.stringify(response.body);
    await route.fulfill({
      status: response.status ?? 200,
      contentType: response.contentType ?? "application/json",
      headers,
      body,
    });
  });

  return {
    calls: () => state.calls,
    currentBody: () => clone(state.body),
    isPending: () => state.pending,
    releaseDelayed: () => state.releasePending?.(),
    setCase: (caseId) => {
      state.body = serviceStatusCase(caseId);
      state.fault = null;
    },
    setBody: (body) => {
      state.body = clone(body);
      state.fault = null;
    },
    setFault: (fault) => {
      state.fault = fault === null ? null : { ...fault };
    },
  };
}
