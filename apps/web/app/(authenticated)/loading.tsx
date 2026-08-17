import { StatePanel } from "@/components/states/state-panel";
import { shellDictionary } from "@/lib/i18n/dictionaries/shell";
import { getLocale } from "@/lib/i18n/server";

export default async function AuthenticatedLoading() {
  const locale = await getLocale();
  const t = shellDictionary[locale];
  return <StatePanel kind="loading" message={t.loadingMessage} title={t.loadingTitle} />;
}
