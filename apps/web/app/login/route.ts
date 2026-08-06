import type { NextRequest } from "next/server";
import { NextResponse } from "next/server";

const NO_STORE_HEADERS = { "Cache-Control": "no-store" } as const;

export function GET(request: NextRequest): NextResponse {
  const destination = new URL("/auth/login", request.url);
  return NextResponse.redirect(destination, { headers: NO_STORE_HEADERS, status: 307 });
}
