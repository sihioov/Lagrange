#!/usr/bin/env node
// KIND correction-evidence viewer capture.
//
// This is deliberately a browser-control path. It submits no reconstructed
// request and never navigates directly to a viewer URL. One invocation handles
// exactly one acceptance number and stages only the viewer's rendered DOM.
import { chromium } from 'playwright';
import {
  closeSync,
  constants as fsConstants,
  fstatSync,
  lstatSync,
  openSync,
  realpathSync,
  writeFileSync,
} from 'node:fs';
import { basename, dirname, join, resolve } from 'node:path';
import {
  clearTimeout as clearControllerTimeout,
  setTimeout as setControllerTimeout,
} from 'node:timers';
import {
  CORRECTION_ARTIFACT_KIND,
  CORRECTION_CONFIRM,
  CORRECTION_ENTRY_URL,
  CORRECTION_SURFACE,
  CORRECTION_TERMINATION,
  CORRECTION_VIEWER_ORIGIN_PATH,
  MAX_CORRECTION_RESPONSE_BODY_BYTES,
  classifyCorrectionResponseBody,
  classifyLiveCorrectionAnchors,
  correctionPublishPlan,
  isCleanCorrectionTermination,
  isExpectedCorrectionViewerUrl,
  matchExplicitCorrectionResponse,
  mainDocTermination,
  shouldCleanupCorrectionOutputDirectory,
  validateCorrectionOutputContract,
  validateCorrectionCliArgs,
  validateCorrectionResponseExpectation,
} from './correction-capture-logic.mjs';
import {
  assertCurrentCorrectionOutputDirectory,
  cleanupCorrectionOutputDirectory,
  reserveCorrectionOutputDirectory,
} from './correction-output.mjs';

const SEARCH_WAIT_MS = 6000;
const ENTRY_RENDER_WAIT_MS = 2500;
const LIVE_TARGET_WAIT_MS = 6000;
const LIVE_TARGET_EVALUATION_WAIT_MS = 500;
const BROWSER_CLOSE_WAIT_MS = 1000;
const POPUP_WAIT_MS = 10000;
const VIEWER_LOAD_WAIT_MS = 10000;
const VIEWER_FILE = 'viewer.html';

function usage(message) {
  console.error(
    `${message}\n\nusage: node capture-correction.mjs --from YYYY-MM-DD --to YYYY-MM-DD ` +
      '--acceptance 14_ASCII_DIGITS --out DIR --confirm KIND_CORRECTION_EVIDENCE_CAPTURE',
  );
  process.exit(2);
}

function parseArgs(argv) {
  if (argv.length % 2 !== 0) {
    usage('each option must have one value');
  }
  const values = {};
  const known = new Set(['from', 'to', 'acceptance', 'out', 'confirm']);
  for (let i = 0; i < argv.length; i += 2) {
    const rawKey = argv[i];
    const value = argv[i + 1];
    if (!rawKey?.startsWith('--') || value === undefined) {
      usage('malformed command line');
    }
    const key = rawKey.slice(2);
    if (!known.has(key) || Object.hasOwn(values, key)) {
      usage('unknown or repeated option');
    }
    values[key] = value;
  }

  const checked = validateCorrectionCliArgs(values);
  if (!checked.ok) {
    const messages = {
      invalid_from: '--from must be YYYY-MM-DD',
      invalid_to: '--to must be YYYY-MM-DD',
      invalid_date_range: '--from must be on or before --to',
      invalid_acceptance: '--acceptance must be exactly 14 ASCII digits',
      invalid_out: '--out is required',
      invalid_confirmation: `--confirm must equal ${CORRECTION_CONFIRM}`,
    };
    usage(messages[checked.error] ?? 'invalid command line');
  }
  return checked.value;
}

async function invokeSiteSearch(page) {
  try {
    return await page.evaluate(() => {
      if (typeof fnSearch !== 'function') return false;
      fnSearch();
      return true;
    });
  } catch {
    return false;
  }
}

function createExplicitResponseTracker(page, expected) {
  if (!validateCorrectionResponseExpectation(expected)) {
    throw new TypeError('invalid correction response expectation');
  }
  let completedResponses = 0;
  const completedResults = [];
  const waiters = new Set();
  const pendingBodyTasks = new Set();

  const noResponse = () => ({
    kind: 'no_response',
    termination: CORRECTION_TERMINATION.NO_RESPONSE,
  });

  const settleBodyTask = (task, result) => {
    if (task.settled) return;
    task.settled = true;
    pendingBodyTasks.delete(task);
    completedResponses += 1;
    completedResults.push(result);
    for (const waiter of waiters) {
      if (completedResponses > waiter.baseline) waiter.resolve(result);
    }
  };

  const onResponse = (response) => {
    let request;
    try {
      request = response.request();
    } catch {
      return;
    }
    let matched;
    try {
      matched = matchExplicitCorrectionResponse(
        {
          method: request.method(),
          url: response.url(),
          postData: request.postData() ?? '',
        },
        expected,
      );
    } catch {
      return;
    }
    if (!matched) return;

    const bodyTask = { settled: false };
    pendingBodyTasks.add(bodyTask);
    try {
      const contentLength = response.headers()['content-length'];
      if (/^[0-9]+$/.test(contentLength ?? '') && Number(contentLength) > MAX_CORRECTION_RESPONSE_BODY_BYTES) {
        settleBodyTask(bodyTask, noResponse());
        return;
      }
    } catch {
      // The body-size validator below remains authoritative if headers are
      // unavailable or use a transfer encoding without a length.
    }
    let bodyPromise;
    try {
      // Read only the already-matched response body. The bytes stay in this
      // task long enough for strict UTF-8/size/anchor validation, then are
      // discarded; they never enter staging, metadata, or logs.
      bodyPromise = response.body();
    } catch {
      settleBodyTask(bodyTask, noResponse());
      return;
    }
    Promise.resolve(bodyPromise).then(
      (body) => {
        let result;
        try {
          result = {
            ...classifyCorrectionResponseBody({
              status: 'fulfilled',
              body,
              acceptance: expected.acceptance,
            }),
            responseBodySize: body.byteLength,
            formFieldCount: matched.formFields.length,
          };
        } catch {
          result = noResponse();
        }
        settleBodyTask(bodyTask, result);
      },
      () => settleBodyTask(bodyTask, noResponse()),
    );
  };

  page.on('response', onResponse);
  return {
    completedCount: () => completedResponses,
    async waitForNext(baseline, waitOnce) {
      if (completedResponses > baseline) return completedResults[baseline];
      let resolve;
      const responsePromise = new Promise((settle) => {
        resolve = settle;
      });
      const waiter = { baseline, resolve };
      waiters.add(waiter);
      const timeoutPromise = Promise.resolve()
        .then(() => waitOnce())
        .then(
          () => ({ kind: 'timeout', pendingBody: pendingBodyTasks.size > 0 }),
          () => ({ kind: 'timeout', pendingBody: pendingBodyTasks.size > 0 }),
        );
      const result = await Promise.race([responsePromise, timeoutPromise]);
      waiters.delete(waiter);
      return result;
    },
    dispose() {
      page.off('response', onResponse);
    },
  };
}

async function findTargetAfterSearch(page, acceptance, responseTracker, invokeSearch) {
  let baseline = responseTracker.completedCount();
  if (!(await invokeSearch())) {
    return { kind: 'no_response', termination: CORRECTION_TERMINATION.NO_RESPONSE };
  }
  for (let attempt = 0; attempt < 2; attempt += 1) {
    const bodyResult = await responseTracker.waitForNext(
      baseline,
      () => page.waitForTimeout(SEARCH_WAIT_MS),
    );
    if (bodyResult.kind === 'unique' || bodyResult.kind === 'duplicate') {
      return bodyResult;
    }
    if (bodyResult.kind === 'no_response') {
      return bodyResult;
    }
    if (bodyResult.kind === 'timeout' && bodyResult.pendingBody) {
      return { kind: 'no_response', termination: CORRECTION_TERMINATION.NO_RESPONSE };
    }
    if (bodyResult.kind !== 'missing' && attempt === 1) {
      return { kind: 'no_response', termination: CORRECTION_TERMINATION.NO_RESPONSE };
    }
    if (bodyResult.kind === 'missing' && attempt === 1) return bodyResult;

    // The only retry is issued after a bounded wait with no exact response or
    // a valid response body that contains no target. A body task that remains
    // pending is terminal no_response rather than an unbounded wait.
    baseline = responseTracker.completedCount();
    if (!(await invokeSearch())) {
      return { kind: 'no_response', termination: CORRECTION_TERMINATION.NO_RESPONSE };
    }
  }
  return { kind: 'missing', termination: CORRECTION_TERMINATION.MISSING_TARGET };
}

function evaluateLiveTarget(page, expectedOnclick, click) {
  return page.evaluate(({ expectedOnclick: expectedRaw, click: shouldClick }) => {
    const expected = /^openDisclsViewer\('([0-9]{14})',''\)$/.exec(expectedRaw);
    if (!expected) return { kind: 'missing' };
    const acceptance = expected[1];
    const anchors = [...document.querySelectorAll('a[onclick]')];
    const candidates = anchors.filter((anchor) => {
      const onclick = anchor.getAttribute('onclick') ?? '';
      const parsed = /^openDisclsViewer\('([0-9]{14})',[\s\S]*\)$/.exec(onclick);
      return parsed?.[1] === acceptance;
    });
    const distinctOnclicks = new Set(
      candidates.map((anchor) => anchor.getAttribute('onclick')),
    );
    if (distinctOnclicks.size > 1) return { kind: 'duplicate' };
    if (
      candidates.length === 0 ||
      distinctOnclicks.values().next().value !== expectedRaw
    ) {
      return { kind: 'missing' };
    }
    if (shouldClick) candidates[0].click();
    return {
      kind: 'unique',
      onclick: candidates[0].getAttribute('onclick'),
      occurrences: candidates.length,
    };
  }, { expectedOnclick, click });
}

async function evaluateWithControllerTimeout(page, expectedOnclick, click, waitMs) {
  let timeoutId;
  const evaluation = evaluateLiveTarget(page, expectedOnclick, click).then(
    (value) => ({ state: 'resolved', value }),
    () => ({ state: 'rejected' }),
  );
  const timeout = new Promise((resolveTimeout) => {
    timeoutId = setControllerTimeout(() => resolveTimeout({ state: 'timeout' }), waitMs);
  });
  const result = await Promise.race([evaluation, timeout]);
  clearControllerTimeout(timeoutId);
  return result;
}

async function cancelOutstandingPageTask(page) {
  try {
    const browser = page.context().browser();
    if (browser) {
      let timeoutId;
      const closeResult = browser.close().then(
        () => ({ state: 'closed' }),
        () => ({ state: 'rejected' }),
      );
      const timeout = new Promise((resolveTimeout) => {
        timeoutId = setControllerTimeout(
          () => resolveTimeout({ state: 'timeout' }),
          BROWSER_CLOSE_WAIT_MS,
        );
      });
      const result = await Promise.race([closeResult, timeout]);
      clearControllerTimeout(timeoutId);
      return result.state === 'closed' && !browser.isConnected() && page.isClosed();
    }
    let timeoutId;
    const closeResult = page.close({ runBeforeUnload: false }).then(
      () => ({ state: 'closed' }),
      () => ({ state: 'rejected' }),
    );
    const timeout = new Promise((resolveTimeout) => {
      timeoutId = setControllerTimeout(
        () => resolveTimeout({ state: 'timeout' }),
        BROWSER_CLOSE_WAIT_MS,
      );
    });
    const result = await Promise.race([closeResult, timeout]);
    clearControllerTimeout(timeoutId);
    return result.state === 'closed' && page.isClosed();
  } catch {
    return false;
  }
}

function terminationForLiveTarget(liveTarget) {
  if (liveTarget?.kind === 'duplicate') return CORRECTION_TERMINATION.DUPLICATE_TARGET;
  if (liveTarget?.kind === 'missing') return CORRECTION_TERMINATION.MISSING_TARGET;
  return liveTarget?.kind === 'unique' ? null : CORRECTION_TERMINATION.NO_RESPONSE;
}

async function clickForViewer(page, target) {
  // Poll readiness without clicking. Controller time is the bound authority;
  // each page task is synchronous and read-only until the final recheck below.
  const deadline = process.hrtime.bigint() + BigInt(LIVE_TARGET_WAIT_MS) * 1_000_000n;
  let liveTarget = { kind: 'missing' };
  while (true) {
    const remainingNs = deadline - process.hrtime.bigint();
    if (remainingNs <= 0n) break;
    const remainingMs = Math.max(1, Number(remainingNs / 1_000_000n));
    const evaluated = await evaluateWithControllerTimeout(
      page,
      target.onclick,
      false,
      Math.min(remainingMs, LIVE_TARGET_EVALUATION_WAIT_MS),
    );
    if (evaluated.state === 'timeout') {
      const publishSafe = await cancelOutstandingPageTask(page);
      return {
        viewer: null,
        termination: CORRECTION_TERMINATION.NO_RESPONSE,
        publishSafe,
      };
    }
    if (evaluated.state === 'rejected') {
      return { viewer: null, termination: CORRECTION_TERMINATION.NO_RESPONSE };
    }
    liveTarget = evaluated.value;
    if (liveTarget.kind !== 'missing') break;
    const afterEvaluationNs = deadline - process.hrtime.bigint();
    if (afterEvaluationNs <= 0n) break;
    const delayMs = Math.min(50, Math.max(1, Number(afterEvaluationNs / 1_000_000n)));
    await new Promise((resolveDelay) => setControllerTimeout(resolveDelay, delayMs));
  }

  const readinessTermination = terminationForLiveTarget(liveTarget);
  if (readinessTermination !== null) {
    return { viewer: null, termination: readinessTermination };
  }
  const liveClassification = classifyLiveCorrectionAnchors([liveTarget], target.onclick);
  if (liveClassification.kind !== 'unique') {
    return {
      viewer: null,
      termination: liveClassification.termination ?? CORRECTION_TERMINATION.NO_RESPONSE,
    };
  }

  // Install the popup waiter only after readiness. The following page task
  // revalidates all same-acceptance handlers and clicks one exact current node
  // atomically; no snapshot index or direct viewer call is used.
  const popupPromise = page
    .waitForEvent('popup', { timeout: POPUP_WAIT_MS })
    .catch(() => null);
  const clicked = await evaluateWithControllerTimeout(
    page,
    target.onclick,
    true,
    LIVE_TARGET_EVALUATION_WAIT_MS,
  );
  if (clicked.state === 'timeout') {
    const publishSafe = await cancelOutstandingPageTask(page);
    return {
      viewer: null,
      termination: CORRECTION_TERMINATION.NO_RESPONSE,
      publishSafe,
    };
  }
  if (clicked.state === 'rejected') {
    return { viewer: null, termination: CORRECTION_TERMINATION.NO_RESPONSE };
  }
  const clickTermination = terminationForLiveTarget(clicked.value);
  if (clickTermination !== null) {
    return { viewer: null, termination: clickTermination };
  }
  const clickedClassification = classifyLiveCorrectionAnchors([clicked.value], target.onclick);
  if (clickedClassification.kind !== 'unique') {
    return {
      viewer: null,
      termination: clickedClassification.termination ?? CORRECTION_TERMINATION.NO_RESPONSE,
    };
  }

  const viewer = await popupPromise;
  if (!viewer) {
    return { viewer: null, termination: CORRECTION_TERMINATION.NO_POPUP };
  }
  return { viewer, termination: null };
}

function captureMetadata(
  options,
  termination,
  terminationStage,
  responseDiagnostics,
  retrievedAt,
  file,
) {
  const metadata = {
    source: 'kind.krx.co.kr',
    entry_url: CORRECTION_ENTRY_URL,
    surface: CORRECTION_SURFACE,
    requested_range: { from: options.from, to: options.to },
    anchor_acceptance_number: options.acceptance,
    viewer_origin_path: CORRECTION_VIEWER_ORIGIN_PATH,
    artifact_kind: CORRECTION_ARTIFACT_KIND,
    retrieved_at: retrievedAt,
    termination,
    termination_stage: terminationStage,
  };
  if (responseDiagnostics !== null) {
    metadata.response_diagnostics = {
      body_size: responseDiagnostics.responseBodySize,
      form_field_count: responseDiagnostics.formFieldCount,
      target_handler_occurrences: responseDiagnostics.occurrences ?? 0,
    };
  }
  if (file !== undefined) metadata.file = file;
  return metadata;
}

function inspectCorrectionOutputPath(rawOut) {
  const outputPath = resolve(rawOut);
  const parentPath = dirname(outputPath);
  let outputExists = false;
  let outputIsSymlink = false;
  try {
    const outputStat = lstatSync(outputPath);
    outputExists = true;
    outputIsSymlink = outputStat.isSymbolicLink();
  } catch (error) {
    if (error?.code !== 'ENOENT') {
      console.error('cannot inspect the requested output path');
      process.exit(2);
    }
  }

  let parentStat;
  let resolvedParent;
  let resolvedParentStat;
  try {
    parentStat = lstatSync(parentPath);
    resolvedParent = realpathSync(parentPath);
    resolvedParentStat = lstatSync(resolvedParent);
  } catch {
    console.error('output parent must be an existing real directory');
    process.exit(2);
  }
  const checked = validateCorrectionOutputContract({
    outputExists,
    outputIsSymlink,
    parentIsDirectory: parentStat.isDirectory(),
    parentIsSymlink: parentStat.isSymbolicLink(),
    resolvedParentIsDirectory: resolvedParentStat.isDirectory(),
    resolvedParentIsSymlink: resolvedParentStat.isSymbolicLink(),
    resolvedParentMatches: resolvedParent === parentPath,
  });
  if (!checked.ok) {
    console.error(
      checked.error === 'output_must_not_exist'
        ? 'output directory must not already exist'
        : 'output parent must be an existing real directory',
    );
    process.exit(2);
  }
  let parentFd;
  try {
    parentFd = openSync(
      resolvedParent,
      fsConstants.O_RDONLY | fsConstants.O_DIRECTORY | fsConstants.O_NOFOLLOW,
    );
    const openedParentStat = fstatSync(parentFd);
    if (
      !openedParentStat.isDirectory() ||
      openedParentStat.dev !== resolvedParentStat.dev ||
      openedParentStat.ino !== resolvedParentStat.ino
    ) {
      throw new Error('output parent changed while it was opened');
    }
  } catch {
    if (parentFd !== undefined) closeSync(parentFd);
    console.error('output parent must remain the same real directory');
    process.exit(2);
  }
  return {
    outputPath,
    outputName: basename(outputPath),
    parentPath: resolvedParent,
    parentFd,
    anchoredParentPath: `/proc/self/fd/${parentFd}`,
    parentDevice: resolvedParentStat.dev,
    parentInode: resolvedParentStat.ino,
  };
}

const options = parseArgs(process.argv.slice(2));
let output = inspectCorrectionOutputPath(options.out);
try {
  // mkdir is the atomic no-replace operation for the final name. Keep the
  // reserved directory open and address its files through that descriptor;
  // capture.json is written last and acts as the consumer commit marker.
  output = reserveCorrectionOutputDirectory(output);
} catch {
  try {
    closeSync(output.parentFd);
  } catch {
    // The reservation error remains authoritative.
  }
  console.error('could not reserve a new output directory');
  process.exit(2);
}

let browser = null;
let viewer = null;
let renderedViewerBytes = null;
let retrievedAt = null;
let responseTracker = null;
let termination = CORRECTION_TERMINATION.NO_RESPONSE;
let terminationStage = 'browser_entry';
let responseDiagnostics = null;
let publishSafe = true;

try {
  browser = await chromium.launch();
  const context = await browser.newContext({ locale: 'ko-KR', timezoneId: 'Asia/Seoul' });
  const page = await context.newPage();

  // This is the only top-level navigation. The viewer is opened by the
  // rendered anchor and observed through this opener page's popup event.
  await page.goto(CORRECTION_ENTRY_URL, { waitUntil: 'domcontentloaded', timeout: 60000 });
  // Match the existing list capture's bounded grace period: KIND initializes
  // its date controls and fnSearch after DOMContentLoaded.
  await page.waitForTimeout(ENTRY_RENDER_WAIT_MS);
  for (const [selector, value] of [
    ['#fromDate', options.from],
    ['#toDate', options.to],
  ]) {
    const field = page.locator(selector);
    await field.fill('');
    await field.type(value, { delay: 12 });
  }

  responseTracker = createExplicitResponseTracker(page, {
    fromDate: options.from,
    toDate: options.to,
    pageIndex: 1,
    acceptance: options.acceptance,
  });
  // Initial search plus at most one retry are the only calls to fnSearch. The
  // target comes from the exact response body; live DOM is checked again only
  // inside the click task immediately before the site control is clicked.
  const target = await findTargetAfterSearch(
    page,
    options.acceptance,
    responseTracker,
    () => invokeSiteSearch(page),
  );
  responseDiagnostics = target;
  terminationStage = 'response_body';
  termination = target.termination ?? null;
  if (target.kind === 'unique') {
    terminationStage = 'live_target';
    const opened = await clickForViewer(page, target);
    publishSafe = opened.publishSafe !== false;
    termination = opened.termination;
    viewer = opened.viewer;
    if (viewer) {
      terminationStage = 'viewer';
      try {
        await viewer.waitForLoadState('domcontentloaded', { timeout: VIEWER_LOAD_WAIT_MS });
        // Only the URL shape is checked. Query contents come from the
        // browser and are intentionally neither recorded nor interpreted.
        if (!isExpectedCorrectionViewerUrl(viewer.url())) {
          termination = CORRECTION_TERMINATION.VIEWER_URL_INVALID;
        } else {
          const selects = await viewer.locator('select').evaluateAll((nodes) =>
            nodes.map((node) => ({
              tagName: node.tagName,
              id: node.id,
              name: node.getAttribute('name') ?? '',
              optionCount: node.options.length,
            })),
          );
          termination = mainDocTermination(selects);
          if (isCleanCorrectionTermination(termination)) {
            // page.content() is the rendered DOM serialization. Keep its
            // UTF-8 encoding exact and timestamp it immediately afterward.
            const snapshot = await viewer.content();
            retrievedAt = new Date().toISOString();
            renderedViewerBytes = Buffer.from(snapshot, 'utf8');
          }
        }
      } catch {
        termination = CORRECTION_TERMINATION.NO_RESPONSE;
      }
    }
  }
} catch {
  termination = CORRECTION_TERMINATION.NO_RESPONSE;
} finally {
  responseTracker?.dispose();
  if (viewer) {
    await viewer.close().catch(() => {});
  }
  if (browser && publishSafe) {
    await browser.close().catch(() => {});
  }
}

// Incomplete captures have no DOM snapshot; use a terminal metadata time for
// them. A complete capture keeps the timestamp taken immediately after
// viewer.content() and never replaces it here.
const metadataRetrievedAt = retrievedAt ?? new Date().toISOString();
let exitCode = 0;
let published = false;
if (!publishSafe) {
  const cleaned = cleanupCorrectionOutputDirectory(output);
  try {
    closeSync(output.outputDirectoryFd);
  } catch {
    // The process exits immediately below; publication is already forbidden.
  }
  try {
    closeSync(output.parentFd);
  } catch {
    // The process exits immediately below; publication is already forbidden.
  }
  console.error('correction capture aborted because browser cancellation was not confirmed');
  if (!cleaned) console.error('correction capture cleanup incomplete');
  process.exit(1);
} else try {
  const complete = isCleanCorrectionTermination(termination) && renderedViewerBytes !== null;
  const metadata = captureMetadata(
    options,
    termination,
    terminationStage,
    responseDiagnostics,
    metadataRetrievedAt,
    complete ? VIEWER_FILE : undefined,
  );
  const metadataBytes = `${JSON.stringify(metadata, null, 2)}\n`;
  const files = correctionPublishPlan(complete);
  if (files.includes(VIEWER_FILE)) {
    writeFileSync(join(output.anchoredOutputPath, VIEWER_FILE), renderedViewerBytes, {
      flag: 'wx',
    });
  }
  // A consumer rejects a directory without capture.json. Verify the reserved
  // name immediately before committing metadata, write that marker last, and
  // verify the same opened directory once more before declaring publication.
  assertCurrentCorrectionOutputDirectory(output);
  writeFileSync(join(output.anchoredOutputPath, 'capture.json'), metadataBytes, { flag: 'wx' });
  assertCurrentCorrectionOutputDirectory(output);
  published = true;
  if (!complete) {
    console.error(`correction capture incomplete (${termination}); do not ingest`);
    exitCode = 1;
  } else {
    console.log(`staged correction viewer for ${options.acceptance}`);
  }
} catch {
  const cleaned = shouldCleanupCorrectionOutputDirectory({ published })
    ? cleanupCorrectionOutputDirectory(output)
    : true;
  console.error('correction capture failed while writing staging metadata');
  if (!cleaned) console.error('correction capture cleanup incomplete');
  exitCode = 1;
}
try {
  closeSync(output.outputDirectoryFd);
} catch {
  console.error('correction capture could not close its anchored output directory');
  exitCode = 1;
}
try {
  closeSync(output.parentFd);
} catch {
  console.error('correction capture could not close its anchored output directory');
  exitCode = 1;
}
process.exitCode = exitCode;
