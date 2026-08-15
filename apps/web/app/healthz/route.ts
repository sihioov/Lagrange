const HEALTH_HEADERS = {
  "Cache-Control": "no-store",
  "Content-Type": "text/plain; charset=utf-8",
} as const;

// Health probes must reflect the running process and must never be served
// from a build-time or shared cache.
export const dynamic = "force-dynamic";
export const fetchCache = "force-no-store";
export const revalidate = 0;

export function GET(): Response {
  return new Response("ok\n", {
    headers: HEALTH_HEADERS,
    status: 200,
  });
}
