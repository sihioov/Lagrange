#!/usr/bin/env node

import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

export const ACCEPTANCE_SCHEMA_VERSION = 1;
export const APPROVED_INSTRUMENTS = Object.freeze([
  "005930.KRX",
  "000660.KRX",
  "373220.KRX",
  "207940.KRX",
  "005380.KRX",
  "000270.KRX",
  "105560.KRX",
  "055550.KRX",
  "068270.KRX",
  "035420.KRX",
  "035720.KRX",
  "005490.KRX",
  "051910.KRX",
  "006400.KRX",
  "012330.KRX",
  "028260.KRX",
  "012450.KRX",
  "329180.KRX",
  "034020.KRX",
  "015760.KRX",
  "017670.KRX",
  "030200.KRX",
  "066570.KRX",
  "009150.KRX",
  "096770.KRX",
  "036570.KRX",
  "090430.KRX",
  "011200.KRX",
  "003490.KRX",
  "000810.KRX",
]);

export const API_PATHS = Object.freeze({
  session: "/api/v1/auth/session",
  csrf: "/api/v1/auth/csrf",
  memberships: "/api/v1/research/owner-beta/equity-universe-v2/memberships",
  quoteCache: "/api/v1/research/owner-beta/equity-universe-v2/instruments",
});

export const LIFECYCLES = Object.freeze([
  "REQUESTED",
  "VALIDATING",
  "BACKFILLING",
  "MATERIALIZING",
  "READY",
  "INSUFFICIENT_HISTORY",
  "FAILED",
  "DISABLED",
]);

export const QUOTE_REASON_CODES = Object.freeze([
  "NO_ACTIVE_DEMAND",
  "QUOTE_PENDING",
  "QUOTE_STALE",
  "PROVIDER_TIMEOUT",
  "PROVIDER_RATE_LIMITED",
  "PROVIDER_UNAVAILABLE",
  "PROVIDER_RESPONSE_INVALID",
  "QUOTE_VALUE_INVALID",
  "QUOTE_BUDGET_EXHAUSTED",
  "CALENDAR_UNAVAILABLE",
  "SESSION_WINDOW_UNAVAILABLE",
  "SESSION_CLOSED",
  "INSTRUMENT_HALTED",
  "PRODUCER_UNAVAILABLE",
  "FEATURE_DISABLED",
]);

const PROJECT_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
export const DEFAULT_UNIVERSE_PATH = resolve(
  PROJECT_ROOT,
  "configs/universes/kr-stock-price-beta-v1.json",
);
export const DEFAULT_LIFECYCLE_TIMEOUT_MS = 15 * 60 * 1_000;
export const DEFAULT_OBSERVATION_TIMEOUT_MS = 90 * 1_000;
// `KIS_HTTP_TIMEOUT` in crates/job-queue/src/bin/owner-equity-v2-runner.rs
// bounds a live producer transport attempt at 30 seconds. A post-close cache
// marker must remain unchanged through that whole attempt horizon: an aborted
// browser demand alone cannot prove that an already-dispatched producer call
// will not settle late.
export const REQUIRED_QUIESCENCE_STABLE_MS = 30 * 1_000;
// The widget lease is 30 seconds and the required stable period is another
// 30 seconds. Cleanup therefore has a separately bounded 60-second horizon.
export const DEFAULT_QUIESCENCE_TIMEOUT_MS = 60 * 1_000;
export const DEFAULT_POLL_INTERVAL_MS = 5_000;
// The current cache contract returns next_poll_after_ms=5000. Do not allow a
// production caller to turn lifecycle, receipt, or cleanup polling into a
// tight loop merely by passing zero.
export const MIN_PRODUCTION_POLL_INTERVAL_MS = DEFAULT_POLL_INTERVAL_MS;

const UUID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const INSTRUMENT_PATTERN = /^\d{6}\.KRX$/;
const CODE_PATTERN = /^\d{6}$/;
const DATE_PATTERN = /^(\d{4})-(\d{2})-(\d{2})$/;
const UTC_DATE_TIME_PATTERN =
  /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.\d{1,9})?Z$/;
const DECIMAL_PATTERN = /^-?(0|[1-9]\d*)(?:\.\d{1,8})?$/;
const BIGINT_PATTERN = /^(0|[1-9]\d*)$/;
const SHA256_PATTERN = /^[0-9a-f]{64}$/;
const WINDOW_SHA256_PATTERN = /^sha256:[0-9a-f]{64}$/;
const I64_MAX = 9_223_372_036_854_775_807n;

// These states are not normal producer warm-up. They prove that this bounded
// observation cannot obtain an eligible receipt without an external change.
// Other reason codes, including retained stale quotes and transient provider
// failures, remain non-receipts and are allowed to age out at the original
// observation deadline.
const TERMINAL_RECEIPT_REASONS = new Set([
  "CALENDAR_UNAVAILABLE",
  "SESSION_WINDOW_UNAVAILABLE",
  "SESSION_CLOSED",
  "INSTRUMENT_HALTED",
  "FEATURE_DISABLED",
]);

const API_ERROR_CODES = new Set([
  "SESSION_UNKNOWN",
  "SESSION_EXPIRED",
  "FORBIDDEN",
  "CSRF_DENIED",
  "OWNER_EQUITY_POLICY_UNAVAILABLE",
  "OWNER_EQUITY_MEMBERSHIP_NOT_FOUND",
  "OWNER_EQUITY_INVALID_STATE",
  "OWNER_EQUITY_ENTITLEMENT_UNAVAILABLE",
  "OWNER_EQUITY_CAPACITY_EXCEEDED",
  "OWNER_EQUITY_INTEGRITY_FAILED",
  "OWNER_EQUITY_SNAPSHOT_UNAVAILABLE",
  "IDEMPOTENCY_KEY_REQUIRED",
  "IDEMPOTENCY_KEY_MISMATCH",
  "INVALID_PARAMETER",
  "PAYLOAD_TOO_LARGE",
  "RESOURCE_NOT_FOUND",
  "QUOTE_DEMAND_SEQUENCE_CONFLICT",
  "QUOTE_DEMAND_CAPACITY",
  "QUOTE_CACHE_UNAVAILABLE",
  "INTERNAL",
]);

const SAFE_DETAIL_KEYS = new Set([
  "api_code",
  "completed_count",
  "expected_count",
  "http_status",
  "instrument_id",
  "lifecycle",
  "observed_count",
  "phase",
  "receipt_count",
  "remaining_count",
  "status",
  "timestamp",
  "version",
]);

const SAFE_LIFECYCLE_SET = new Set(LIFECYCLES);

function isPlainObject(value) {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function hasExactKeys(value, expected) {
  if (!isPlainObject(value)) return false;
  const keys = Object.keys(value).sort();
  return keys.length === expected.length && keys.every((key, index) => key === expected[index]);
}

function isNonnegativeInteger(value) {
  return Number.isSafeInteger(value) && value >= 0;
}

function isPositiveInteger(value) {
  return Number.isSafeInteger(value) && value > 0;
}

function isCalendarDate(value) {
  if (typeof value !== "string") return false;
  const match = DATE_PATTERN.exec(value);
  if (match === null) return false;
  const year = Number(match[1]);
  const month = Number(match[2]);
  const day = Number(match[3]);
  if (month < 1 || month > 12 || day < 1) return false;
  const lastDay = new Date(Date.UTC(year, month, 0)).getUTCDate();
  return day <= lastDay;
}

function isUtcDateTime(value) {
  return (
    typeof value === "string" &&
    UTC_DATE_TIME_PATTERN.test(value) &&
    isCalendarDate(value.slice(0, 10)) &&
    Number.isFinite(Date.parse(value))
  );
}

function isUuid(value) {
  return typeof value === "string" && UUID_PATTERN.test(value);
}

function isCanonicalDecimal(value) {
  if (typeof value !== "string" || !DECIMAL_PATTERN.test(value)) return false;
  if (value.startsWith("-")) {
    const unsigned = value.slice(1);
    if (/^0(?:\.0+)?$/.test(unsigned)) return false;
  }
  return true;
}

function isZeroDecimal(value) {
  const unsigned = value.startsWith("-") ? value.slice(1) : value;
  return /^0(?:\.0+)?$/.test(unsigned);
}

function decimalIsPositive(value) {
  return isCanonicalDecimal(value) && !value.startsWith("-") && !isZeroDecimal(value);
}

function decimalIsNegative(value) {
  return isCanonicalDecimal(value) && value.startsWith("-") && !isZeroDecimal(value);
}

function safeApiCode(value) {
  return typeof value === "string" && API_ERROR_CODES.has(value) ? value : "API_ERROR";
}

function safeDetails(details) {
  if (!isPlainObject(details)) return {};
  const result = {};
  for (const [key, value] of Object.entries(details)) {
    if (!SAFE_DETAIL_KEYS.has(key)) continue;
    if (key === "instrument_id" && typeof value === "string" && INSTRUMENT_PATTERN.test(value)) {
      result[key] = value;
    } else if (key === "lifecycle" && typeof value === "string" && SAFE_LIFECYCLE_SET.has(value)) {
      result[key] = value;
    } else if (key === "api_code" && typeof value === "string") {
      result[key] = safeApiCode(value);
    } else if (
      ["completed_count", "expected_count", "http_status", "observed_count", "receipt_count", "remaining_count"].includes(key) &&
      Number.isSafeInteger(value) &&
      value >= 0
    ) {
      result[key] = value;
    } else if (key === "version" && typeof value === "string" && BIGINT_PATTERN.test(value)) {
      result[key] = value;
    } else if (key === "timestamp" && typeof value === "string" && isUtcDateTime(value)) {
      result[key] = value;
    } else if (key === "phase" && typeof value === "string" && /^[a-z][a-z0-9-]{0,31}$/.test(value)) {
      result[key] = value;
    } else if (key === "status" && typeof value === "string" && /^[A-Z][A-Z0-9_]{0,63}$/.test(value)) {
      result[key] = value;
    }
  }
  return result;
}

export class LiveAcceptanceError extends Error {
  constructor(code, phase, details = {}, progress = undefined) {
    super(code);
    this.name = "LiveAcceptanceError";
    this.code = code;
    this.phase = phase;
    this.details = safeDetails({ ...details, phase });
    this.progress = progress;
  }
}

function fail(code, phase, details = {}, progress = undefined) {
  throw new LiveAcceptanceError(code, phase, details, progress);
}

function asAcceptanceError(error, fallbackPhase) {
  if (error instanceof LiveAcceptanceError) return error;
  return new LiveAcceptanceError("INTERNAL_TOOL_ERROR", fallbackPhase);
}

function errorReport(error) {
  const typed = asAcceptanceError(error, "tool");
  return {
    code: typed.code,
    phase: typed.phase,
    ...(Object.keys(typed.details).length === 0 ? {} : { details: typed.details }),
  };
}

function normalizeInstrumentId(value) {
  if (typeof value !== "string") return null;
  const trimmed = value.trim().toUpperCase();
  if (INSTRUMENT_PATTERN.test(trimmed)) return trimmed;
  if (CODE_PATTERN.test(trimmed)) return `${trimmed}.KRX`;
  return null;
}

function codeForInstrument(instrumentId) {
  return instrumentId.slice(0, 6);
}

export function onboardingIdempotencyKey(operation, instrumentId, membershipId = undefined) {
  const normalized = normalizeInstrumentId(instrumentId);
  if (normalized === null || !/^(add|retry)$/.test(operation)) {
    throw new TypeError("invalid onboarding idempotency input");
  }
  const membershipSuffix = operation === "retry" && isUuid(membershipId) ? `-${membershipId}` : "";
  return `owner-equity-v2-live-v1-${operation}-${codeForInstrument(normalized)}${membershipSuffix}`;
}

function validateApprovedUniverse(value) {
  if (!isPlainObject(value)) fail("UNIVERSE_INVALID", "input");
  if (
    value.universe_id !== "kr-stock-price-beta-v1" ||
    value.instrument_count !== APPROVED_INSTRUMENTS.length ||
    !Array.isArray(value.instruments) ||
    value.instruments.length !== APPROVED_INSTRUMENTS.length
  ) {
    fail("COUNT_DRIFT", "input", {
      expected_count: APPROVED_INSTRUMENTS.length,
      observed_count: Array.isArray(value.instruments) ? value.instruments.length : 0,
    });
  }
  const ids = value.instruments.map((instrument) => instrument?.id);
  if (ids.some((id) => typeof id !== "string" || !INSTRUMENT_PATTERN.test(id))) {
    fail("UNIVERSE_INVALID", "input");
  }
  if (new Set(ids).size !== ids.length) fail("DUPLICATE_INPUT", "input");
  if (ids.some((id, index) => id !== APPROVED_INSTRUMENTS[index])) fail("UNIVERSE_DRIFT", "input");
  return Object.freeze([...ids]);
}

export async function readApprovedUniverse(universePath = DEFAULT_UNIVERSE_PATH) {
  let raw;
  try {
    raw = await readFile(universePath, "utf8");
  } catch {
    fail("UNIVERSE_UNAVAILABLE", "input");
  }
  let value;
  try {
    value = JSON.parse(raw);
  } catch {
    fail("UNIVERSE_INVALID", "input");
  }
  return validateApprovedUniverse(value);
}

function validatePolicy(policy) {
  if (
    !hasExactKeys(policy, [
      "active_instruments",
      "max_active_instruments",
      "minimum_observed_sessions",
      "remaining_capacity",
      "target_observed_sessions",
    ]) ||
    !Object.values(policy).every(isNonnegativeInteger) ||
    policy.active_instruments > policy.max_active_instruments ||
    policy.remaining_capacity !== policy.max_active_instruments - policy.active_instruments ||
    policy.minimum_observed_sessions > policy.target_observed_sessions
  ) {
    fail("CONTRACT_INVALID", "membership-read");
  }
  return policy;
}

function validateFailure(failure) {
  if (
    !hasExactKeys(failure, ["code", "retryable"]) ||
    typeof failure.code !== "string" ||
    !/^[A-Z][A-Z0-9_]{0,63}$/.test(failure.code) ||
    typeof failure.retryable !== "boolean"
  ) {
    fail("CONTRACT_INVALID", "membership-read");
  }
  return { code: failure.code, retryable: failure.retryable };
}

function validateCoverage(coverage) {
  if (
    !isPlainObject(coverage) ||
    Object.keys(coverage).some(
      (key) => !["first_session", "last_session", "minimum_observed_sessions", "observed_sessions", "target_observed_sessions"].includes(key),
    ) ||
    !["observed_sessions", "target_observed_sessions", "minimum_observed_sessions"].every(
      (key) => key in coverage,
    ) ||
    !isNonnegativeInteger(coverage.observed_sessions) ||
    !isNonnegativeInteger(coverage.target_observed_sessions) ||
    !isNonnegativeInteger(coverage.minimum_observed_sessions) ||
    coverage.minimum_observed_sessions > coverage.target_observed_sessions
  ) {
    fail("CONTRACT_INVALID", "membership-read");
  }
  for (const key of ["first_session", "last_session"]) {
    if (key in coverage && !isCalendarDate(coverage[key])) fail("CONTRACT_INVALID", "membership-read");
  }
  return coverage;
}

function validateMembership(value, expectedInstrument = undefined, phase = "membership-read") {
  if (
    !isPlainObject(value) ||
    Object.keys(value).some(
      (key) => !["coverage", "disabled_at", "failure", "generation", "id", "instrument_id", "lifecycle", "requested_at", "updated_at"].includes(key),
    ) ||
    !["id", "instrument_id", "lifecycle", "generation", "coverage", "requested_at", "updated_at"].every(
      (key) => key in value,
    ) ||
    !isUuid(value.id) ||
    typeof value.instrument_id !== "string" ||
    !INSTRUMENT_PATTERN.test(value.instrument_id) ||
    (expectedInstrument !== undefined && value.instrument_id !== expectedInstrument) ||
    typeof value.lifecycle !== "string" ||
    !SAFE_LIFECYCLE_SET.has(value.lifecycle) ||
    !isNonnegativeInteger(value.generation) ||
    !isUtcDateTime(value.requested_at) ||
    !isUtcDateTime(value.updated_at)
  ) {
    fail("CONTRACT_INVALID", phase, {
      ...(expectedInstrument === undefined ? {} : { instrument_id: expectedInstrument }),
    });
  }
  validateCoverage(value.coverage);
  if ("disabled_at" in value && !isUtcDateTime(value.disabled_at)) fail("CONTRACT_INVALID", phase);
  if ("failure" in value) validateFailure(value.failure);
  return value;
}

function validateMembershipList(value, approved) {
  if (
    !isPlainObject(value) ||
    !hasExactKeys(value, ["memberships", "policy"]) ||
    !Array.isArray(value.memberships)
  ) {
    fail("CONTRACT_INVALID", "membership-read");
  }
  validatePolicy(value.policy);
  const approvedSet = new Set(approved);
  const seen = new Set();
  const memberships = value.memberships.map((membership) => {
    const parsed = validateMembership(membership);
    if (!approvedSet.has(parsed.instrument_id)) {
      fail("FOREIGN_MEMBERSHIP", "membership-read", { instrument_id: parsed.instrument_id });
    }
    if (seen.has(parsed.instrument_id)) {
      fail("DUPLICATE_MEMBERSHIP", "membership-read", { instrument_id: parsed.instrument_id });
    }
    seen.add(parsed.instrument_id);
    return parsed;
  });
  const activeCount = memberships.filter((membership) => membership.lifecycle !== "DISABLED").length;
  if (activeCount !== value.policy.active_instruments) fail("POLICY_MISMATCH", "membership-read");
  return { policy: value.policy, memberships };
}

function validateMutation(value, expectedInstrument, phase) {
  if (
    !isPlainObject(value) ||
    !hasExactKeys(value, ["duplicate_active", "job_id", "resource"]) ||
    !isUuid(value.job_id) ||
    typeof value.duplicate_active !== "boolean"
  ) {
    fail("CONTRACT_INVALID", phase, { instrument_id: expectedInstrument });
  }
  validateMembership(value.resource, expectedInstrument, phase);
  return value;
}

function validateStatus(value, expectedInstrument) {
  if (!isPlainObject(value) || !hasExactKeys(value, ["membership", "policy"])) {
    fail("CONTRACT_INVALID", "lifecycle", { instrument_id: expectedInstrument });
  }
  validatePolicy(value.policy);
  validateMembership(value.membership, expectedInstrument, "lifecycle");
  return value;
}

function validateSession(value) {
  if (
    !isPlainObject(value) ||
    Object.keys(value).some(
      (key) =>
        ![
          "auth_time_secs",
          "expires_at_secs",
          "owner_beta_access_mode",
          "owner_beta_paper_mode",
          "role",
          "user_id",
        ].includes(key),
    ) ||
    typeof value.user_id !== "string" ||
    !isUuid(value.user_id) ||
    !["owner", "member"].includes(value.role) ||
    !Number.isSafeInteger(value.expires_at_secs) ||
    ("auth_time_secs" in value && !Number.isSafeInteger(value.auth_time_secs)) ||
    ("owner_beta_access_mode" in value && !["disabled", "owner_only"].includes(value.owner_beta_access_mode)) ||
    ("owner_beta_paper_mode" in value && !["disabled", "enabled"].includes(value.owner_beta_paper_mode))
  ) {
    fail("CONTRACT_INVALID", "authentication");
  }
  return value;
}

function extractApiCode(body) {
  return safeApiCode(body?.error?.code);
}

function responseFailure(response, phase, expectedStatus) {
  const code = extractApiCode(response.body);
  if (response.status === 401) {
    fail(phase === "authentication" ? "AUTH_REQUIRED" : "AUTH_LOST", phase, {
      api_code: code,
      http_status: response.status,
    });
  }
  if (phase === "authentication" && response.status === 403) {
    fail("OWNER_MISMATCH", phase, { api_code: code, http_status: response.status });
  }
  fail(code, phase, { api_code: code, http_status: response.status, status: String(expectedStatus) });
}

function validateCsrf(value) {
  if (!isPlainObject(value) || typeof value.csrf_token !== "string" || value.csrf_token.length === 0) {
    fail("CONTRACT_INVALID", "csrf");
  }
  return value.csrf_token;
}

function validateSessionEvidence(value) {
  if (
    !isPlainObject(value) ||
    !hasExactKeys(value, [
      "calendar_content_sha256",
      "calendar_source",
      "calendar_source_version",
      "date",
      "timezone",
      "window_contract_sha256",
    ]) ||
    !isCalendarDate(value.date) ||
    value.timezone !== "Asia/Seoul" ||
    value.calendar_source !== "kis" ||
    value.calendar_source_version !== "kis-chk-holiday-v1:schema-1" ||
    !SHA256_PATTERN.test(value.calendar_content_sha256) ||
    !WINDOW_SHA256_PATTERN.test(value.window_contract_sha256)
  ) {
    return false;
  }
  return true;
}

function validateQuotePayload(value, nowMs) {
  if (
    !isPlainObject(value) ||
    !hasExactKeys(value, [
      "base_price",
      "change_from_previous_day",
      "change_percent_from_previous_day",
      "direction",
      "last_success_at",
      "price",
      "quote_version",
      "received_at",
    ]) ||
    !decimalIsPositive(value.price) ||
    !decimalIsPositive(value.base_price) ||
    !isCanonicalDecimal(value.change_from_previous_day) ||
    !isCanonicalDecimal(value.change_percent_from_previous_day) ||
    !["UP", "DOWN", "FLAT", "LIMIT_UP", "LIMIT_DOWN"].includes(value.direction) ||
    !isUtcDateTime(value.received_at) ||
    !isUtcDateTime(value.last_success_at) ||
    value.received_at !== value.last_success_at ||
    typeof value.quote_version !== "string" ||
    !BIGINT_PATTERN.test(value.quote_version)
  ) {
    return false;
  }
  let version;
  try {
    version = BigInt(value.quote_version);
  } catch {
    return false;
  }
  if (version > I64_MAX || Date.parse(value.last_success_at) > nowMs) return false;
  const positive = decimalIsPositive(value.change_from_previous_day) && decimalIsPositive(value.change_percent_from_previous_day);
  const negative = decimalIsNegative(value.change_from_previous_day) && decimalIsNegative(value.change_percent_from_previous_day);
  const flat = isZeroDecimal(value.change_from_previous_day) && isZeroDecimal(value.change_percent_from_previous_day);
  if (["UP", "LIMIT_UP"].includes(value.direction) && !positive) return false;
  if (["DOWN", "LIMIT_DOWN"].includes(value.direction) && !negative) return false;
  if (value.direction === "FLAT" && !flat) return false;
  return value;
}

export function validateQuote(value, identity, nowMs = Date.now()) {
  if (
    !isPlainObject(value) ||
    !hasExactKeys(value, [
      "currency",
      "freshness",
      "generation",
      "instrument_id",
      "market_state",
      "membership_id",
      "next_poll_after_ms",
      "quote",
      "reason_code",
      "schema_version",
      "session",
      "venue",
    ]) ||
    value.schema_version !== 1 ||
    !isUuid(value.membership_id) ||
    typeof value.instrument_id !== "string" ||
    !INSTRUMENT_PATTERN.test(value.instrument_id) ||
    !isPositiveInteger(value.generation) ||
    value.venue !== "KRX" ||
    value.currency !== "KRW" ||
    !["OPEN", "CLOSED", "HALTED", "UNKNOWN"].includes(value.market_state) ||
    !["RECENT", "STALE", "UNAVAILABLE"].includes(value.freshness) ||
    (value.reason_code !== null && !QUOTE_REASON_CODES.includes(value.reason_code)) ||
    value.next_poll_after_ms !== 5_000 ||
    (value.session !== null && !validateSessionEvidence(value.session)) ||
    (value.quote !== null && !validateQuotePayload(value.quote, nowMs))
  ) {
    fail("CONTRACT_INVALID", "receipt", { instrument_id: identity.instrument_id });
  }
  if (value.session === null && value.market_state !== "UNKNOWN") {
    fail("CONTRACT_INVALID", "receipt", { instrument_id: identity.instrument_id });
  }
  if (value.market_state === "UNKNOWN" && value.quote !== null) {
    fail("CONTRACT_INVALID", "receipt", { instrument_id: identity.instrument_id });
  }
  if (value.quote !== null && value.session === null) {
    fail("CONTRACT_INVALID", "receipt", { instrument_id: identity.instrument_id });
  }
  if (value.quote !== null && value.freshness === "UNAVAILABLE") {
    fail("CONTRACT_INVALID", "receipt", { instrument_id: identity.instrument_id });
  }
  if (!quoteIdentityMatches(value, identity)) {
    fail("RECEIPT_IDENTITY_MISMATCH", "receipt", { instrument_id: identity.instrument_id });
  }
  return value;
}

function summaryMembership(membership) {
  return {
    instrument_id: membership.instrument_id,
    lifecycle: membership.lifecycle,
    generation: membership.generation,
    observed_sessions: membership.coverage.observed_sessions,
    updated_at: membership.updated_at,
    ...(membership.failure === undefined ? {} : { failure_code: membership.failure.code }),
  };
}

function actionForMembership(membership) {
  if (membership === undefined) return "ADD";
  if (membership.lifecycle === "READY") return "PRESERVE_READY";
  if (membership.lifecycle === "DISABLED") return "STOP_DISABLED";
  if (membership.lifecycle === "FAILED") {
    return membership.failure?.retryable === true ? "RETRY" : "STOP_PERMANENT_FAILURE";
  }
  return "RESUME";
}

function buildPlan(approved, memberships) {
  const byInstrument = new Map(memberships.map((membership) => [membership.instrument_id, membership]));
  const actions = approved.map((instrumentId) => ({
    instrument_id: instrumentId,
    action: actionForMembership(byInstrument.get(instrumentId)),
  }));
  const pilot = approved[0];
  const pilotAction = actions[0]?.action;
  return {
    pilot: { instrument_id: pilot, action: pilotAction },
    remaining_count: Math.max(0, approved.length - 1),
    actions,
    requires_operator: actions
      .filter((action) => action.action === "STOP_DISABLED" || action.action === "STOP_PERMANENT_FAILURE")
      .map((action) => ({ instrument_id: action.instrument_id, status: action.action })),
  };
}

class ApiClient {
  constructor(browser) {
    this.browser = browser;
  }

  async request(method, path, { body = undefined, headers = {}, expectedStatus, phase }) {
    let response;
    try {
      response = await this.browser.request({ method, path, body, headers });
    } catch (error) {
      if (error instanceof LiveAcceptanceError) throw error;
      fail("BROWSER_REQUEST_FAILED", phase);
    }
    if (response === null || typeof response !== "object") fail("BROWSER_REQUEST_FAILED", phase);
    if (response.status !== expectedStatus) responseFailure(response, phase, expectedStatus);
    return response;
  }

  async getSession() {
    const response = await this.request("GET", API_PATHS.session, {
      expectedStatus: 200,
      phase: "authentication",
    });
    const session = validateSession(response.body);
    if (session.role !== "owner") fail("OWNER_MISMATCH", "authentication");
    return session;
  }

  async getCsrf() {
    const response = await this.request("GET", API_PATHS.csrf, {
      expectedStatus: 200,
      phase: "csrf",
    });
    return validateCsrf(response.body);
  }

  async mutate(path, body, idempotencyKey, expectedStatus, phase, method = "POST") {
    const csrf = await this.getCsrf();
    return this.request(method, path, {
      body,
      expectedStatus,
      phase,
      headers: {
        "Content-Type": "application/json",
        "Idempotency-Key": idempotencyKey,
        "X-CSRF-Token": csrf,
      },
    });
  }

  async getMemberships(approved) {
    const response = await this.request("GET", API_PATHS.memberships, {
      expectedStatus: 200,
      phase: "membership-read",
    });
    return validateMembershipList(response.body, approved);
  }

  async getStatus(instrumentId, membershipId) {
    const response = await this.request(
      "GET",
      `${API_PATHS.memberships}/${encodeURIComponent(membershipId)}`,
      { expectedStatus: 200, phase: "lifecycle" },
    );
    return validateStatus(response.body, instrumentId);
  }

  async add(instrumentId) {
    const response = await this.mutate(
      API_PATHS.memberships,
      { instrument_code: codeForInstrument(instrumentId) },
      onboardingIdempotencyKey("add", instrumentId),
      202,
      "onboarding",
    );
    return validateMutation(response.body, instrumentId, "onboarding").resource;
  }

  async retry(instrumentId, membershipId) {
    const response = await this.mutate(
      `${API_PATHS.memberships}/${encodeURIComponent(membershipId)}/retry`,
      {},
      onboardingIdempotencyKey("retry", instrumentId, membershipId),
      202,
      "onboarding",
    );
    return validateMutation(response.body, instrumentId, "onboarding").resource;
  }

  async getQuote(identity, nowMs) {
    const path = `${API_PATHS.quoteCache}/${encodeURIComponent(identity.instrument_id)}/quote?membership_id=${encodeURIComponent(identity.membership_id)}&generation=${encodeURIComponent(String(identity.generation))}`;
    const response = await this.request("GET", path, { expectedStatus: 200, phase: "receipt" });
    return validateQuote(response.body, identity, nowMs);
  }

}

function quoteSummary(quote) {
  if (quote === null) return { version: null, timestamp: null };
  return { version: quote.quote_version, timestamp: quote.last_success_at };
}

function quoteFingerprint(quote) {
  if (quote === null) return null;
  // Keep this only in process. The report intentionally retains versions and
  // timestamps, not quote values, while this catches a same-version mutation.
  return JSON.stringify([
    quote.quote_version,
    quote.last_success_at,
    quote.price,
    quote.base_price,
    quote.change_from_previous_day,
    quote.change_percent_from_previous_day,
    quote.direction,
  ]);
}

function identityFromMembership(membership) {
  if (membership.lifecycle !== "READY" || membership.generation <= 0) return null;
  return {
    membership_id: membership.id,
    instrument_id: membership.instrument_id,
    generation: membership.generation,
  };
}

function quoteIdentityMatches(quote, identity) {
  return (
    quote.membership_id === identity.membership_id &&
    quote.instrument_id === identity.instrument_id &&
    quote.generation === identity.generation
  );
}

function receiptProgress(instrumentId, baseline, receipts, extra = {}) {
  return {
    status: "blocked",
    instrument_id: instrumentId,
    baseline,
    receipts: receipts.map((receipt) => ({ version: receipt.version, timestamp: receipt.timestamp })),
    receipt_count: receipts.length,
    ...extra,
  };
}

async function readDomUntilMatch(ownedQuotePage, expected, deadline, clock, domWaitMs) {
  const domDeadline = Math.min(deadline, clock.now() + domWaitMs);
  while (true) {
    const dom = await ownedQuotePage.readDom(expected.instrument_id);
    if (
      dom !== null &&
      dom.instrument_id === expected.instrument_id &&
      dom.price === expected.price &&
      dom.last_success_at === expected.last_success_at &&
      dom.status_phase === "ready"
    ) {
      return true;
    }
    if (clock.now() >= domDeadline) return false;
    await clock.sleep(Math.min(100, Math.max(0, domDeadline - clock.now())));
  }
}

async function waitForQuiescence(api, identity, clock, timeoutMs, pollMs) {
  if (!Number.isSafeInteger(pollMs) || pollMs <= 0) {
    fail("CLI_ARGUMENT", "cleanup");
  }
  const deadline = clock.now() + timeoutMs;
  let stableSamples = 0;
  let previousMarker = null;
  let lastMarker = null;
  let stableStartedAt = null;
  while (true) {
    const quote = await api.getQuote(identity, clock.now());
    // A null cache entry says nothing about the last producer attempt.  Only
    // an actual, unchanged version/timestamp can show that no late result has
    // appeared across the full live transport-attempt horizon.
    if (quote.reason_code === "NO_ACTIVE_DEMAND" && quote.quote !== null) {
      const marker = quoteSummary(quote.quote);
      const markerKey = JSON.stringify(marker);
      if (markerKey === previousMarker && stableStartedAt !== null) {
        stableSamples += 1;
      } else {
        stableSamples = 1;
        stableStartedAt = clock.now();
      }
      previousMarker = markerKey;
      lastMarker = marker;
      if (clock.now() - stableStartedAt >= REQUIRED_QUIESCENCE_STABLE_MS) {
        return {
          verified: true,
          stable_duration_ms: clock.now() - stableStartedAt,
          stable_samples: stableSamples,
          stable_started_at: stableStartedAt,
          last_marker: lastMarker,
        };
      }
    } else {
      stableSamples = 0;
      previousMarker = null;
      stableStartedAt = null;
    }
    if (clock.now() >= deadline) return false;
    const remainingMs = deadline - clock.now();
    const delayMs = Math.min(pollMs, remainingMs);
    if (delayMs <= 0) return false;
    await clock.sleep(delayMs);
  }
}

async function runReceiptQa(api, ownedQuotePage, membership, options, report) {
  const identity = identityFromMembership(membership);
  if (identity === null) fail("NO_READY_RECEIPT_CANDIDATE", "receipt");
  if (ownedQuotePage === undefined || typeof ownedQuotePage.readDom !== "function" || typeof ownedQuotePage.close !== "function") {
    fail("PREREQUISITE_BROWSER_CONTEXT", "receipt");
  }
  const clock = options.clock;
  const observationStartedAt = options.observationStartedAt;
  if (!Number.isSafeInteger(observationStartedAt) || observationStartedAt < 0) {
    fail("CONTRACT_INVALID", "receipt", { instrument_id: identity.instrument_id });
  }
  const observationStartedAtText = new Date(observationStartedAt).toISOString();
  let baseline = { version: null, timestamp: null };
  let baselineFingerprint = null;
  const receipts = [];
  let ignoredRepetitions = 0;
  let ignoredBeforeStart = 0;
  let ignoredNonReceipts = 0;
  report.receipt = receiptProgress(identity.instrument_id, baseline, receipts, {
    status: "running",
    observation_started_at: observationStartedAtText,
    owned_page_closed: false,
    quiescent: false,
    cleanup_status: "pending",
    ignored_repetitions: 0,
    ignored_before_start: 0,
    ignored_non_receipts: 0,
    dom_api_agreement: false,
  });
  let primaryError = null;
  let cleanupError = null;
  let pageClosed = false;
  const deadline = observationStartedAt + options.observationTimeoutMs;
  const progress = (extra = {}) => receiptProgress(identity.instrument_id, baseline, receipts, {
    status: primaryError === null ? "running" : "blocked",
    observation_started_at: observationStartedAtText,
    owned_page_closed: pageClosed,
    quiescent: report.receipt?.quiescent === true,
    cleanup_status: report.receipt?.cleanup_status ?? "pending",
    ignored_repetitions: ignoredRepetitions,
    ignored_before_start: ignoredBeforeStart,
    ignored_non_receipts: ignoredNonReceipts,
    dom_api_agreement: report.receipt?.dom_api_agreement === true,
    ...extra,
  });
  const sleepForPoll = async () => {
    if (clock.now() >= deadline) {
      fail("RECEIPT_TIMEOUT", "receipt", {
        instrument_id: identity.instrument_id,
        receipt_count: receipts.length,
      });
    }
    await clock.sleep(Math.min(options.pollIntervalMs, Math.max(0, deadline - clock.now())));
  };
  try {
    const baselineResponse = options.baselineResponse;
    if (baselineResponse === undefined || baselineResponse === null) {
      fail("CONTRACT_INVALID", "receipt", { instrument_id: identity.instrument_id });
    }
    if (!quoteIdentityMatches(baselineResponse, identity)) {
      fail("RECEIPT_IDENTITY_MISMATCH", "receipt", { instrument_id: identity.instrument_id });
    }
    baseline = quoteSummary(baselineResponse.quote);
    baselineFingerprint = quoteFingerprint(baselineResponse.quote);
    report.receipt = progress({ baseline });
    while (receipts.length < 2) {
      if (clock.now() >= deadline) {
        fail("RECEIPT_TIMEOUT", "receipt", {
          instrument_id: identity.instrument_id,
          receipt_count: receipts.length,
        });
      }
      const response = await api.getQuote(identity, clock.now());
      if (!quoteIdentityMatches(response, identity)) {
        fail("RECEIPT_IDENTITY_MISMATCH", "receipt", { instrument_id: identity.instrument_id }, progress());
      }
      if (response.quote === null) {
        if (response.reason_code !== null && TERMINAL_RECEIPT_REASONS.has(response.reason_code)) {
          fail("RECEIPT_TERMINAL_REASON", "receipt", {
            instrument_id: identity.instrument_id,
            status: response.reason_code,
          }, progress());
        }
        ignoredNonReceipts += 1;
        report.receipt = progress();
        await sleepForPoll();
        continue;
      }
      const version = BigInt(response.quote.quote_version);
      const timestampMs = Date.parse(response.quote.last_success_at);
      const previousVersion = receipts.length === 0 ? baseline.version : receipts.at(-1).version;
      const previousTimestamp = receipts.length === 0 ? baseline.timestamp : receipts.at(-1).timestamp;
      const previousFingerprint = receipts.length === 0 ? baselineFingerprint : receipts.at(-1).fingerprint;
      const fingerprint = quoteFingerprint(response.quote);
      if (previousVersion !== null) {
        const previousVersionBigInt = BigInt(previousVersion);
        if (version < previousVersionBigInt) {
          fail("RECEIPT_VERSION_REVERSED", "receipt", {
            instrument_id: identity.instrument_id,
            version: response.quote.quote_version,
          }, progress());
        }
        if (version === previousVersionBigInt) {
          if (previousTimestamp !== null) {
            const previousTimestampMs = Date.parse(previousTimestamp);
            if (timestampMs < previousTimestampMs) {
              fail("RECEIPT_TIME_REVERSED", "receipt", {
                instrument_id: identity.instrument_id,
                timestamp: response.quote.last_success_at,
              }, progress());
            }
            if (timestampMs > previousTimestampMs) {
              fail("RECEIPT_VERSION_NOT_ADVANCED", "receipt", {
                instrument_id: identity.instrument_id,
                version: response.quote.quote_version,
              }, progress());
            }
          }
          if (fingerprint !== previousFingerprint) {
            fail("RECEIPT_SAME_VERSION_MUTATION", "receipt", {
              instrument_id: identity.instrument_id,
              version: response.quote.quote_version,
            }, progress());
          }
          if (response.reason_code !== null && TERMINAL_RECEIPT_REASONS.has(response.reason_code)) {
            fail("RECEIPT_TERMINAL_REASON", "receipt", {
              instrument_id: identity.instrument_id,
              status: response.reason_code,
            }, progress());
          }
          // The API may keep returning the exact baseline or last-seen cache
          // while the newly opened widget waits for its first producer cycle.
          // This includes a retained stale/NO_ACTIVE_DEMAND prior-session
          // baseline. It is not a stop and it never counts as a receipt.
          ignoredRepetitions += 1;
          report.receipt = progress();
          await sleepForPoll();
          continue;
        }
      }
      if (previousTimestamp !== null) {
        const previousTimestampMs = Date.parse(previousTimestamp);
        if (timestampMs < previousTimestampMs) {
          fail("RECEIPT_TIME_REVERSED", "receipt", {
            instrument_id: identity.instrument_id,
            timestamp: response.quote.last_success_at,
          }, progress());
        }
        if (timestampMs === previousTimestampMs) {
          fail("RECEIPT_TIME_NOT_ADVANCED", "receipt", {
            instrument_id: identity.instrument_id,
            timestamp: response.quote.last_success_at,
          }, progress());
        }
      }
      if (response.reason_code !== null && TERMINAL_RECEIPT_REASONS.has(response.reason_code)) {
        fail("RECEIPT_TERMINAL_REASON", "receipt", {
          instrument_id: identity.instrument_id,
          status: response.reason_code,
        }, progress());
      }
      // A retained stale quote, NO_ACTIVE_DEMAND, QUOTE_PENDING, or transient
      // producer error is evidence to keep waiting, never an eligible receipt.
      // The observation stays bounded by the original 90-second deadline.
      if (response.freshness !== "RECENT" || response.reason_code !== null) {
        ignoredNonReceipts += 1;
        report.receipt = progress();
        await sleepForPoll();
        continue;
      }
      if (timestampMs <= observationStartedAt) {
        // A RECENT cache entry from before the owned observation is not an
        // accepted receipt, even when no baseline quote was available.
        ignoredBeforeStart += 1;
        report.receipt = progress();
        await sleepForPoll();
        continue;
      }
      const domMatches = await readDomUntilMatch(
        ownedQuotePage,
        {
          instrument_id: identity.instrument_id,
          price: response.quote.price,
          last_success_at: response.quote.last_success_at,
          status_phase: "ready",
        },
        deadline,
        clock,
        options.domWaitMs,
      );
      if (!domMatches) {
        fail("DOM_API_MISMATCH", "receipt", { instrument_id: identity.instrument_id }, progress());
      }
      receipts.push({
        version: response.quote.quote_version,
        timestamp: response.quote.last_success_at,
        fingerprint,
      });
      report.receipt = progress({
        status: "running",
        quiescent: false,
        dom_api_agreement: true,
      });
      if (receipts.length < 2) await sleepForPoll();
    }
  } catch (error) {
    primaryError = asAcceptanceError(error, "receipt");
    primaryError.progress = progress({ status: "blocked" });
  } finally {
    try {
      const closeResult = await ownedQuotePage.close();
      if (closeResult?.closed !== true) fail("CLEANUP_PAGE_UNVERIFIED", "cleanup");
      pageClosed = true;
      report.receipt = progress({ owned_page_closed: true });
    } catch (error) {
      cleanupError = asAcceptanceError(error, "cleanup");
      report.receipt = progress({ cleanup_status: "unverified" });
    }
    if (pageClosed) {
      try {
        const quiescence = await waitForQuiescence(
          api,
          identity,
          clock,
          options.quiescenceTimeoutMs,
          options.pollIntervalMs,
        );
        if (quiescence === false || quiescence.verified !== true) {
          if (cleanupError === null) cleanupError = new LiveAcceptanceError("CLEANUP_NOT_QUIESCENT", "cleanup");
          report.receipt = progress({ cleanup_status: "unverified", quiescent: false });
        } else {
          report.receipt = progress({
            cleanup_status: "verified",
            quiescent: true,
            quiescence_stable_ms: quiescence.stable_duration_ms,
            quiescence_samples: quiescence.stable_samples,
            quiescence_last: quiescence.last_marker,
          });
        }
      } catch (error) {
        cleanupError = cleanupError ?? asAcceptanceError(error, "cleanup");
        report.receipt = progress({ cleanup_status: "unverified", quiescent: false });
      }
    } else if (cleanupError === null) {
      cleanupError = new LiveAcceptanceError("CLEANUP_PAGE_UNVERIFIED", "cleanup");
      report.receipt = progress({ cleanup_status: "unverified", quiescent: false });
    }
  }
  if (primaryError !== null) {
    primaryError.progress = progress({ status: "blocked" });
    throw primaryError;
  }
  if (cleanupError !== null) {
    cleanupError.progress = progress({ status: "blocked" });
    throw cleanupError;
  }
  report.receipt = {
    ...(report.receipt ?? {}),
    ...progress(),
    status: "passed",
    quiescent: true,
    dom_api_agreement: true,
  };
}

async function waitForReady(api, instrumentId, membership, options, report) {
  const deadline = options.clock.now() + options.lifecycleTimeoutMs;
  let current = membership;
  while (true) {
    const status = await api.getStatus(instrumentId, current.id);
    current = status.membership;
    report.onboarding.statuses[instrumentId] = summaryMembership(current);
    if (current.lifecycle === "READY") return current;
    if (current.lifecycle === "DISABLED") {
      fail("DISABLED_MEMBERSHIP", "onboarding", { instrument_id: instrumentId, lifecycle: current.lifecycle });
    }
    if (current.lifecycle === "INSUFFICIENT_HISTORY") {
      fail("INSUFFICIENT_HISTORY", "onboarding", { instrument_id: instrumentId, lifecycle: current.lifecycle });
    }
    if (current.lifecycle === "FAILED") {
      fail(
        current.failure?.retryable === false ? "PERMANENT_FAILURE" : "RETRYABLE_FAILURE",
        "onboarding",
        { instrument_id: instrumentId, lifecycle: current.lifecycle },
      );
    }
    if (options.clock.now() >= deadline) {
      fail("LIFECYCLE_TIMEOUT", "onboarding", { instrument_id: instrumentId, lifecycle: current.lifecycle });
    }
    await options.clock.sleep(Math.min(options.pollIntervalMs, Math.max(0, deadline - options.clock.now())));
  }
}

async function onboardInstrument(api, instrumentId, existing, options, report) {
  let membership = existing;
  if (membership === undefined) {
    membership = await api.add(instrumentId);
  } else if (membership.lifecycle === "READY") {
    report.onboarding.statuses[instrumentId] = summaryMembership(membership);
    return membership;
  } else if (membership.lifecycle === "DISABLED") {
    fail("DISABLED_MEMBERSHIP", "onboarding", { instrument_id: instrumentId, lifecycle: membership.lifecycle });
  } else if (membership.lifecycle === "FAILED") {
    if (membership.failure?.retryable !== true) {
      fail("PERMANENT_FAILURE", "onboarding", { instrument_id: instrumentId, lifecycle: membership.lifecycle });
    }
    membership = await api.retry(instrumentId, membership.id);
  }
  return waitForReady(api, instrumentId, membership, options, report);
}

function initialReport(mode, approved, pilot, plan) {
  return {
    schema_version: ACCEPTANCE_SCHEMA_VERSION,
    mode,
    status: mode === "plan" ? "PLAN_READY" : "RUNNING",
    universe: { count: approved.length },
    pilot,
    plan,
    onboarding: {
      completed_count: 0,
      completed_instruments: [],
      statuses: {},
    },
    receipt: { status: "not-run" },
  };
}

export async function runAcceptance({
  mode = "plan",
  browser,
  universePath = DEFAULT_UNIVERSE_PATH,
  pilot = undefined,
  lifecycleTimeoutMs = DEFAULT_LIFECYCLE_TIMEOUT_MS,
  observationTimeoutMs = DEFAULT_OBSERVATION_TIMEOUT_MS,
  quiescenceTimeoutMs = DEFAULT_QUIESCENCE_TIMEOUT_MS,
  pollIntervalMs = DEFAULT_POLL_INTERVAL_MS,
  domWaitMs = 1_000,
  clock = { now: () => Date.now(), sleep: (ms) => new Promise((resolvePromise) => setTimeout(resolvePromise, ms)) },
}) {
  let report;
  try {
    if (!["plan", "apply"].includes(mode)) fail("CLI_ARGUMENT", "input");
    if (browser === undefined || typeof browser.request !== "function") fail("PREREQUISITE_BROWSER_CONTEXT", "authentication");
    if (typeof browser.origin !== "string") fail("PREREQUISITE_BROWSER_CONTEXT", "authentication");
    let origin;
    try {
      origin = new URL(browser.origin);
    } catch {
      fail("WRONG_ORIGIN", "authentication");
    }
    if (!["http:", "https:"].includes(origin.protocol)) fail("WRONG_ORIGIN", "authentication");
    if (!Number.isInteger(observationTimeoutMs) || observationTimeoutMs < 1 || observationTimeoutMs > DEFAULT_OBSERVATION_TIMEOUT_MS) {
      fail("OBSERVATION_BOUND_EXCEEDED", "input");
    }
    if (
      !Number.isInteger(quiescenceTimeoutMs) ||
      quiescenceTimeoutMs < REQUIRED_QUIESCENCE_STABLE_MS ||
      quiescenceTimeoutMs > DEFAULT_QUIESCENCE_TIMEOUT_MS
    ) {
      fail("QUIESCENCE_BOUND_EXCEEDED", "input");
    }
    if (
      !Number.isInteger(pollIntervalMs) ||
      pollIntervalMs < MIN_PRODUCTION_POLL_INTERVAL_MS ||
      pollIntervalMs > 30_000
    ) {
      fail("CLI_ARGUMENT", "input");
    }
    const approved = await readApprovedUniverse(universePath);
    const normalizedPilot = normalizeInstrumentId(pilot ?? approved[0]);
    if (normalizedPilot === null || !approved.includes(normalizedPilot)) fail("PILOT_INVALID", "input");
    const ordered = [normalizedPilot, ...approved.filter((instrumentId) => instrumentId !== normalizedPilot)];
    const api = new ApiClient(browser);
    await api.getSession();
    const membershipList = await api.getMemberships(approved);
    const plan = buildPlan(ordered, membershipList.memberships);
    report = initialReport(mode, approved, normalizedPilot, plan);
    if (mode === "plan") {
      report.status = "PLAN_READY";
      return report;
    }

    const byInstrument = new Map(membershipList.memberships.map((membership) => [membership.instrument_id, membership]));
    const states = new Map(byInstrument);
    for (const instrumentId of ordered) {
      const ready = await onboardInstrument(api, instrumentId, states.get(instrumentId), {
        clock,
        lifecycleTimeoutMs,
        pollIntervalMs,
      }, report);
      states.set(instrumentId, ready);
      report.onboarding.completed_instruments.push(instrumentId);
      report.onboarding.completed_count = report.onboarding.completed_instruments.length;
    }

    const receiptMembership =
      states.get(normalizedPilot)?.lifecycle === "READY" && states.get(normalizedPilot)?.generation > 0
        ? states.get(normalizedPilot)
        : [...states.values()].find((membership) => membership.lifecycle === "READY" && membership.generation > 0);
    if (receiptMembership === undefined) fail("NO_READY_RECEIPT_CANDIDATE", "receipt");
    if (typeof browser.openOwnedQuotePage !== "function") fail("PREREQUISITE_BROWSER_CONTEXT", "receipt");
    // Capture the cutoff and retained-cache baseline before opening the detail
    // view. The view itself creates/renews the genuine widget demand, so a
    // stale prior-session cache must not be mistaken for a post-start receipt.
    const observationStartedAt = clock.now();
    if (!Number.isSafeInteger(observationStartedAt) || observationStartedAt < 0) {
      fail("CONTRACT_INVALID", "receipt");
    }
    const receiptIdentity = identityFromMembership(receiptMembership);
    if (receiptIdentity === null) fail("NO_READY_RECEIPT_CANDIDATE", "receipt");
    const baselineResponse = await api.getQuote(receiptIdentity, clock.now());
    const ownedQuotePage = await browser.openOwnedQuotePage(receiptMembership.instrument_id);
    await runReceiptQa(
      api,
      ownedQuotePage,
      receiptMembership,
      {
        clock,
        baselineResponse,
        observationStartedAt,
        observationTimeoutMs,
        quiescenceTimeoutMs,
        pollIntervalMs,
        domWaitMs,
      },
      report,
    );
    report.status = "APPLIED";
    return report;
  } catch (error) {
    const typed = asAcceptanceError(error, report?.status === "RUNNING" ? "onboarding" : "tool");
    if (report === undefined) {
      report = {
        schema_version: ACCEPTANCE_SCHEMA_VERSION,
        mode,
        status: "BLOCKED",
        universe: { count: 0 },
        pilot: null,
        plan: null,
        onboarding: { completed_count: 0, completed_instruments: [], statuses: {} },
        receipt: { status: "not-run" },
      };
    }
    report.status = "BLOCKED";
    report.error = errorReport(typed);
    if (typed.progress !== undefined) {
      report.receipt = {
        ...(report.receipt ?? {}),
        ...typed.progress,
        status: "blocked",
      };
    }
    return report;
  }
}

// Kept browser-evaluable: the owned-page runtime and the local rendered-DOM
// boundary test execute this exact extractor against a real Document.
export function extractQuoteDom(expectedInstrument, rootDocument = globalThis.document) {
  const document = rootDocument;
  if (document === null || typeof document?.querySelector !== "function") return null;
  const isVisible = (element) => {
    if (element === null || element === undefined) return false;
    const view = element.ownerDocument?.defaultView;
    if (view === null || view === undefined || typeof view.getComputedStyle !== "function") return true;
    const style = view.getComputedStyle(element);
    if (style.display === "none" || style.visibility === "hidden") return false;
    if (typeof element.getBoundingClientRect !== "function") return true;
    const box = element.getBoundingClientRect();
    return box.width > 0 && box.height > 0;
  };
  const board = document.querySelector('[data-testid="stock-beta-detail-board"]');
  if (board === null) return null;
  const instrumentWidget = board.querySelector('[data-testid="stock-beta-detail-widget-instrument-header"]');
  const instrumentTitle = instrumentWidget?.querySelector("h2, h3")?.textContent?.trim() ?? null;
  if (instrumentTitle !== expectedInstrument) return null;
  const currentWidget = board.querySelector(
    '[data-testid="stock-beta-detail-widget-current-quote"][data-widget-id="current-quote"]',
  );
  const frame = currentWidget?.querySelector("section[data-state]") ?? currentWidget?.querySelector("section");
  const quote = frame?.querySelector('[data-testid="stock-beta-current-quote"]');
  const price = quote?.querySelector("[data-quote-value]")?.getAttribute("data-quote-value") ?? null;
  const lastSuccessAt = quote?.querySelector("[data-last-success-at]")?.getAttribute("data-last-success-at") ?? null;
  const statusPhase = frame?.querySelector("[data-status-phase]")?.getAttribute("data-status-phase") ?? null;
  if (
    !isVisible(currentWidget) ||
    !isVisible(frame) ||
    !isVisible(quote) ||
    !isVisible(frame?.querySelector("[data-status-phase]")) ||
    price === null ||
    !/^[1-9]\d*(?:\.\d{1,8})?$/.test(price) ||
    lastSuccessAt === null ||
    !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{1,9})?Z$/.test(lastSuccessAt) ||
    !Number.isFinite(Date.parse(lastSuccessAt)) ||
    statusPhase !== "ready"
  ) {
    return null;
  }
  return {
    instrument_id: instrumentTitle,
    last_success_at: lastSuccessAt,
    price,
    status_phase: statusPhase,
  };
}

export function createPlaywrightBoundary({ page, origin, requestTimeoutMs = 10_000 }) {
  if (
    page === undefined ||
    typeof page.evaluate !== "function" ||
    typeof page.url !== "function" ||
    typeof page.context !== "function"
  ) {
    fail("PREREQUISITE_BROWSER_CONTEXT", "authentication");
  }
  let expectedOrigin;
  try {
    expectedOrigin = new URL(origin);
  } catch {
    fail("WRONG_ORIGIN", "authentication");
  }
  if (!["http:", "https:"].includes(expectedOrigin.protocol)) fail("WRONG_ORIGIN", "authentication");
  const checkOrigin = (candidate, phase) => {
    let current;
    try {
      current = new URL(candidate);
    } catch {
      fail("BROWSER_PAGE_UNAVAILABLE", phase);
    }
    if (!["http:", "https:"].includes(current.protocol)) fail("BROWSER_PAGE_UNAVAILABLE", phase);
    if (current.origin !== expectedOrigin.origin) fail("WRONG_ORIGIN", phase);
    return current;
  };
  const checkPageOrigin = () => checkOrigin(page.url(), "authentication");
  const sameOriginPath = (path, phase) => {
    let target;
    try {
      target = new URL(path, expectedOrigin.origin);
    } catch {
      fail("WRONG_ORIGIN", phase);
    }
    if (target.origin !== expectedOrigin.origin) fail("WRONG_ORIGIN", phase);
    return `${target.pathname}${target.search}`;
  };
  checkPageOrigin();
  return {
    origin: expectedOrigin.origin,
    async request({ method, path, body, headers }) {
      checkPageOrigin();
      const requestPath = sameOriginPath(path, "browser");
      let result;
      try {
        result = await page.evaluate(
          async ({ requestMethod, requestPath: pathForFetch, requestBody, requestHeaders, timeoutMs }) => {
            const controller = new AbortController();
            const timeout = setTimeout(() => controller.abort(), timeoutMs);
            try {
              const init = {
                method: requestMethod,
                credentials: "same-origin",
                cache: "no-store",
                redirect: "error",
                headers: { ...requestHeaders },
                signal: controller.signal,
              };
              if (requestBody !== undefined) {
                init.headers["Content-Type"] = "application/json";
                init.body = JSON.stringify(requestBody);
              }
              const response = await fetch(pathForFetch, init);
              let responseBody = null;
              const contentType = response.headers.get("content-type") ?? "";
              if (contentType.toLowerCase().includes("application/json")) {
                try {
                  responseBody = await response.json();
                } catch {
                  responseBody = null;
                }
              }
              return {
                body: responseBody,
                redirected: response.redirected,
                status: response.status,
                url: response.url,
              };
            } finally {
              clearTimeout(timeout);
            }
          },
          {
            requestBody: body,
            requestHeaders: headers,
            requestMethod: method,
            requestPath,
            timeoutMs: requestTimeoutMs,
          },
        );
      } catch {
        fail("BROWSER_REQUEST_FAILED", "browser");
      }
      let finalUrl;
      try {
        finalUrl = new URL(result.url);
      } catch {
        fail("WRONG_ORIGIN", "browser");
      }
      if (result.redirected || finalUrl.origin !== expectedOrigin.origin) fail("WRONG_ORIGIN", "browser");
      return { body: result.body, status: result.status };
    },
    async openOwnedQuotePage(instrumentId) {
      const normalizedInstrument = normalizeInstrumentId(instrumentId);
      if (normalizedInstrument === null) fail("PILOT_INVALID", "receipt");
      checkPageOrigin();
      const context = page.context();
      if (context === null || typeof context.newPage !== "function") fail("PREREQUISITE_BROWSER_CONTEXT", "receipt");
      let ownedPage = null;
      let navigationViolation = false;
      const targetPath = `/stock-beta/${encodeURIComponent(normalizedInstrument)}`;
      const targetUrl = new URL(targetPath, expectedOrigin.origin).toString();
      try {
        ownedPage = await context.newPage();
        if (
          typeof ownedPage.route !== "function" ||
          typeof ownedPage.goto !== "function" ||
          typeof ownedPage.url !== "function" ||
          typeof ownedPage.evaluate !== "function" ||
          typeof ownedPage.close !== "function" ||
          typeof ownedPage.isClosed !== "function"
        ) {
          fail("PREREQUISITE_BROWSER_CONTEXT", "receipt");
        }
        await ownedPage.route("**/*", async (route) => {
          const request = route.request();
          let requestUrl;
          try {
            requestUrl = new URL(request.url());
          } catch {
            navigationViolation = true;
            await route.abort("blockedbyclient");
            return;
          }
          const foreignOrigin = requestUrl.origin !== expectedOrigin.origin;
          const redirectedNavigation =
            typeof request.isNavigationRequest === "function" &&
            request.isNavigationRequest() &&
            typeof request.redirectedFrom === "function" &&
            request.redirectedFrom() !== null;
          if (foreignOrigin || redirectedNavigation) {
            navigationViolation = true;
            await route.abort("blockedbyclient");
            return;
          }
          await route.continue();
        });
        const response = await ownedPage.goto(targetUrl, {
          timeout: requestTimeoutMs,
          waitUntil: "domcontentloaded",
        });
        if (navigationViolation) fail("WRONG_ORIGIN", "receipt");
        const current = checkOrigin(ownedPage.url(), "receipt");
        if (current.pathname !== targetPath || (response?.status?.() ?? 200) >= 400) {
          fail("BROWSER_NAVIGATION_FAILED", "receipt");
        }
        let closed = false;
        return {
          async readDom(expectedInstrument) {
            if (closed) fail("BROWSER_PAGE_UNAVAILABLE", "receipt");
            const currentPage = checkOrigin(ownedPage.url(), "receipt");
            if (currentPage.pathname !== targetPath) fail("WRONG_ORIGIN", "receipt");
            try {
              return await ownedPage.evaluate(extractQuoteDom, expectedInstrument);
            } catch {
              fail("BROWSER_DOM_UNAVAILABLE", "receipt");
            }
          },
          async close() {
            if (closed) return { closed: true };
            try {
              await ownedPage.close({ reason: "WP-2 owned quote view cleanup", runBeforeUnload: false });
            } catch {
              fail("CLEANUP_PAGE_UNVERIFIED", "cleanup");
            }
            if (!ownedPage.isClosed()) fail("CLEANUP_PAGE_UNVERIFIED", "cleanup");
            closed = true;
            return { closed: true };
          },
        };
      } catch (error) {
        if (ownedPage !== null && !ownedPage.isClosed()) {
          try {
            await ownedPage.close({ reason: "WP-2 partial quote view cleanup", runBeforeUnload: false });
          } catch {
            // Preserve the primary navigation/authentication error. The page
            // close itself is reported only when the acceptance page exists.
          }
        }
        if (error instanceof LiveAcceptanceError) throw error;
        if (navigationViolation) fail("WRONG_ORIGIN", "receipt");
        fail("BROWSER_NAVIGATION_FAILED", "receipt");
      }
    },
  };
}

async function importChromium() {
  const specifiers = [
    "playwright",
    "@playwright/test",
    resolve(PROJECT_ROOT, "apps/web/node_modules/playwright/index.mjs"),
    resolve(PROJECT_ROOT, "apps/web/node_modules/@playwright/test/index.mjs"),
  ];
  for (const specifier of specifiers) {
    try {
      const module = await import(specifier);
      if (module.chromium !== undefined) return module.chromium;
    } catch {
      // Try the next installed package location without exposing loader details.
    }
  }
  fail("PREREQUISITE_PLAYWRIGHT_UNAVAILABLE", "browser-attach");
}

// Playwright 1.61 exposes Browser.close(), not Browser.disconnect(). For a
// CDP-connected browser the documented operation clears contexts created by
// the connected Browser object and disconnects from the browser server. The
// externally launched default context is deliberately never closed here;
// refuse a missing/ambiguous API instead of silently leaking a live connection.
export async function disposeAttachedBrowser(browser) {
  if (
    browser === null ||
    typeof browser !== "object" ||
    typeof browser.close !== "function" ||
    typeof browser.isConnected !== "function"
  ) {
    fail("PREREQUISITE_PLAYWRIGHT_UNAVAILABLE", "browser-attach");
  }
  if (!browser.isConnected()) return { detached: true };
  try {
    await browser.close({ reason: "WP-2 CDP client detach" });
  } catch {
    fail("BROWSER_DETACH_FAILED", "browser-attach");
  }
  if (browser.isConnected()) fail("BROWSER_DETACH_FAILED", "browser-attach");
  return { detached: true };
}

export async function attachExistingOwnerBrowser({ cdpUrl, origin, contextIndex = 0, pageIndex = 0 }) {
  if (typeof cdpUrl !== "string" || cdpUrl.length === 0) fail("PREREQUISITE_BROWSER_CONTEXT", "browser-attach");
  let expectedOrigin;
  try {
    expectedOrigin = new URL(origin);
  } catch {
    fail("WRONG_ORIGIN", "browser-attach");
  }
  if (!["http:", "https:"].includes(expectedOrigin.protocol)) fail("WRONG_ORIGIN", "browser-attach");
  const chromium = await importChromium();
  let browser;
  try {
    browser = await chromium.connectOverCDP(cdpUrl);
  } catch {
    fail("BROWSER_ATTACH_FAILED", "browser-attach");
  }
  if (
    typeof browser?.contexts !== "function" ||
    typeof browser?.isConnected !== "function" ||
    typeof browser?.close !== "function"
  ) {
    await disposeAttachedBrowser(browser);
    fail("PREREQUISITE_PLAYWRIGHT_UNAVAILABLE", "browser-attach");
  }
  let context;
  let page;
  try {
    const contexts = browser.contexts();
    context = contexts[contextIndex];
    page = context?.pages?.()[pageIndex];
  } catch {
    await disposeAttachedBrowser(browser);
    fail("BROWSER_PAGE_UNAVAILABLE", "browser-attach");
  }
  if (context === undefined || page === undefined) {
    await disposeAttachedBrowser(browser);
    fail("BROWSER_PAGE_UNAVAILABLE", "browser-attach");
  }
  try {
    const boundary = createPlaywrightBoundary({ page, origin: expectedOrigin.origin });
    return {
      boundary,
      disconnect: () => disposeAttachedBrowser(browser),
    };
  } catch (error) {
    await disposeAttachedBrowser(browser);
    throw error;
  }
}

function usage() {
  return [
    "Usage:",
    "  node scripts/qa/owner-equity-v2-live-acceptance.mjs --cdp-url <url> --origin <origin>",
    "  node scripts/qa/owner-equity-v2-live-acceptance.mjs --apply --cdp-url <url> --origin <origin>",
    "",
    "Default mode is --plan. --apply performs the pilot first, then remaining symbols sequentially.",
    "The CDP endpoint must expose an existing authenticated Owner browser context; this tool never launches one.",
    "Receipt QA opens one tool-owned same-origin detail page; the original Owner tab is never navigated or closed.",
    "",
    "Options:",
    "  --plan                         Read and report the plan (default; no mutations).",
    "  --apply                        Apply onboarding and bounded receipt QA.",
    "  --cdp-url <url>                Existing browser CDP endpoint (required).",
    "  --origin <origin>              Exact same-origin API/UI origin (required).",
    "  --pilot <code|code.KRX>         Pilot instrument (default: first approved symbol).",
    "  --universe <path>               Checked-in V1 universe path override.",
    "  --context-index <n>             Existing CDP context index (default: 0).",
    "  --page-index <n>                Existing page index (default: 0).",
    "  --poll-ms <n>                   Lifecycle/receipt poll interval (5000-30000; default: 5000).",
    "  --lifecycle-timeout-seconds <n> Lifecycle wait bound (default: 900).",
    "  --observation-seconds <n>       Receipt bound, at most 90 (default: 90).",
    "  Cleanup uses a separate 60-second horizon: NO_ACTIVE_DEMAND and an unchanged quote marker must persist for 30 seconds.",
    "  --help                         Show this usage text.",
  ].join("\n");
}

function parseIntegerOption(value, option) {
  if (!/^\d+$/.test(value)) fail("CLI_ARGUMENT", "input");
  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed)) fail("CLI_ARGUMENT", "input");
  return parsed;
}

export function parseCli(argv) {
  const options = {
    mode: "plan",
    cdpUrl: undefined,
    origin: undefined,
    pilot: undefined,
    universePath: DEFAULT_UNIVERSE_PATH,
    contextIndex: 0,
    pageIndex: 0,
    pollMs: DEFAULT_POLL_INTERVAL_MS,
    lifecycleTimeoutSeconds: DEFAULT_LIFECYCLE_TIMEOUT_MS / 1_000,
    observationSeconds: DEFAULT_OBSERVATION_TIMEOUT_MS / 1_000,
    help: false,
  };
  let explicitMode = null;
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    const next = () => {
      const value = argv[index + 1];
      if (value === undefined || value.startsWith("--")) fail("CLI_ARGUMENT", "input");
      index += 1;
      return value;
    };
    if (argument === "--help" || argument === "-h") options.help = true;
    else if (argument === "--plan" || argument === "--apply") {
      const requestedMode = argument === "--apply" ? "apply" : "plan";
      if (explicitMode !== null && explicitMode !== requestedMode) fail("CLI_ARGUMENT", "input");
      explicitMode = requestedMode;
      options.mode = requestedMode;
    }
    else if (argument === "--cdp-url") options.cdpUrl = next();
    else if (argument === "--origin") options.origin = next();
    else if (argument === "--pilot") options.pilot = next();
    else if (argument === "--universe") options.universePath = resolve(next());
    else if (argument === "--context-index") options.contextIndex = parseIntegerOption(next(), argument);
    else if (argument === "--page-index") options.pageIndex = parseIntegerOption(next(), argument);
    else if (argument === "--poll-ms") options.pollMs = parseIntegerOption(next(), argument);
    else if (argument === "--lifecycle-timeout-seconds") options.lifecycleTimeoutSeconds = parseIntegerOption(next(), argument);
    else if (argument === "--observation-seconds") options.observationSeconds = parseIntegerOption(next(), argument);
    else fail("CLI_ARGUMENT", "input");
  }
  if (options.help) return options;
  if (options.cdpUrl === undefined || options.origin === undefined) fail("PREREQUISITE_BROWSER_CONTEXT", "input");
  if (options.observationSeconds < 1 || options.observationSeconds > 90) fail("OBSERVATION_BOUND_EXCEEDED", "input");
  if (options.pollMs < MIN_PRODUCTION_POLL_INTERVAL_MS || options.pollMs > 30_000) {
    fail("CLI_ARGUMENT", "input");
  }
  return options;
}

function printJson(value) {
  process.stdout.write(`${JSON.stringify(value)}\n`);
}

export async function main(argv = process.argv.slice(2)) {
  try {
    const cli = parseCli(argv);
    if (cli.help) {
      process.stdout.write(`${usage()}\n`);
      return 0;
    }
    const attached = await attachExistingOwnerBrowser({
      cdpUrl: cli.cdpUrl,
      origin: cli.origin,
      contextIndex: cli.contextIndex,
      pageIndex: cli.pageIndex,
    });
    try {
      const report = await runAcceptance({
        browser: attached.boundary,
        lifecycleTimeoutMs: cli.lifecycleTimeoutSeconds * 1_000,
        mode: cli.mode,
        observationTimeoutMs: cli.observationSeconds * 1_000,
        pilot: cli.pilot,
        pollIntervalMs: cli.pollMs,
        universePath: cli.universePath,
      });
      printJson(report);
      return report.status === "BLOCKED" ? 1 : 0;
    } finally {
      await attached.disconnect();
    }
  } catch (error) {
    printJson({
      schema_version: ACCEPTANCE_SCHEMA_VERSION,
      mode: "unknown",
      status: "BLOCKED",
      error: errorReport(asAcceptanceError(error, "cli")),
    });
    return 1;
  }
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const exitCode = await main();
  process.exitCode = exitCode;
}
