import type { z } from "zod";
import type { ApiErrorCode, ApiErrorEnvelope } from "./contracts";
import { apiErrorEnvelopeSchema } from "./contracts";

export class ApiProblem extends Error {
  override readonly name = "ApiProblem";
  readonly code: ApiErrorCode;
  readonly requestId: string;
  readonly status: number;

  constructor(status: number, envelope: ApiErrorEnvelope) {
    super(envelope.error.message);
    this.code = envelope.error.code;
    this.requestId = envelope.error.request_id;
    this.status = status;
  }
}

export class ApiContractError extends Error {
  override readonly name = "ApiContractError";
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.status = status;
  }
}

/** Only an absent or expired session should send the user back to login. */
export function isLoginRequiredError(error: unknown): error is ApiProblem {
  return (
    error instanceof ApiProblem &&
    (error.code === "SESSION_UNKNOWN" || error.code === "SESSION_EXPIRED")
  );
}

async function responseJson(response: Response): Promise<unknown> {
  try {
    return await response.json();
  } catch (error) {
    if (error instanceof SyntaxError) {
      throw new ApiContractError(response.status, "API returned malformed JSON");
    }
    throw error;
  }
}

export async function parseApiResponse<Output>(
  response: Response,
  schema: z.ZodType<Output>,
): Promise<Output> {
  const body = await responseJson(response);
  if (!response.ok) {
    const envelope = apiErrorEnvelopeSchema.safeParse(body);
    if (envelope.success) {
      throw new ApiProblem(response.status, envelope.data);
    }
    throw new ApiContractError(response.status, "API returned an invalid error envelope");
  }
  const parsed = schema.safeParse(body);
  if (!parsed.success) {
    throw new ApiContractError(
      response.status,
      "API response did not match the generated contract",
    );
  }
  return parsed.data;
}
