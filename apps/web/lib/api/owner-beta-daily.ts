import type { BrowserClientOptions } from "@/lib/api/browser-client";
import { mutateWithCsrf } from "@/lib/api/browser-client";
import { parseBrowserApiResponse } from "@/lib/api/browser-response";
import { strategyConfigSchema } from "@/lib/products/contracts";
import {
  ownerBetaDailyConfigPath,
  ownerBetaDailyStatePath,
  ownerBetaDailyStateSchema,
} from "@/lib/products/owner-beta-daily-contracts";

export async function saveOwnerBetaDailyPreference(
  configId: string,
  enabled: boolean,
  options: BrowserClientOptions = {},
) {
  const response = await mutateWithCsrf(ownerBetaDailyConfigPath(configId), {
    ...options,
    json: { owner_beta_daily_enabled: enabled },
    method: "PATCH",
  });
  const config = await parseBrowserApiResponse(response, strategyConfigSchema, options);
  if (config.id !== configId || config.owner_beta_daily_enabled !== enabled) {
    throw new Error("Daily preference response did not match the requested configuration.");
  }
  return config;
}

export async function fetchOwnerBetaDailyState(
  configId: string,
  options: BrowserClientOptions & { readonly signal?: AbortSignal } = {},
) {
  const path = ownerBetaDailyStatePath(configId);
  const url = options.origin === undefined ? path : new URL(path, options.origin).toString();
  const response = await (options.fetcher ?? fetch)(url, {
    cache: "no-store",
    credentials: "same-origin",
    ...(options.signal === undefined ? {} : { signal: options.signal }),
  });
  return parseBrowserApiResponse(response, ownerBetaDailyStateSchema, options);
}
