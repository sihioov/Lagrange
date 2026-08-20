export const TERMINATION = Object.freeze({
  CLAMPED_DUPLICATE: 'clamped_duplicate',
  PAGE_BOUND_REACHED: 'page_bound_reached',
  ADVANCE_CONTROL_MISSING: 'advance_control_missing',
  NO_RESPONSE: 'no_response',
});

export const CAPTURE_CONFIRM = 'KIND_ETF_DISCLOSURE_CAPTURE';

const CAPTURE_DATE = /^\d{4}-\d{2}-\d{2}$/;

export function isCalendarDate(value) {
  if (!CAPTURE_DATE.test(value ?? '')) return false;
  const year = Number(value.slice(0, 4));
  const month = Number(value.slice(5, 7));
  const day = Number(value.slice(8, 10));
  if (month < 1 || month > 12 || day < 1) return false;
  const leap = year % 400 === 0 || (year % 4 === 0 && year % 100 !== 0);
  const daysInMonth = [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
  return day <= daysInMonth[month - 1];
}

// Keep the operator gate in a side-effect-free helper so tests can prove that
// invalid invocations are rejected before capture.mjs launches a browser.
export function validateCaptureCliArgs(options, defaultMaxPages = 40) {
  if (!options || typeof options !== 'object') return { ok: false, error: 'invalid_options' };
  if (!isCalendarDate(options.from)) return { ok: false, error: 'invalid_from' };
  if (!isCalendarDate(options.to)) return { ok: false, error: 'invalid_to' };
  if (options.from > options.to) return { ok: false, error: 'invalid_date_range' };
  if (typeof options.out !== 'string' || options.out.length === 0) {
    return { ok: false, error: 'invalid_out' };
  }
  if (options.confirm !== CAPTURE_CONFIRM) {
    return { ok: false, error: 'invalid_confirmation' };
  }

  const maxPages = options['max-pages'] === undefined
    ? defaultMaxPages
    : Number(options['max-pages']);
  if (!Number.isInteger(maxPages) || maxPages < 1 || maxPages > defaultMaxPages) {
    return { ok: false, error: 'invalid_max_pages' };
  }
  return {
    ok: true,
    value: {
      from: options.from,
      to: options.to,
      out: options.out,
      maxPages,
      confirm: options.confirm,
    },
  };
}

// A response is admitted only when its request form is the one expected by
// the current browser action.  A duplicate response with the same request
// metadata and bytes is harmless; every other duplicate is ambiguous.
export const CAPTURE_RESPONSE = Object.freeze({
  IGNORED: 'ignored',
  STORED: 'stored',
  DUPLICATE: 'duplicate',
  CONFLICT: 'conflict',
});

const SURFACE_FIELDS = Object.freeze([
  ['method', 'searchDisclosureByStockTypeEtfSub'],
  ['forward', 'disclosurebystocktype_etf_sub'],
]);

const EXPECTED_RESPONSE_URL = 'https://kind.krx.co.kr/disclosure/disclosurebystocktype.do';

export const ISSUED_PAGE_STATUS = Object.freeze({
  ISSUED: 'issued',
  STORED: 'stored',
  CONFLICT: 'conflict',
});

// The browser must report the one observed KIND response surface exactly.
// `href` equality additionally rejects URL normalization (for example an
// explicit default port, trailing query marker, or case variation) that is
// outside the approved request contract.
export function isExpectedResponseUrl(responseUrl) {
  if (typeof responseUrl !== 'string') return false;
  let parsed;
  try {
    parsed = new URL(responseUrl);
  } catch {
    return false;
  }
  return (
    responseUrl === EXPECTED_RESPONSE_URL &&
    parsed.protocol === 'https:' &&
    parsed.hostname === 'kind.krx.co.kr' &&
    parsed.pathname === '/disclosure/disclosurebystocktype.do' &&
    parsed.port === '' &&
    parsed.username === '' &&
    parsed.password === '' &&
    parsed.search === '' &&
    parsed.hash === '' &&
    parsed.href === EXPECTED_RESPONSE_URL
  );
}

// Ordered, URL-decoded form fields as the page sent them.  Returning null on
// a decode error is intentional: a request whose form cannot be decoded is
// not admissible evidence.  Pairs remain an array so duplicate names and
// their order survive into capture.json.
export function parseFormFields(postData) {
  if (postData === undefined || postData === null || postData === '') return [];
  if (typeof postData !== 'string') return null;

  const fields = [];
  for (const pair of postData.split('&')) {
    if (pair === '') continue;
    const separator = pair.indexOf('=');
    const rawName = separator === -1 ? pair : pair.slice(0, separator);
    const rawValue = separator === -1 ? '' : pair.slice(separator + 1);
    try {
      const decode = (value) => decodeURIComponent(value.replace(/\+/g, ' '));
      fields.push([decode(rawName), decode(rawValue)]);
    } catch {
      return null;
    }
  }
  return fields;
}

function hasExactFieldOnce(formFields, name, expectedValue) {
  const matches = formFields.filter(([fieldName]) => fieldName === name);
  return matches.length === 1 && matches[0][1] === expectedValue;
}

function isPositivePageIndex(value) {
  return typeof value === 'string' && /^[1-9]\d*$/.test(value) && Number.isSafeInteger(Number(value));
}

// Identifies a request by its decoded form, without depending on the page the
// browser most recently attempted.  This lets a delayed response carry its
// own issued page index through the capture state.
export function identifyCaptureRequest(formFields, { fromDate, toDate } = {}) {
  if (!Array.isArray(formFields)) return null;
  if (typeof fromDate !== 'string' || typeof toDate !== 'string') return null;

  const expected = [
    ...SURFACE_FIELDS,
    ['fromDate', fromDate],
    ['toDate', toDate],
  ];
  if (!expected.every(([name, value]) => hasExactFieldOnce(formFields, name, value))) return null;

  const pageFields = formFields.filter(([name]) => name === 'pageIndex');
  if (pageFields.length !== 1 || !isPositivePageIndex(pageFields[0][1])) return null;
  return { pageIndex: Number(pageFields[0][1]), formFields };
}

// Checks the decoded ordered fields for one exact explicit request.  The
// surface fields are observed in the KIND capture contract/runbook; they are
// required here so a late request from another disclosure surface cannot win.
export function isExpectedCaptureRequest(formFields, { fromDate, toDate, pageIndex } = {}) {
  if (!isPositivePageIndex(String(pageIndex))) return false;
  const identified = identifyCaptureRequest(formFields, { fromDate, toDate });
  return identified !== null && identified.pageIndex === Number(pageIndex);
}

// Parses once and returns the exact ordered array used by the predicate and,
// if matched, by the captured record.  A null result means the response must
// be ignored.
export function parseExpectedCaptureRequest(postData, expected) {
  const formFields = parseFormFields(postData);
  return formFields !== null && isExpectedCaptureRequest(formFields, expected) ? formFields : null;
}

export function parseCaptureRequest(postData, expectedRange) {
  const formFields = parseFormFields(postData);
  return formFields === null ? null : identifyCaptureRequest(formFields, expectedRange);
}

function sameFormFields(left, right) {
  if (!Array.isArray(left) || !Array.isArray(right) || left.length !== right.length) return false;
  return left.every(
    ([name, value], index) => right[index]?.[0] === name && right[index]?.[1] === value,
  );
}

function sameBytes(left, right) {
  if (left === right) return true;
  if (!left || !right || typeof left.length !== 'number' || left.length !== right.length) return false;
  for (let index = 0; index < left.length; index += 1) {
    if (left[index] !== right[index]) return false;
  }
  return true;
}

// Stores a matched response without allowing a racing response to overwrite
// it.  The only safe first-response-wins case is identical request metadata
// and identical bytes.  A conflict leaves the first bytes in place but gives
// the caller a typed result so the capture can terminate incomplete.
export function recordCapturedResponse(captured, pageIndex, record) {
  const existing = captured.get(pageIndex);
  if (!existing) {
    captured.set(pageIndex, record);
    return CAPTURE_RESPONSE.STORED;
  }
  if (!sameFormFields(existing.formFields, record.formFields) || !sameBytes(existing.body, record.body)) {
    return CAPTURE_RESPONSE.CONFLICT;
  }
  return CAPTURE_RESPONSE.DUPLICATE;
}

// Explicit browser-capture state. `issuedPages` remains populated until the
// whole capture closes, so a response for page N that arrives after page N+1
// was issued can still be compared with the already stored page-N record.
export function createCaptureState({ fromDate, toDate } = {}) {
  return {
    fromDate,
    toDate,
    active: false,
    issuedPages: new Map(),
    captured: new Map(),
    failure: null,
  };
}

export function startCapture(state) {
  if (!state || state.failure !== null) return false;
  state.active = true;
  return true;
}

// Reissuing the same page is the bounded retry for its existing contract; it
// does not create a second contract or discard the first response.
export function issueCapturePage(state, pageIndex) {
  if (!state?.active || state.failure !== null || !isPositivePageIndex(String(pageIndex))) return false;
  const numericPageIndex = Number(pageIndex);
  if (!state.issuedPages.has(numericPageIndex)) {
    state.issuedPages.set(numericPageIndex, { status: ISSUED_PAGE_STATUS.ISSUED });
  }
  return true;
}

export function closeCaptureState(state) {
  if (state) state.active = false;
  return state;
}

// Pure request-to-contract lookup.  It is intentionally separate from body
// reading so the response event can reject an as-yet-unissued page before an
// asynchronous body read completes.
export function matchIssuedCaptureRequest(state, { url, postData } = {}) {
  if (!state?.active || !isExpectedResponseUrl(url)) return null;
  const parsed = parseCaptureRequest(postData, { fromDate: state.fromDate, toDate: state.toDate });
  if (!parsed || !state.issuedPages.has(parsed.pageIndex)) return null;
  return parsed;
}

export function recordIssuedResponse(state, { pageIndex, formFields }, { body, retrievedAt } = {}) {
  if (!state?.active) return CAPTURE_RESPONSE.IGNORED;
  const contract = state.issuedPages.get(pageIndex);
  if (!contract) return CAPTURE_RESPONSE.IGNORED;

  const outcome = recordCapturedResponse(state.captured, pageIndex, { body, retrievedAt, formFields });
  if (outcome === CAPTURE_RESPONSE.STORED) contract.status = ISSUED_PAGE_STATUS.STORED;
  if (outcome === CAPTURE_RESPONSE.CONFLICT) {
    contract.status = ISSUED_PAGE_STATUS.CONFLICT;
    state.failure = CAPTURE_RESPONSE.CONFLICT;
  }
  return outcome;
}

export function captureIssuedResponse(state, response) {
  const parsed = matchIssuedCaptureRequest(state, response);
  if (!parsed) return CAPTURE_RESPONSE.IGNORED;
  return recordIssuedResponse(state, parsed, response);
}

// Pure response admission used by capture.mjs and the race tests.  The body
// is never interpreted; it is retained only as bytes after the form matches.
export function captureExpectedResponse(captured, { url, postData, body, retrievedAt }, expected) {
  if (!isExpectedResponseUrl(url)) return CAPTURE_RESPONSE.IGNORED;
  const formFields = parseExpectedCaptureRequest(postData, expected);
  if (formFields === null) return CAPTURE_RESPONSE.IGNORED;
  return recordCapturedResponse(captured, Number(expected.pageIndex), { body, retrievedAt, formFields });
}

// Tracks only already-created response-body promises.  It never starts a
// request or adds a timer; the caller controls when an existing task resolves.
export function createPendingTaskTracker() {
  return new Set();
}

export function trackPendingTask(pendingTasks, task) {
  const tracked = Promise.resolve(task);
  pendingTasks.add(tracked);
  tracked.then(
    () => pendingTasks.delete(tracked),
    () => pendingTasks.delete(tracked),
  );
  return tracked;
}

// Give already-settled tasks their microtask turns to remove themselves.  An
// unresolved body promise is deliberately not awaited: false makes the page
// incomplete and lets the caller close the capture fail-closed.
export async function settlePendingTasks(pendingTasks) {
  await Promise.resolve();
  await Promise.resolve();
  return pendingTasks.size === 0;
}

export function isCleanTermination(termination) {
  return termination === TERMINATION.CLAMPED_DUPLICATE;
}

export function terminationForAdvance(moved) {
  return moved ? null : TERMINATION.ADVANCE_CONTROL_MISSING;
}

// Wait once, then issue exactly one retry and wait once more. Both callbacks
// deliberately have no browser dependency so the bounded retry contract is
// unit-testable.
export async function waitForCapturedPage(
  captured,
  pageIndex,
  waitOnce,
  retry,
  failed = () => false,
  settle = async () => true,
) {
  await waitOnce(0);
  const firstSettled = await settle();
  if (!firstSettled) return false;
  if (failed()) return false;
  if (captured.has(pageIndex)) return true;
  await retry();
  await waitOnce(1);
  const secondSettled = await settle();
  if (!secondSettled) return false;
  if (failed()) return false;
  return captured.has(pageIndex);
}

// Classifies one successfully requested page. A byte-identical response is
// KIND's only observed terminal condition. A distinct response beyond the
// configured stored-page bound is retained nowhere, but proves truncation.
export function classifyReceivedPage({ captured, pageIndex, maxPages }) {
  const current = captured.get(pageIndex);
  const previous = captured.get(pageIndex - 1);
  if (!current || !previous) throw new Error('captured pages must be contiguous before classification');
  if (current.body.equals(previous.body)) {
    captured.delete(pageIndex);
    return TERMINATION.CLAMPED_DUPLICATE;
  }
  if (pageIndex > maxPages) {
    captured.delete(pageIndex);
    return TERMINATION.PAGE_BOUND_REACHED;
  }
  return null;
}
