import { redirect } from "next/navigation";
import type { ReactNode } from "react";
import { AppShell } from "@/components/shell/app-shell";
import type { ApiErrorCode } from "@/lib/api/contracts";
import { ApiProblem } from "@/lib/api/response";
import { getServerSession } from "@/lib/api/server-session";

export const dynamic = "force-dynamic";
export const fetchCache = "force-no-store";
export const revalidate = 0;

const LOGIN_REQUIRED_CODES = new Set<ApiErrorCode>(["SESSION_UNKNOWN", "SESSION_EXPIRED"]);

export type AuthenticatedLayoutProps = {
  readonly children: ReactNode;
};

export default async function AuthenticatedLayout({ children }: AuthenticatedLayoutProps) {
  try {
    const session = await getServerSession();
    return <AppShell session={session}>{children}</AppShell>;
  } catch (error) {
    if (error instanceof ApiProblem && LOGIN_REQUIRED_CODES.has(error.code)) {
      redirect("/login");
    }
    throw error;
  }
}
