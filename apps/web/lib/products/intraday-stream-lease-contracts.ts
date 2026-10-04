import { z } from "zod";
import {
  IntradayStreamContractError,
  type IntradayStreamIdentity,
  intradayStreamIdentitySchema,
  intradayStreamTimeNs,
  intradayStreamUuidSchema,
} from "./intraday-stream-contracts";

export const INTRADAY_STREAM_PREFIX = "/api/v1/research/owner-beta/equity-universe-v2";
export const INTRADAY_STREAM_LEASE_PATH = `${INTRADAY_STREAM_PREFIX}/stream-leases`;
export const INTRADAY_STREAM_RENEW_MS = 15_000;
export const INTRADAY_STREAM_LEASE_MS = 30_000;

const sequence = z.number().int().safe().nonnegative();
const identities = z
  .array(intradayStreamIdentitySchema)
  .min(1)
  .max(30)
  .superRefine((values, context) => {
    let previous = "";
    const instruments = new Set<string>();
    for (const value of values) {
      if (value.membership_id <= previous || instruments.has(value.instrument_id)) {
        context.addIssue({ code: "custom", message: "Identities must be ordered and unique" });
        return;
      }
      previous = value.membership_id;
      instruments.add(value.instrument_id);
    }
  });

export const intradayStreamLeaseRequestSchema = z
  .object({
    schema_version: z.literal(2),
    consumer_id: intradayStreamUuidSchema,
    renewal_sequence: sequence,
    identities,
  })
  .strict();

export const intradayStreamLeaseResponseSchema = z
  .object({
    schema_version: z.literal(2),
    lease_id: intradayStreamUuidSchema,
    consumer_id: intradayStreamUuidSchema,
    renewal_sequence: sequence,
    lease_expires_at: z
      .string()
      .max(30)
      .refine((value) => {
        try {
          intradayStreamTimeNs(value);
          return true;
        } catch {
          return false;
        }
      }),
    renew_after_ms: z.literal(INTRADAY_STREAM_RENEW_MS),
    identities,
  })
  .strict();

export const intradayStreamReleaseRequestSchema = z
  .object({
    schema_version: z.literal(2),
    consumer_id: intradayStreamUuidSchema,
    renewal_sequence: sequence,
  })
  .strict();

export const intradayStreamReleaseResponseSchema = z
  .object({
    schema_version: z.literal(2),
    lease_id: intradayStreamUuidSchema,
    released: z.literal(true),
  })
  .strict();

export type IntradayStreamLeaseRequest = z.infer<typeof intradayStreamLeaseRequestSchema>;
export type IntradayStreamLease = z.infer<typeof intradayStreamLeaseResponseSchema>;
export type IntradayStreamReleaseRequest = z.infer<typeof intradayStreamReleaseRequestSchema>;

export function canonicalStreamIdentities(
  input: readonly IntradayStreamIdentity[],
): IntradayStreamIdentity[] {
  const sorted = [...input].sort((a, b) => a.membership_id.localeCompare(b.membership_id));
  const parsed = identities.safeParse(sorted);
  if (!parsed.success) throw new IntradayStreamContractError();
  return parsed.data;
}

export function matchStreamLease(
  input: unknown,
  request: IntradayStreamLeaseRequest,
  previousLeaseId?: string,
): IntradayStreamLease {
  const parsed = intradayStreamLeaseResponseSchema.safeParse(input);
  if (
    !parsed.success ||
    parsed.data.consumer_id !== request.consumer_id ||
    parsed.data.renewal_sequence !== request.renewal_sequence ||
    (previousLeaseId !== undefined && parsed.data.lease_id !== previousLeaseId) ||
    JSON.stringify(parsed.data.identities) !== JSON.stringify(request.identities)
  )
    throw new IntradayStreamContractError();
  return parsed.data;
}

export function intradayStreamLeasePath(leaseId: string): string {
  if (!intradayStreamUuidSchema.safeParse(leaseId).success) throw new IntradayStreamContractError();
  return `${INTRADAY_STREAM_LEASE_PATH}/${leaseId}`;
}

export function intradayStreamEventPath(leaseId: string): string {
  intradayStreamLeasePath(leaseId);
  return `${INTRADAY_STREAM_PREFIX}/market-stream?lease_id=${leaseId}`;
}
