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

const ENTRY_URL =
  'https://kind.krx.co.kr/disclosure/disclosurebystocktype.do?method=searchDisclosureByStockTypeEtf';
const RESPONSE_MATCH = /disclosurebystocktype\.do/;
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

// Ordered form fields exactly as the page sent them. Kept as pairs rather than an
// object so repeated names survive and the recorded order matches the request.
function parseFormFields(postData) {
  return (postData || '')
    .split('&')
    .filter(Boolean)
    .map((pair) => {
      const i = pair.indexOf('=');
      const raw = i === -1 ? [pair, ''] : [pair.slice(0, i), pair.slice(i + 1)];
      const dec = (s) => {
        try {
          return decodeURIComponent(s.replace(/\+/g, ' '));
        } catch {
          return s;
        }
      };
      return [dec(raw[0]), dec(raw[1])];
    });
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

/** @type {Map<number, {body: Buffer, retrievedAt: string, formFields: [string,string][]}>} */
const captured = new Map();
let capturing = false;

page.on('response', async (res) => {
  if (!capturing) return;
  if (res.request().method() !== 'POST') return;
  if (!RESPONSE_MATCH.test(res.url())) return;
  let body;
  try {
    body = await res.body();
  } catch {
    return; // body no longer retrievable; the page-count check below will notice
  }
  const postData = res.request().postData() || '';
  const m = postData.match(/(?:^|&)pageIndex=(\d+)/);
  if (!m) return;
  const pageIndex = Number(m[1]);
  if (captured.has(pageIndex)) return; // first response for a page wins
  captured.set(pageIndex, {
    body,
    retrievedAt: new Date().toISOString().replace(/\.\d{3}Z$/, 'Z'),
    formFields: parseFormFields(postData),
  });
});

let exitCode = 0;
try {
  await page.goto(ENTRY_URL, { waitUntil: 'domcontentloaded', timeout: 60000 });
  await page.waitForTimeout(2500);

  for (const [sel, val] of [['#fromDate', from], ['#toDate', to]]) {
    const el = await page.$(sel);
    if (!el) throw new Error(`date field ${sel} not present — the page layout changed`);
    await el.fill('');
    await el.type(val, { delay: 12 });
  }

  // Only the explicit search is captured; the entry page fires its own default
  // search on load, which is not the query being recorded.
  capturing = true;
  await page.evaluate(() => {
    if (typeof fnSearch !== 'function') throw new Error('fnSearch missing');
    fnSearch();
  });
  await page.waitForTimeout(6000);
  if (!captured.has(1)) throw new Error('no response captured for page 1');

  // Advance with the page's own paging function. The terminal condition is
  // observed, not guessed, and it is a pure byte comparison rather than any
  // reading of the body: past the last page KIND clamps `pageIndex` and returns
  // the final page again, so a response identical to its predecessor means the
  // end was already reached. That duplicate is discarded — keeping it would also
  // trip the ingest side's own duplicate-bytes rejection.
  for (let next = 2; next <= maxPages; next += 1) {
    const before = captured.size;
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
    if (!moved) break;
    await page.waitForTimeout(4500);
    if (captured.size === before || !captured.has(next)) break;
    if (captured.get(next).body.equals(captured.get(next - 1).body)) {
      captured.delete(next);
      break;
    }
  }

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
        pages,
      },
      null,
      2,
    )}\n`,
  );

  console.log(`captured ${pages.length} page(s) for ${from}..${to}`);
  for (const p of pages) console.log(`  ${p.file}  pageIndex=${p.page_index}  retrieved_at=${p.retrieved_at}`);
  console.log(`staged in ${out}; ingest with the kind-raw CLI, which recomputes every hash`);
} catch (error) {
  console.error(`capture failed: ${error && error.message ? error.message : error}`);
  exitCode = 1;
} finally {
  await browser.close();
}
process.exit(exitCode);
