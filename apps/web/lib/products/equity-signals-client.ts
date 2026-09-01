import type { z } from "zod";
import { type BrowserClientOptions, mutateWithCsrf } from "@/lib/api/browser-client";
import { parseBrowserApiResponse } from "@/lib/api/browser-response";
import type { ProductMutationPath } from "@/lib/api/contracts";
import {
  OWNER_EQUITY_V2_MEMBERSHIPS_PATH,
  OWNER_EQUITY_V2_SIGNALS_LATEST_PATH,
  OWNER_EQUITY_V2_SIGNALS_SCREEN_PATH,
  type OwnerEquityV2AddBody,
  type OwnerEquityV2LatestSignalsModel,
  type OwnerEquityV2MembershipListModel,
  type OwnerEquityV2MembershipStatusModel,
  type OwnerEquityV2MutationModel,
  type OwnerEquityV2ScreenBody,
  type OwnerEquityV2ScreenSignalsModel,
  type OwnerEquityV2SignalDetailModel,
  ownerEquityV2AddBodySchema,
  ownerEquityV2DisablePath,
  ownerEquityV2LatestSignalsSchema,
  ownerEquityV2MembershipListSchema,
  ownerEquityV2MembershipPath,
  ownerEquityV2MembershipStatusSchema,
  ownerEquityV2MutationSchema,
  ownerEquityV2RetryPath,
  ownerEquityV2ScreenBodySchema,
  ownerEquityV2ScreenSignalsSchema,
  ownerEquityV2SignalDetailPath,
  ownerEquityV2SignalDetailSchema,
} from "@/lib/products/equity-signals-contracts";

export type EquitySignalsBrowserOptions = BrowserClientOptions & {
  readonly fetcher?: typeof fetch;
  readonly origin?: string;
};

function requestUrl(path: string, origin: string | undefined): string {
  return origin === undefined ? path : new URL(path, origin).toString();
}

async function getParsed<Output>(
  path: string,
  schema: z.ZodType<Output>,
  options: EquitySignalsBrowserOptions,
): Promise<Output> {
  const response = await (options.fetcher ?? fetch)(requestUrl(path, options.origin), {
    cache: "no-store",
    credentials: "same-origin",
  });
  return parseBrowserApiResponse(response, schema, options);
}

export function getOwnerEquityV2Memberships(
  options: EquitySignalsBrowserOptions = {},
): Promise<OwnerEquityV2MembershipListModel> {
  return getParsed(OWNER_EQUITY_V2_MEMBERSHIPS_PATH, ownerEquityV2MembershipListSchema, options);
}

export function getOwnerEquityV2MembershipStatus(
  membershipId: string,
  options: EquitySignalsBrowserOptions = {},
): Promise<OwnerEquityV2MembershipStatusModel> {
  return getParsed(
    ownerEquityV2MembershipPath(membershipId),
    ownerEquityV2MembershipStatusSchema,
    options,
  );
}

export function getOwnerEquityV2LatestSignals(
  options: EquitySignalsBrowserOptions = {},
): Promise<OwnerEquityV2LatestSignalsModel> {
  return getParsed(OWNER_EQUITY_V2_SIGNALS_LATEST_PATH, ownerEquityV2LatestSignalsSchema, options);
}

export async function screenOwnerEquityV2Signals(
  body: OwnerEquityV2ScreenBody,
  options: EquitySignalsBrowserOptions = {},
): Promise<OwnerEquityV2ScreenSignalsModel> {
  const requestBody = ownerEquityV2ScreenBodySchema.parse(body);
  const response = await (options.fetcher ?? fetch)(
    requestUrl(OWNER_EQUITY_V2_SIGNALS_SCREEN_PATH, options.origin),
    {
      body: JSON.stringify(requestBody),
      cache: "no-store",
      credentials: "same-origin",
      headers: { "Content-Type": "application/json" },
      method: "POST",
    },
  );
  return parseBrowserApiResponse(response, ownerEquityV2ScreenSignalsSchema, options);
}

export async function addOwnerEquityV2Membership(
  body: OwnerEquityV2AddBody,
  options: EquitySignalsBrowserOptions = {},
): Promise<OwnerEquityV2MutationModel> {
  const requestBody = ownerEquityV2AddBodySchema.parse(body);
  const response = await mutateWithCsrf(OWNER_EQUITY_V2_MEMBERSHIPS_PATH, {
    ...options,
    json: requestBody,
    method: "POST",
  });
  return parseBrowserApiResponse(response, ownerEquityV2MutationSchema, options);
}

async function transitionOwnerEquityV2Membership(
  path: string,
  options: EquitySignalsBrowserOptions,
): Promise<OwnerEquityV2MutationModel> {
  const response = await mutateWithCsrf(path as ProductMutationPath, {
    ...options,
    json: {},
    method: "POST",
  });
  return parseBrowserApiResponse(response, ownerEquityV2MutationSchema, options);
}

export function retryOwnerEquityV2Membership(
  membershipId: string,
  options: EquitySignalsBrowserOptions = {},
): Promise<OwnerEquityV2MutationModel> {
  return transitionOwnerEquityV2Membership(ownerEquityV2RetryPath(membershipId), options);
}

export function disableOwnerEquityV2Membership(
  membershipId: string,
  options: EquitySignalsBrowserOptions = {},
): Promise<OwnerEquityV2MutationModel> {
  return transitionOwnerEquityV2Membership(ownerEquityV2DisablePath(membershipId), options);
}

export function getOwnerEquityV2SignalDetail(
  instrumentId: string,
  options: EquitySignalsBrowserOptions = {},
): Promise<OwnerEquityV2SignalDetailModel> {
  return getParsed(
    ownerEquityV2SignalDetailPath(instrumentId),
    ownerEquityV2SignalDetailSchema,
    options,
  );
}
