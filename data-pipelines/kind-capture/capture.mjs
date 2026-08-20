#!/usr/bin/env node
// KIND ETF disclosure capture stage.
//
// KIND has no API. Its search runs only in a browser: the form page serves real
// HTML, but the search POST is refused outside the page because the endpoint
// depends on state the page's JavaScript produces. ADR-0004 D11 therefore fixes
// the mechanism — drive the site's OWN controls in a browser engine, never a
// reconstructed request — and this script is that stage.
//
// It writes a staging directory that `kind-raw` (Rust) ingests into the
// immutable Raw zone. The split is deliberate: a browser cannot be trusted to
// report what it retrieved, so this stage records only what it did and the bytes
// it received, and the Rust side recomputes every content hash from those bytes.
//
// What it does NOT do: interpret, filter, or reshape the response. Selection by
// 종목명 happens later at normalization, where it is reversible. Raw holds
// provider bytes.
//
// Usage:
//   node capture.mjs --from 2020-02-03 --to 2020-02-07 --out /path/to/staging
//                    [--max-pages 40]
import { chromium } from 'playwright';
import { mkdirSync, writeFileSync, existsSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
import {
  TERMINATION,
  classifyReceivedPage,
  closeCaptureState,
  createCaptureState,
  createPendingTaskTracker,
  issueCapturePage,
  isCleanTermination,
  matchIssuedCaptureRequest,
  recordIssuedResponse,
  settlePendingTasks,
  startCapture,
  trackPendingTask,
  terminationForAdvance,
  waitForCapturedPage,
} from './capture-logic.mjs';

const ENTRY_URL =
  'https://kind.krx.co.kr/disclosure/disclosurebystocktype.do?method=searchDisclosureByStockTypeEtf';
// Mirrors KIND_DISCLOSURE_MAX_PAGES in crates/market-data/src/providers/kind.rs.
// The Rust side enforces it too; this is a courtesy bound, not the guarantee.
const DEFAULT_MAX_PAGES = 40;
const DATE = /^\d{4}-\d{2}-\d{2}$/;

function usage(msg) {
  console.error(`${msg}\n\nusage: node capture.mjs --from YYYY-MM-DD --to YYYY-MM-DD --out DIR [--max-pages N]`);
  process.exit(2);
}

function parseArgs(argv) {
  const a = {};
  for (let i = 0; i < argv.length; i += 2) {
    const k = argv[i];
    const v = argv[i + 1];
    if (!k?.startsWith('--') || v === undefined) usage(`malformed argument near "${k ?? ''}"`);
    a[k.slice(2)] = v;
  }
  if (!DATE.test(a.from ?? '')) usage('--from must be YYYY-MM-DD');
  if (!DATE.test(a.to ?? '')) usage('--to must be YYYY-MM-DD');
  if (!a.out) usage('--out is required');
  const maxPages = a['max-pages'] ? Number(a['max-pages']) : DEFAULT_MAX_PAGES;
  if (!Number.isInteger(maxPages) || maxPages < 1 || maxPages > DEFAULT_MAX_PAGES) {
    usage(`--max-pages must be an integer in 1..=${DEFAULT_MAX_PAGES}`);
  }
  return { from: a.from, to: a.to, out: a.out, maxPages };
}

const { from, to, out, maxPages } = parseArgs(process.argv.slice(2));

// Never write into a populated directory: a staging dir maps 1:1 onto one Raw
// batch, and silently mixing two captures would corrupt that correspondence.
if (existsSync(out) && readdirSync(out).length > 0) {
  console.error(`refusing to write into a non-empty staging directory: ${out}`);
  process.exit(2);
}
mkdirSync(out, { recursive: true });

const browser = await chromium.launch();
const ctx = await browser.newContext({ locale: 'ko-KR', timezoneId: 'Asia/Seoul' });
const page = await ctx.newPage();

const captureState = createCaptureState({ fromDate: from, toDate: to });
const { captured } = captureState;
const pendingResponseTasks = createPendingTaskTracker();

page.on('response', (res) => {
  if (!captureState.active) return;
  if (res.request().method() !== 'POST') return;
  const url = res.url();
  const postData = res.request().postData() || '';
  // Identify the issued page at response-event time, before reading the body.
  // This prevents an unissued page from becoming admissible merely because a
  // later page was issued while its body read was pending.
  const issuedRequest = matchIssuedCaptureRequest(captureState, { url, postData });
  if (!issuedRequest) return;
  trackPendingTask(
    pendingResponseTasks,
    (async () => {
      let body;
      try {
        body = await res.body();
      } catch {
        return; // body no longer retrievable; the page-count check below will notice
      }
      // The contract remains issued while pagination advances, so this late
      // body is compared with the already stored response for its own page.
      recordIssuedResponse(captureState, issuedRequest, {
        body,
        retrievedAt: new Date().toISOString().replace(/\.\d{3}Z$/, 'Z'),
      });
    })(),
  );
});

let exitCode = 0;
try {
  await page.goto(ENTRY_URL, { waitUntil: 'domcontentloaded', timeout: 60000 });
  // The delay gives the entry page time to render its controls. It is not a
  // correctness barrier: late default responses are rejected by the exact
  // request predicate below.
  await page.waitForTimeout(2500);

  for (const [sel, val] of [['#fromDate', from], ['#toDate', to]]) {
    const el = await page.$(sel);
    if (!el) throw new Error(`date field ${sel} not present — the page layout changed`);
    await el.fill('');
    await el.type(val, { delay: 12 });
  }

  // Arm the response listener for the exact page-1 request. The entry page's
  // own default search may still be in flight, so time ordering is not used as
  // the correctness proof.
  if (!startCapture(captureState) || !issueCapturePage(captureState, 1)) {
    throw new Error('could not issue initial capture page');
  }
  await page.evaluate(() => {
    if (typeof fnSearch !== 'function') throw new Error('fnSearch missing');
    fnSearch();
  });
  const initialReceived = await waitForCapturedPage(
    captured,
    1,
    () => page.waitForTimeout(6000),
    () =>
      page.evaluate(() => {
        if (typeof fnSearch !== 'function') throw new Error('fnSearch missing during retry');
        fnSearch();
      }),
    () => captureState.failure !== null,
    () => settlePendingTasks(pendingResponseTasks),
  );
  const initialTasksSettled = await settlePendingTasks(pendingResponseTasks);
  let termination = initialReceived && initialTasksSettled && !captureState.failure ? null : TERMINATION.NO_RESPONSE;

  // Advance with the page's own paging function. The terminal condition is
  // observed, not guessed, and it is a pure byte comparison rather than any
  // reading of the body: past the last page KIND clamps `pageIndex` and returns
  // the final page again, so a response identical to its predecessor means the
  // end was already reached. That duplicate is discarded — keeping it would also
  // trip the ingest side's own duplicate-bytes rejection.
  // `maxPages` counts stored pages. Request one additional page to observe
  // whether the last stored page is clamped, without admitting a page beyond
  // the configured bound into staging.
  for (let next = 2; termination === null && next <= maxPages + 1; next += 1) {
    if (!issueCapturePage(captureState, next)) {
      termination = TERMINATION.NO_RESPONSE;
      break;
    }
    const moved = await page.evaluate((n) => {
      if (typeof fnPageGo === 'function') {
        fnPageGo(n);
        return 'fnPageGo';
      }
      const a = [...document.querySelectorAll('a')].find((x) => (x.innerText || '').trim() === String(n));
      if (a) {
        a.click();
        return 'anchor';
      }
      return null;
    }, next);
    termination = terminationForAdvance(moved);
    if (termination !== null) {
      break;
    }
    let retryMoved = true;
    const received = await waitForCapturedPage(
      captured,
      next,
      () => page.waitForTimeout(4500),
      async () => {
        retryMoved = await page.evaluate((n) => {
          if (typeof fnPageGo === 'function') {
            fnPageGo(n);
            return 'fnPageGo';
          }
          const a = [...document.querySelectorAll('a')].find((x) => (x.innerText || '').trim() === String(n));
          if (a) {
            a.click();
            return 'anchor';
          }
          return null;
        }, next);
      },
      () => captureState.failure !== null,
      () => settlePendingTasks(pendingResponseTasks),
    );
    const pageTasksSettled = await settlePendingTasks(pendingResponseTasks);
    if (!pageTasksSettled) {
      termination = TERMINATION.NO_RESPONSE;
      break;
    }
    if (!retryMoved) {
      termination = TERMINATION.ADVANCE_CONTROL_MISSING;
      break;
    }
    if (!received || captureState.failure) {
      termination = TERMINATION.NO_RESPONSE;
      break;
    }
    termination = classifyReceivedPage({ captured, pageIndex: next, maxPages });
  }

  const finalTasksSettled = await settlePendingTasks(pendingResponseTasks);
  if (!finalTasksSettled || captureState.failure) {
    termination = TERMINATION.NO_RESPONSE;
  }
  closeCaptureState(captureState);

  // The loop always resolves one of the explicit outcomes. Keep this guard
  // fail-closed if a future edit changes that invariant.
  if (termination === null) throw new Error('page walk ended without a termination state');

  const indices = [...captured.keys()].sort((x, y) => x - y);
  // The Rust side rejects gaps; fail here too rather than stage a bad capture.
  for (let i = 0; i < indices.length; i += 1) {
    if (indices[i] !== i + 1) throw new Error(`captured page indices are not contiguous from 1: ${indices.join(',')}`);
  }

  const pages = [];
  for (const idx of indices) {
    const rec = captured.get(idx);
    const file = `page-${String(idx).padStart(4, '0')}.html`;
    writeFileSync(join(out, file), rec.body);
    pages.push({
      page_index: idx,
      file,
      retrieved_at: rec.retrievedAt,
      form_fields: rec.formFields,
    });
  }

  writeFileSync(
    join(out, 'capture.json'),
    `${JSON.stringify(
      {
        source: 'kind.krx.co.kr',
        entry_url: ENTRY_URL,
        surface: 'etf-disclosure-list',
        requested_range: { from, to },
        termination,
        pages,
      },
      null,
      2,
    )}\n`,
  );

  if (!isCleanTermination(termination)) {
    console.error(
      `capture incomplete (${termination}): staged ${pages.length} page(s) for ${from}..${to}; do not ingest`,
    );
    exitCode = 1;
  } else {
    console.log(`captured ${pages.length} page(s) for ${from}..${to}`);
  }
  for (const p of pages) console.log(`  ${p.file}  pageIndex=${p.page_index}  retrieved_at=${p.retrieved_at}`);
  console.log(
    isCleanTermination(termination)
      ? `staged in ${out}; ingest with the kind-raw CLI, which recomputes every hash`
      : `incomplete staging retained in ${out} for diagnosis; kind-raw will reject it`,
  );
} catch (error) {
  console.error(`capture failed: ${error && error.message ? error.message : error}`);
  exitCode = 1;
} finally {
  await browser.close();
}
process.exit(exitCode);
