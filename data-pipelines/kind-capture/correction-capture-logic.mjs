import {
  isCalendarDate,
  isExpectedResponseUrl,
  parseExpectedCaptureRequest,
} from './capture-logic.mjs';

export const CORRECTION_ENTRY_URL =
  'https://kind.krx.co.kr/disclosure/disclosurebystocktype.do?method=searchDisclosureByStockTypeEtf';
export const CORRECTION_SURFACE = 'etf-disclosure-correction-viewer';
export const CORRECTION_ARTIFACT_KIND = 'rendered_dom_snapshot';
export const CORRECTION_VIEWER_ORIGIN_PATH = '/common/disclsviewer.do';
export const CORRECTION_CONFIRM = 'KIND_CORRECTION_EVIDENCE_CAPTURE';
export const MAX_CORRECTION_RESPONSE_BODY_BYTES = 1024 * 1024;

export const CORRECTION_TERMINATION = Object.freeze({
  VIEWER_LOADED: 'viewer_loaded',
  MISSING_TARGET: 'missing_target',
  DUPLICATE_TARGET: 'duplicate_target',
  NO_RESPONSE: 'no_response',
  NO_POPUP: 'no_popup',
  VIEWER_URL_INVALID: 'viewer_url_invalid',
  MAIN_DOC_MISSING: 'main_doc_missing',
  MAIN_DOC_DUPLICATE: 'main_doc_duplicate',
  MAIN_DOC_EMPTY: 'main_doc_empty',
});

const ACCEPTANCE_RE = /^[0-9]{14}$/;
const VIEWER_AUTHORITY_RE = /^https:\/\/([^/?#]*)/;

export function validateCorrectionCliArgs(options = {}) {
  if (!isCalendarDate(options.from)) {
    return { ok: false, error: 'invalid_from' };
  }
  if (!isCalendarDate(options.to)) {
    return { ok: false, error: 'invalid_to' };
  }
  if (options.from > options.to) {
    return { ok: false, error: 'invalid_date_range' };
  }
  if (!ACCEPTANCE_RE.test(options.acceptance ?? '')) {
    return { ok: false, error: 'invalid_acceptance' };
  }
  if (typeof options.out !== 'string' || options.out.length === 0) {
    return { ok: false, error: 'invalid_out' };
  }
  if (options.confirm !== CORRECTION_CONFIRM) {
    return { ok: false, error: 'invalid_confirmation' };
  }
  return {
    ok: true,
    value: {
      from: options.from,
      to: options.to,
      acceptance: options.acceptance,
      out: options.out,
      confirm: options.confirm,
    },
  };
}

export function validateCorrectionResponseExpectation(expected = {}) {
  return (
    isCalendarDate(expected.fromDate) &&
    isCalendarDate(expected.toDate) &&
    expected.fromDate <= expected.toDate &&
    expected.pageIndex === 1 &&
    ACCEPTANCE_RE.test(expected.acceptance ?? '')
  );
}

// Reuse the list-capture contract rather than trusting the page currently
// showing the rendered rows. The method, exact response URL, observed KIND
// surface fields, requested dates, and page 1 must all match.
export function matchExplicitCorrectionResponse(response = {}, expected = {}) {
  if (response.method !== 'POST' || !isExpectedResponseUrl(response.url)) return null;
  const pageIndex = expected.pageIndex ?? 1;
  if (pageIndex !== 1) return null;
  const formFields = parseExpectedCaptureRequest(response.postData, {
    fromDate: expected.fromDate,
    toDate: expected.toDate,
    pageIndex,
  });
  return formFields === null ? null : { pageIndex, formFields };
}

export function extractCorrectionAcceptance(onclick) {
  if (typeof onclick !== 'string') {
    return null;
  }
  const match = /^openDisclsViewer\('([0-9]{14})',''\)$/.exec(onclick);
  return match?.[1] ?? null;
}

// Broader recognition is used only to detect ambiguity. A handler found here
// is never approved for clicking unless it also satisfies the exact empty
// second-argument contract above.
export function extractCorrectionViewerAcceptance(onclick) {
  if (typeof onclick !== 'string' || onclick.length > 512) {
    return null;
  }
  const match = /^openDisclsViewer\('([0-9]{14})',[\s\S]*\)$/.exec(onclick);
  return match?.[1] ?? null;
}

function extractQuotedAttribute(tagHtml, attributeName) {
  const escapedName = attributeName.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const pattern = new RegExp(
    `(?:^|[\\t\\n\\r ])${escapedName}[\\t\\n\\r ]*=[\\t\\n\\r ]*([\\\"'])([\\s\\S]*?)\\1`,
    'i',
  );
  return pattern.exec(tagHtml)?.[2] ?? null;
}

// This follows the existing KIND normalizer's deliberately small HTML
// scanner: locate real <a> opening tags, respect quoted attribute values when
// finding '>', and parse only the quoted onclick attribute. It is not a
// general-purpose HTML parser.
export function extractCorrectionAnchorsFromHtml(html) {
  if (typeof html !== 'string') return [];
  const anchors = [];
  const openingAnchor = /<a(?=[\s/>])/gi;
  let match;
  while ((match = openingAnchor.exec(html)) !== null) {
    const start = match.index;
    let quote = null;
    let end = -1;
    for (let index = start + match[0].length; index < html.length; index += 1) {
      const character = html[index];
      if (quote !== null) {
        if (character === quote) quote = null;
      } else if (character === '\"' || character === "'") {
        quote = character;
      } else if (character === '>') {
        end = index + 1;
        break;
      }
    }
    if (end < 0) break;
    const onclick = extractQuotedAttribute(html.slice(start, end), 'onclick');
    if (onclick !== null) anchors.push({ onclick });
    openingAnchor.lastIndex = end;
  }
  return anchors;
}

export function decodeCorrectionResponseBody(body) {
  if (!(body instanceof Uint8Array) || body.byteLength > MAX_CORRECTION_RESPONSE_BODY_BYTES) {
    return null;
  }
  try {
    return new TextDecoder('utf-8', { fatal: true }).decode(body);
  } catch {
    return null;
  }
}

export function findUniqueCorrectionAnchor(anchors, acceptance) {
  if (!Array.isArray(anchors) || !ACCEPTANCE_RE.test(acceptance ?? '')) {
    return { kind: 'missing', termination: CORRECTION_TERMINATION.MISSING_TARGET };
  }

  const matches = anchors
    .map((anchor, index) => ({ anchor, index }))
    .filter(({ anchor }) => {
      const onclick = typeof anchor === 'string' ? anchor : anchor?.onclick;
      return extractCorrectionViewerAcceptance(onclick) === acceptance;
    });

  if (matches.length === 0) {
    return { kind: 'missing', termination: CORRECTION_TERMINATION.MISSING_TARGET };
  }
  const distinctOnclicks = new Set(
    matches.map(({ anchor }) => (typeof anchor === 'string' ? anchor : anchor.onclick)),
  );
  if (distinctOnclicks.size > 1) {
    return { kind: 'duplicate', termination: CORRECTION_TERMINATION.DUPLICATE_TARGET };
  }
  const onlyOnclick = distinctOnclicks.values().next().value;
  if (extractCorrectionAcceptance(onlyOnclick) !== acceptance) {
    return { kind: 'missing', termination: CORRECTION_TERMINATION.MISSING_TARGET };
  }
  return {
    kind: 'unique',
    index: matches[0].index,
    onclick: typeof matches[0].anchor === 'string' ? matches[0].anchor : matches[0].anchor.onclick,
    anchor: matches[0].anchor,
    occurrences: matches.length,
  };
}

export function findUniqueCorrectionTargetInResponseBody(body, acceptance) {
  const html = decodeCorrectionResponseBody(body);
  if (html === null) {
    return { kind: 'no_response', termination: CORRECTION_TERMINATION.NO_RESPONSE };
  }
  return findUniqueCorrectionAnchor(extractCorrectionAnchorsFromHtml(html), acceptance);
}

export function classifyCorrectionResponseBody({ status, body, acceptance } = {}) {
  if (status !== 'fulfilled') {
    return { kind: 'no_response', termination: CORRECTION_TERMINATION.NO_RESPONSE };
  }
  return findUniqueCorrectionTargetInResponseBody(body, acceptance);
}

// The response body supplies the expected handler string. Live DOM matching
// is repeated by one page-context evaluate immediately before click; a
// changed, missing, or duplicated live target is never guessed by index.
export function classifyLiveCorrectionAnchors(anchors, expectedOnclick) {
  const acceptance = extractCorrectionAcceptance(expectedOnclick);
  if (!Array.isArray(anchors) || acceptance === null) {
    return { kind: 'missing', termination: CORRECTION_TERMINATION.MISSING_TARGET };
  }
  const matches = anchors.filter((anchor) => {
    const onclick = typeof anchor === 'string' ? anchor : anchor?.onclick;
    return extractCorrectionViewerAcceptance(onclick) === acceptance;
  });
  if (matches.length === 0) {
    return { kind: 'missing', termination: CORRECTION_TERMINATION.MISSING_TARGET };
  }
  const distinctOnclicks = new Set(
    matches.map((anchor) => (typeof anchor === 'string' ? anchor : anchor.onclick)),
  );
  if (distinctOnclicks.size > 1) {
    return { kind: 'duplicate', termination: CORRECTION_TERMINATION.DUPLICATE_TARGET };
  }
  if (distinctOnclicks.values().next().value !== expectedOnclick) {
    return { kind: 'missing', termination: CORRECTION_TERMINATION.MISSING_TARGET };
  }
  return { kind: 'unique' };
}

export function validateCorrectionOutputContract({
  outputExists = false,
  outputIsSymlink = false,
  parentIsDirectory = false,
  parentIsSymlink = false,
  resolvedParentIsDirectory = false,
  resolvedParentIsSymlink = false,
  resolvedParentMatches = false,
} = {}) {
  if (outputExists || outputIsSymlink) {
    return { ok: false, error: 'output_must_not_exist' };
  }
  if (
    !parentIsDirectory ||
    !resolvedParentIsDirectory ||
    parentIsSymlink ||
    resolvedParentIsSymlink ||
    !resolvedParentMatches
  ) {
    return { ok: false, error: 'parent_must_be_real_directory' };
  }
  return { ok: true };
}

export function correctionPublishPlan(complete) {
  return complete ? ['viewer.html', 'capture.json'] : ['capture.json'];
}

export function shouldCleanupCorrectionOutputDirectory({ published = false } = {}) {
  return published !== true;
}

export function isExpectedCorrectionViewerUrl(rawUrl) {
  if (typeof rawUrl !== 'string' || rawUrl.length === 0) {
    return false;
  }

  let parsed;
  try {
    parsed = new URL(rawUrl);
  } catch {
    return false;
  }

  // Keep the authority comparison raw as well as checking URL fields. WHATWG
  // URL normalizes an explicit default port, but the capture contract rejects
  // every explicit port. Query semantics are intentionally not inspected.
  const authority = VIEWER_AUTHORITY_RE.exec(rawUrl)?.[1];
  return (
    parsed.protocol === 'https:' &&
    parsed.hostname === 'kind.krx.co.kr' &&
    parsed.pathname === CORRECTION_VIEWER_ORIGIN_PATH &&
    parsed.username === '' &&
    parsed.password === '' &&
    parsed.port === '' &&
    parsed.hash === '' &&
    authority === 'kind.krx.co.kr'
  );
}

function isMainDocSelect(element) {
  if (!element || typeof element !== 'object') {
    return false;
  }
  const tagName = typeof element.tagName === 'string' ? element.tagName.toUpperCase() : '';
  return tagName === 'SELECT' && (element.id === 'mainDoc' || element.name === 'mainDoc');
}

export function mainDocTermination(selects) {
  const matches = Array.isArray(selects) ? selects.filter(isMainDocSelect) : [];
  if (matches.length > 1) {
    return CORRECTION_TERMINATION.MAIN_DOC_DUPLICATE;
  }
  if (matches.length === 0) return CORRECTION_TERMINATION.MAIN_DOC_MISSING;
  return Number.isInteger(matches[0].optionCount) && matches[0].optionCount >= 1
    ? CORRECTION_TERMINATION.VIEWER_LOADED
    : CORRECTION_TERMINATION.MAIN_DOC_EMPTY;
}

export function isCleanCorrectionTermination(termination) {
  return termination === CORRECTION_TERMINATION.VIEWER_LOADED;
}
