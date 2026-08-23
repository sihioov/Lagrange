import type { ReactNode } from "react";
import { OwnerAccessRefusal } from "@/components/pages/owner-route";
import { permitsOwnerBetaProduct } from "@/lib/api/contracts";
import { getServerSession } from "@/lib/api/server-session";
import { getLocale } from "@/lib/i18n/server";

export type OwnerBetaProductRouteProps = {
  readonly renderProduct: () => ReactNode | Promise<ReactNode>;
  readonly title: string;
};

/**
 * Defense-in-depth boundary for the temporary Owner-only beta.
 *
 * `renderProduct` is deliberately lazy: a refused Member never constructs the
 * product page and cannot start recommendation, backtest, or Paper requests.
 * The API middleware remains authoritative for every direct request.
 */
export async function OwnerBetaProductRoute({ renderProduct, title }: OwnerBetaProductRouteProps) {
  const [session, locale] = await Promise.all([getServerSession(), getLocale()]);
  if (!permitsOwnerBetaProduct(session)) {
    return <OwnerAccessRefusal locale={locale} title={title} />;
  }
  return renderProduct();
}
