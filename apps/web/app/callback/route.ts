import type { NextRequest } from "next/server";
import { NextResponse } from "next/server";

const NO_STORE_HEADERS = { "Cache-Control": "no-store" } as const;

function redirect(destination: URL): NextResponse {
  return NextResponse.redirect(destination, { headers: NO_STORE_HEADERS, status: 307 });
}

export function GET(request: NextRequest): NextResponse {
  const code = request.nextUrl.searchParams.get("code");
  const state = request.nextUrl.searchParams.get("state");
  if (code === null || state === null) {
    const invalid = new URL("/login", request.url);
    invalid.searchParams.set("reason", "callback-invalid");
    return redirect(invalid);
  }
  const destination = new URL("/auth/callback", request.url);
  destination.searchParams.set("code", code);
  destination.searchParams.set("state", state);
  return redirect(destination);
}
