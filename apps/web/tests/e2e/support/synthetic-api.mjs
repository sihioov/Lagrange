import { createServer } from "node:http";

const port = Number.parseInt(process.env.SYNTHETIC_API_PORT ?? "38180", 10);
const defaultScenario = Object.freeze({
  entitlement: "active",
  exclusions: "present",
  recommendation: "fresh",
});
let scenario = { ...defaultScenario };

function json(response, status, body) {
  response.writeHead(status, {
    "Cache-Control": "no-store",
    "Content-Type": "application/json; charset=utf-8",
  });
  response.end(JSON.stringify(body));
}

async function requestBody(request) {
  const chunks = [];
  for await (const chunk of request) {
    chunks.push(chunk);
  }
  if (chunks.length === 0) {
    return {};
  }
  return JSON.parse(Buffer.concat(chunks).toString("utf8"));
}

const server = createServer(async (request, response) => {
  const url = new URL(request.url ?? "/", `http://127.0.0.1:${port}`);
  if (request.method === "POST" && url.pathname === "/__test/scenario") {
    const body = await requestBody(request);
    scenario = { ...scenario, ...body };
    json(response, 200, { scenario });
    return;
  }
  if (request.method === "GET" && url.pathname === "/api/v1/auth/session") {
    json(response, 200, {
      expires_at_secs: 2_000_000_000,
      role: "member",
      user_id: "00000000-0000-4000-8000-000000000002",
    });
    return;
  }
  json(response, 501, {
    error: {
      code: "NOT_IMPLEMENTED",
      message: `Synthetic endpoint ${request.method} ${url.pathname} is not implemented`,
      request_id: "request-synthetic-red",
    },
  });
});

server.listen(port, "127.0.0.1", () => {
  process.stdout.write(`SYNTHETIC_API_READY http://127.0.0.1:${port}\n`);
});

function shutdown() {
  server.close(() => process.exit(0));
}

process.on("SIGINT", shutdown);
process.on("SIGTERM", shutdown);
