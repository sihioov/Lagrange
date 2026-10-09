// Contract fragment for the read-only Owner service-status API.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const contractPath = fileURLToPath(
  new URL("../../../configs/operations/service-status.schema.json", import.meta.url),
);
const contract = JSON.parse(readFileSync(contractPath, "utf8"));
const definitions = contract.$defs;

function componentName(name) {
  return "ServiceStatus" + name[0].toUpperCase() + name.slice(1);
}

function rewriteRefs(value) {
  if (Array.isArray(value)) return value.map(rewriteRefs);
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).map(([key, child]) => [
      key,
      key === "$ref" && typeof child === "string" && child.startsWith("#/$defs/")
        ? "#/components/schemas/" + componentName(child.slice("#/$defs/".length))
        : rewriteRefs(child),
    ]));
  }
  return value;
}

const apiDefinitionNames = ["timestamp", "status", "reason", "source", "metrics", "service", "apiResponse"];
export const SERVICE_STATUS_SCHEMAS = Object.fromEntries(
  apiDefinitionNames.map((name) => [componentName(name), rewriteRefs(definitions[name])]),
);

export const SERVICE_STATUS_ROUTE = [
  "GET",
  "/api/v1/admin/service-status",
  { owner: true, audit: true, serviceStatus: true },
];

export const SERVICE_STATUS_SUCCESS = {
  description: "Current Owner service and source status, re-evaluated at response time",
  headers: {
    "Cache-Control": { schema: { type: "string", const: "private, no-store" } },
    Vary: { schema: { type: "string", const: "Cookie" } },
  },
  content: {
    "application/json": {
      schema: { $ref: "#/components/schemas/ServiceStatusApiResponse" },
    },
  },
};
