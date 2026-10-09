import type { components, paths } from "@lagrange/api-contract";

export const SERVICE_STATUS_IDS = [
  "postgres",
  "reverse-proxy",
  "web",
  "api-server",
  "owner-equity-v2-runner",
  "owner-beta-runner",
  "candidate-runner",
  "recommendation-runner",
  "nt-backtest-worker-1",
  "nt-backtest-worker-2",
  "research-worker",
  "kis-eod",
  "market-stream",
  "intraday-calendar",
  "production-backup",
  "production-backup-verify",
  "tls-renewal",
  "docker",
  "tailscaled",
  "paper-scheduler",
  "metrics",
  "live-node-owner",
] as const satisfies readonly components["schemas"]["ServiceStatusService"]["id"][];

export type ServiceStatusId = components["schemas"]["ServiceStatusService"]["id"];

export const SERVICE_STATUS_STATES = [
  "healthy",
  "waiting",
  "degraded",
  "failed",
  "unknown",
  "disabled",
] as const satisfies readonly components["schemas"]["ServiceStatusStatus"][];

export type ServiceStatusState = components["schemas"]["ServiceStatusStatus"];

export const SERVICE_STATUS_PROCESS_STATES = [
  "running",
  "starting",
  "inactive",
  "failed",
  "missing",
  "unknown",
  "not_applicable",
  "disabled",
] as const satisfies readonly components["schemas"]["ServiceStatusService"]["process_status"][];

export type ServiceStatusProcessState =
  components["schemas"]["ServiceStatusService"]["process_status"];

export const SERVICE_STATUS_WORK_STATES = [
  "active",
  "idle",
  "scheduled",
  "closed",
  "no_demand",
  "current",
  "pending",
  "stale",
  "degraded",
  "failed",
  "unknown",
  "disabled",
  "not_applicable",
] as const satisfies readonly components["schemas"]["ServiceStatusService"]["work_status"][];

export type ServiceStatusWorkState = components["schemas"]["ServiceStatusService"]["work_status"];

export const SERVICE_STATUS_REASONS = [
  "PROCESS_RUNNING",
  "PROCESS_STARTING",
  "PROCESS_UNHEALTHY",
  "PROCESS_FAILED",
  "PROCESS_MISSING",
  "PROCESS_INACTIVE",
  "HOST_MISSING",
  "HOST_STALE",
  "HOST_INVALID",
  "HOST_FUTURE",
  "HOST_TIMEOUT",
  "HOST_COMMAND_UNAVAILABLE",
  "HOST_PERMISSION_DENIED",
  "HOST_DEADLINE",
  "HOST_REBOOTED",
  "POLICY_MISSING",
  "POLICY_MISMATCH",
  "UNEXPECTED_ACTIVE",
  "TIMER_MISSING",
  "TIMER_DISABLED",
  "TIMER_INACTIVE",
  "NEXT_RUN_MISSING",
  "RUN_ACTIVE",
  "RUN_FAILED",
  "RUN_RESULT_UNKNOWN",
  "RUN_HISTORY_EMPTY",
  "RUN_OVERDUE",
  "DAEMON_MODE_CONFLICT",
  "OLD_UNIT_IGNORED",
  "DB_TIMEOUT",
  "DB_UNAVAILABLE",
  "SCHEMA_DRIFT",
  "SOURCE_FUTURE",
  "QUERY_LIMIT",
  "QUEUE_IDLE",
  "QUEUE_ACTIVE",
  "QUEUE_PENDING",
  "QUEUE_DELAYED",
  "CLAIM_EXPIRED",
  "RECENT_JOB_FAILURE",
  "RECOVERED",
  "NO_HISTORY",
  "CALENDAR_UNAVAILABLE",
  "CALENDAR_STALE",
  "CALENDAR_LINEAGE_INVALID",
  "CALENDAR_COVERAGE_GAP",
  "SESSION_WINDOW_UNAVAILABLE",
  "SESSION_WINDOW_INVALID",
  "SESSION_CLOSED",
  "SESSION_PREOPEN",
  "NO_ACTIVE_DEMAND",
  "PRODUCER_UNAVAILABLE",
  "PRODUCER_STALE",
  "STREAM_DISCONNECTED",
  "STREAM_CONNECTING",
  "STREAM_BACKOFF",
  "STREAM_GAP",
  "QUOTE_PENDING",
  "QUOTE_STALE",
  "QUOTE_LINEAGE_INVALID",
  "INSTRUMENT_HALTED",
  "DATA_CURRENT",
  "EOD_PENDING",
  "EOD_STALE",
  "RAW_ONLY",
  "DATA_BLOCKED",
  "V2_COVERAGE_INCOMPLETE",
  "SNAPSHOT_STALE",
  "SCHEDULER_DISABLED",
  "SCHEDULER_UNAVAILABLE",
  "FEATURE_DISABLED",
  "NOT_APPLICABLE",
  "OBSERVATION_EXPIRED",
  "API_UNAVAILABLE",
  "EXECUTION_INTERRUPTED",
  "AWAITING_FIRST_TRADE",
  "CONNECTION_LOST",
  "RECONNECT_GAP",
  "SUBSCRIPTION_PENDING",
  "SUBSCRIPTION_REJECTED",
  "SUBSCRIPTION_AMBIGUOUS",
  "APPROVAL_UNAVAILABLE",
  "BUDGET_EXHAUSTED",
  "PIPELINE_LAG",
  "WIRE_SCHEMA_MISMATCH",
  "PROVIDER_RESPONSE_INVALID",
  "QUOTE_VALUE_INVALID",
  "MARKET_CLASS_UNSUPPORTED",
  "LOCAL_INGRESS_LIMIT",
  "RESYNC_REQUIRED",
  "ACCESS_REVOKED",
  "STREAM_BINDING_UNAVAILABLE",
] as const satisfies readonly components["schemas"]["ServiceStatusReason"][];

export type ServiceStatusReason = components["schemas"]["ServiceStatusReason"];
export type ServiceStatusKind = components["schemas"]["ServiceStatusService"]["kind"];
export type ServiceStatusExpectedMode =
  components["schemas"]["ServiceStatusService"]["expected_mode"];
export type ServiceStatusSourceResult = components["schemas"]["ServiceStatusSource"]["result"];
export type ServiceStatusSource = components["schemas"]["ServiceStatusSource"];
export type ServiceStatusMetrics = components["schemas"]["ServiceStatusMetrics"];
export type ServiceStatusService = components["schemas"]["ServiceStatusService"];
export type ServiceStatusResponse =
  paths["/api/v1/admin/service-status"]["get"]["responses"][200]["content"]["application/json"];

export type ServiceStatusReasonGroup =
  | "process"
  | "schedule"
  | "queue"
  | "calendar"
  | "stream"
  | "data"
  | "policy"
  | "source"
  | "expired"
  | "disabled"
  | "information";

const SERVICE_KINDS: Readonly<Record<ServiceStatusId, ServiceStatusKind>> = {
  postgres: "daemon",
  "reverse-proxy": "daemon",
  web: "daemon",
  "api-server": "daemon",
  "owner-equity-v2-runner": "daemon",
  "owner-beta-runner": "daemon",
  "candidate-runner": "daemon",
  "recommendation-runner": "daemon",
  "nt-backtest-worker-1": "daemon",
  "nt-backtest-worker-2": "daemon",
  "research-worker": "execution",
  "kis-eod": "scheduled",
  "market-stream": "capability",
  "intraday-calendar": "capability",
  "production-backup": "scheduled",
  "production-backup-verify": "scheduled",
  "tls-renewal": "scheduled",
  docker: "host",
  tailscaled: "host",
  "paper-scheduler": "optional",
  metrics: "optional",
  "live-node-owner": "optional",
};

const timestampPattern = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,6})?Z$/;
const datePattern = /^\d{4}-\d{2}-\d{2}$/;

const METRIC_KEYS = [
  "last_attempt_at",
  "last_success_at",
  "last_failure_at",
  "last_received_at",
  "oldest_provider_event_at",
  "next_run_at",
  "data_due_at",
  "data_as_of",
  "expected_session",
  "raw_as_of",
  "v2_data_as_of",
  "v2_snapshot_as_of",
  "queued",
  "running",
  "recent_failed",
  "expired_claims",
  "active_demand",
  "live_identities",
  "halted_identities",
  "required_identities",
  "covered_identities",
  "queue_scope",
  "calendar_status",
  "window_status",
  "last_state_change_at",
] as const;

const OBSERVATION_METRIC_KEYS = [
  "last_attempt_at",
  "last_success_at",
  "last_failure_at",
  "last_received_at",
  "oldest_provider_event_at",
  "last_state_change_at",
] as const;

const STREAM_DIAGNOSTIC_METRIC_KEYS = [
  "last_received_at",
  "oldest_provider_event_at",
  "last_state_change_at",
] as const;

const STATUS_PRIORITY: Readonly<Record<ServiceStatusState, number>> = {
  failed: 0,
  degraded: 1,
  unknown: 2,
  waiting: 3,
  healthy: 4,
  disabled: 5,
};

const OVERALL_PRIORITY: readonly ServiceStatusState[] = [
  "failed",
  "degraded",
  "unknown",
  "waiting",
  "healthy",
];

export class ServiceStatusContractError extends Error {
  constructor() {
    super("Invalid service status response");
    this.name = "ServiceStatusContractError";
  }
}

function fail(): never {
  throw new ServiceStatusContractError();
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function exactKeys(value: unknown, keys: readonly string[]): value is Record<string, unknown> {
  if (!isRecord(value)) return false;
  const actual = Object.keys(value);
  return actual.length === keys.length && keys.every((key) => Object.hasOwn(value, key));
}

function enumValue<const T extends readonly string[]>(value: unknown, allowed: T): T[number] {
  if (typeof value !== "string") return fail();
  const match = allowed.find((candidate) => candidate === value);
  if (match === undefined) return fail();
  return match;
}

function isValidUtcDate(year: number, month: number, day: number): boolean {
  if (month < 1 || month > 12 || day < 1 || day > 31) return false;
  const date = new Date(0);
  date.setUTCFullYear(year, month - 1, day);
  date.setUTCHours(0, 0, 0, 0);
  return (
    date.getUTCFullYear() === year && date.getUTCMonth() === month - 1 && date.getUTCDate() === day
  );
}

export function parseServiceStatusTimestampMicros(value: string): bigint | null {
  if (!timestampPattern.test(value)) return null;
  const match = /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.(\d{1,6}))?Z$/.exec(value);
  if (match === null) return null;
  const [, yearText, monthText, dayText, hourText, minuteText, secondText, fractionText] = match;
  const year = Number(yearText);
  const month = Number(monthText);
  const day = Number(dayText);
  const hour = Number(hourText);
  const minute = Number(minuteText);
  const second = Number(secondText);
  if (!isValidUtcDate(year, month, day) || hour > 23 || minute > 59 || second > 59) {
    return null;
  }
  const date = new Date(0);
  date.setUTCFullYear(year, month - 1, day);
  date.setUTCHours(hour, minute, second, 0);
  const milliseconds = date.getTime();
  if (!Number.isFinite(milliseconds)) return null;
  const micros = BigInt((fractionText ?? "").padEnd(6, "0") || "0");
  return BigInt(milliseconds) * 1_000n + micros;
}

function isValidDateOnly(value: string): boolean {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  return match !== null && isValidUtcDate(Number(match[1]), Number(match[2]), Number(match[3]));
}

function parseTimestamp(value: unknown): string | null {
  if (value === null) return null;
  if (typeof value !== "string" || parseServiceStatusTimestampMicros(value) === null) return fail();
  return value;
}

function parseNullableDate(value: unknown): string | null {
  if (value === null) return null;
  if (typeof value !== "string" || !datePattern.test(value) || !isValidDateOnly(value))
    return fail();
  return value;
}

function parseCount(value: unknown): number | null {
  if (value === null) return null;
  if (typeof value !== "number" || !Number.isInteger(value) || value < 0 || value > 1_000_000) {
    return fail();
  }
  return value;
}

function compareTimestampToGenerated(value: string | null, generatedMicros: bigint): void {
  if (value === null) return;
  const valueMicros = parseServiceStatusTimestampMicros(value);
  if (valueMicros === null || valueMicros > generatedMicros) fail();
}

function kstDateFromTimestampMicros(generatedMicros: bigint): string {
  const generatedMilliseconds = Number(generatedMicros / 1_000n);
  const kstMilliseconds = generatedMilliseconds + 9 * 60 * 60 * 1_000;
  return new Date(kstMilliseconds).toISOString().slice(0, 10);
}

function parseSource(value: unknown, generatedMicros: bigint): ServiceStatusSource | null {
  if (value === null) return null;
  if (!exactKeys(value, ["result", "observed_at", "valid_until"])) return fail();
  const observedAt = parseTimestamp(value["observed_at"]);
  const validUntil = parseTimestamp(value["valid_until"]);
  compareTimestampToGenerated(observedAt, generatedMicros);
  return {
    result: enumValue(value["result"], [
      "ok",
      "missing",
      "timeout",
      "unavailable",
      "denied",
      "invalid",
      "future",
      "stale",
      "limit",
      "partial",
    ] as const),
    observed_at: observedAt,
    valid_until: validUntil,
  };
}

function parseMetrics(value: unknown, generatedMicros: bigint): ServiceStatusMetrics {
  if (!exactKeys(value, METRIC_KEYS)) return fail();
  const timestamps = {
    last_attempt_at: parseTimestamp(value["last_attempt_at"]),
    last_success_at: parseTimestamp(value["last_success_at"]),
    last_failure_at: parseTimestamp(value["last_failure_at"]),
    last_received_at: parseTimestamp(value["last_received_at"]),
    oldest_provider_event_at: parseTimestamp(value["oldest_provider_event_at"]),
    next_run_at: parseTimestamp(value["next_run_at"]),
    data_due_at: parseTimestamp(value["data_due_at"]),
    last_state_change_at: parseTimestamp(value["last_state_change_at"]),
  };
  for (const key of OBSERVATION_METRIC_KEYS) {
    // Raw stream clocks are checked against their row's explicit diagnosis
    // below. They remain diagnostic data, separate from positive coverage.
    if (STREAM_DIAGNOSTIC_METRIC_KEYS.some((metric) => metric === key)) continue;
    compareTimestampToGenerated(timestamps[key], generatedMicros);
  }

  const generationKstDate = kstDateFromTimestampMicros(generatedMicros);
  const dates = {
    data_as_of: parseNullableDate(value["data_as_of"]),
    expected_session: parseNullableDate(value["expected_session"]),
    raw_as_of: parseNullableDate(value["raw_as_of"]),
    v2_data_as_of: parseNullableDate(value["v2_data_as_of"]),
    v2_snapshot_as_of: parseNullableDate(value["v2_snapshot_as_of"]),
  };
  for (const key of ["data_as_of", "raw_as_of", "v2_data_as_of", "v2_snapshot_as_of"] as const) {
    const date = dates[key];
    if (date !== null && date > generationKstDate) return fail();
  }

  return {
    ...timestamps,
    ...dates,
    queued: parseCount(value["queued"]),
    running: parseCount(value["running"]),
    recent_failed: parseCount(value["recent_failed"]),
    expired_claims: parseCount(value["expired_claims"]),
    active_demand: parseCount(value["active_demand"]),
    live_identities: parseCount(value["live_identities"]),
    halted_identities: parseCount(value["halted_identities"]),
    required_identities: parseCount(value["required_identities"]),
    covered_identities: parseCount(value["covered_identities"]),
    queue_scope: enumValue(value["queue_scope"], [
      "actor_shared",
      "actor",
      "not_applicable",
    ] as const),
    calendar_status: enumValue(value["calendar_status"], [
      "ready",
      "closed",
      "missing",
      "stale",
      "invalid",
      "unknown",
      "not_applicable",
    ] as const),
    window_status: enumValue(value["window_status"], [
      "ready",
      "closed",
      "missing",
      "invalid",
      "unknown",
      "not_applicable",
    ] as const),
  };
}

function isContributingService(id: ServiceStatusId): boolean {
  return id !== "research-worker";
}

export function serviceStatusOverall(
  statuses: readonly { readonly id: ServiceStatusId; readonly status: ServiceStatusState }[],
): ServiceStatusState {
  const included = statuses.filter((row) => isContributingService(row.id));
  for (const status of OVERALL_PRIORITY) {
    if (included.some((row) => row.status === status)) return status;
  }
  return "disabled";
}

export function serviceStatusContributesToOverall(id: ServiceStatusId): boolean {
  return isContributingService(id);
}

export function serviceStatusSortRank(status: ServiceStatusState): number {
  return STATUS_PRIORITY[status];
}

export function serviceStatusReasonGroup(reason: ServiceStatusReason): ServiceStatusReasonGroup {
  if (reason === "FEATURE_DISABLED") return "disabled";
  if (reason === "OBSERVATION_EXPIRED") return "expired";
  if (reason === "NOT_APPLICABLE" || reason === "PROCESS_RUNNING" || reason === "DATA_CURRENT") {
    return "information";
  }
  if (reason.startsWith("PROCESS_")) return "process";
  if (reason.startsWith("HOST_") || reason.startsWith("DB_") || reason === "SCHEMA_DRIFT") {
    return "source";
  }
  if (
    reason.startsWith("TIMER_") ||
    reason.startsWith("RUN_") ||
    reason === "NEXT_RUN_MISSING" ||
    reason === "DAEMON_MODE_CONFLICT" ||
    reason === "OLD_UNIT_IGNORED" ||
    reason.startsWith("SCHEDULER_")
  ) {
    return "schedule";
  }
  if (
    reason.startsWith("QUEUE_") ||
    reason === "CLAIM_EXPIRED" ||
    reason === "RECENT_JOB_FAILURE" ||
    reason === "RECOVERED" ||
    reason === "NO_HISTORY" ||
    reason === "EXECUTION_INTERRUPTED"
  ) {
    return "queue";
  }
  if (reason.startsWith("CALENDAR_") || reason.startsWith("SESSION_")) return "calendar";
  if (
    reason.startsWith("STREAM_") ||
    reason.startsWith("QUOTE_") ||
    reason.startsWith("PRODUCER_") ||
    reason.startsWith("CONNECTION_") ||
    reason.startsWith("RECONNECT_") ||
    reason.startsWith("SUBSCRIPTION_") ||
    reason === "NO_ACTIVE_DEMAND" ||
    reason === "INSTRUMENT_HALTED" ||
    reason === "AWAITING_FIRST_TRADE" ||
    reason === "APPROVAL_UNAVAILABLE" ||
    reason === "BUDGET_EXHAUSTED" ||
    reason === "LOCAL_INGRESS_LIMIT" ||
    reason === "RESYNC_REQUIRED" ||
    reason === "ACCESS_REVOKED"
  ) {
    return "stream";
  }
  if (
    reason.startsWith("DATA_") ||
    reason.startsWith("EOD_") ||
    reason.startsWith("V2_") ||
    reason === "RAW_ONLY" ||
    reason === "SNAPSHOT_STALE" ||
    reason === "PIPELINE_LAG" ||
    reason === "MARKET_CLASS_UNSUPPORTED"
  ) {
    return "data";
  }
  if (reason.startsWith("POLICY_") || reason === "UNEXPECTED_ACTIVE") return "policy";
  return "source";
}

export function parseServiceStatusResponse(value: unknown): ServiceStatusResponse {
  const responseKeys = [
    "schema_version",
    "generated_at",
    "overall_status",
    "policy_state",
    "services",
  ] as const;
  if (!exactKeys(value, responseKeys)) return fail();
  if (value["schema_version"] !== "service-status.api.v1" || !Array.isArray(value["services"]))
    return fail();
  const generatedAt = parseTimestamp(value["generated_at"]);
  if (generatedAt === null) return fail();
  const generatedMicros = parseServiceStatusTimestampMicros(generatedAt);
  if (generatedMicros === null || value["services"].length !== SERVICE_STATUS_IDS.length)
    return fail();

  const services: ServiceStatusService[] = value["services"].map((candidate, index) => {
    const id = SERVICE_STATUS_IDS[index];
    if (
      id === undefined ||
      !exactKeys(candidate, [
        "id",
        "kind",
        "expected_mode",
        "process_status",
        "work_status",
        "status",
        "reason_codes",
        "observed_at",
        "valid_until",
        "sources",
        "metrics",
      ])
    ) {
      return fail();
    }
    if (candidate["id"] !== id || candidate["kind"] !== SERVICE_KINDS[id]) return fail();
    const expectedMode = enumValue(candidate["expected_mode"], [
      "always",
      "daily",
      "daemon",
      "capability",
      "disabled",
      "unknown",
    ] as const);
    const processStatus = enumValue(candidate["process_status"], SERVICE_STATUS_PROCESS_STATES);
    const workStatus = enumValue(candidate["work_status"], SERVICE_STATUS_WORK_STATES);
    const status = enumValue(candidate["status"], SERVICE_STATUS_STATES);
    if (
      !Array.isArray(candidate["reason_codes"]) ||
      candidate["reason_codes"].length < 1 ||
      candidate["reason_codes"].length > 12
    ) {
      return fail();
    }
    const reasons = candidate["reason_codes"].map((reason) =>
      enumValue(reason, SERVICE_STATUS_REASONS),
    );
    if (new Set(reasons).size !== reasons.length) return fail();
    const observedAt = parseTimestamp(candidate["observed_at"]);
    const validUntil = parseTimestamp(candidate["valid_until"]);
    compareTimestampToGenerated(observedAt, generatedMicros);
    if (status !== "unknown" && validUntil === null) return fail();
    if (status === "disabled" && expectedMode !== "disabled") return fail();
    if (!exactKeys(candidate["sources"], ["host", "database", "window"])) return fail();
    const metrics = parseMetrics(candidate["metrics"], generatedMicros);
    for (const key of STREAM_DIAGNOSTIC_METRIC_KEYS) {
      const diagnosticAt = metrics[key];
      if (
        diagnosticAt === null ||
        (parseServiceStatusTimestampMicros(diagnosticAt) ?? 0n) <= generatedMicros
      ) {
        continue;
      }
      if (
        id !== "market-stream" ||
        (status !== "degraded" && status !== "failed") ||
        !reasons.includes("SOURCE_FUTURE")
      ) {
        return fail();
      }
      // A future minimum provider clock rules out all current quotes. The
      // maximum receipt can belong to one invalid quote amid valid coverage.
      // Expired evidence may have already cleared its coverage to null.
      if (
        key === "oldest_provider_event_at" &&
        ((metrics.live_identities !== null && metrics.live_identities !== 0) ||
          (metrics.halted_identities !== null && metrics.halted_identities !== 0))
      ) {
        return fail();
      }
    }

    return {
      id,
      kind: SERVICE_KINDS[id],
      expected_mode: expectedMode,
      process_status: processStatus,
      work_status: workStatus,
      status,
      reason_codes: reasons,
      observed_at: observedAt,
      valid_until: validUntil,
      sources: {
        host: parseSource(candidate["sources"]["host"], generatedMicros),
        database: parseSource(candidate["sources"]["database"], generatedMicros),
        window: parseSource(candidate["sources"]["window"], generatedMicros),
      },
      metrics,
    };
  });

  const overallStatus = enumValue(value["overall_status"], SERVICE_STATUS_STATES);
  if (overallStatus !== serviceStatusOverall(services)) return fail();

  return {
    schema_version: "service-status.api.v1",
    generated_at: generatedAt,
    overall_status: overallStatus,
    policy_state: enumValue(value["policy_state"], [
      "verified",
      "unavailable",
      "mismatch",
    ] as const),
    services,
  };
}
