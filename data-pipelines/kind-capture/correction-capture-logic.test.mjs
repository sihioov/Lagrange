import test from 'node:test';
import assert from 'node:assert/strict';
import {
  closeSync,
  constants as fsConstants,
  fstatSync,
  mkdtempSync,
  openSync,
  readFileSync,
  rmSync,
  writeFileSync,
} from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import {
  CORRECTION_CONFIRM,
  CORRECTION_TERMINATION,
  MAX_CORRECTION_RESPONSE_BODY_BYTES,
  classifyCorrectionResponseBody,
  classifyLiveCorrectionAnchors,
  correctionPublishPlan,
  decodeCorrectionResponseBody,
  extractCorrectionAcceptance,
  extractCorrectionAnchorsFromHtml,
  extractCorrectionViewerAcceptance,
  findUniqueCorrectionAnchor,
  findUniqueCorrectionTargetInResponseBody,
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

const ACCEPTANCE = '20200207000058';

test('extracts only the exact correction viewer onclick contract', () => {
  assert.equal(extractCorrectionAcceptance(`openDisclsViewer('${ACCEPTANCE}','')`), ACCEPTANCE);
  assert.equal(extractCorrectionAcceptance(`openDisclsViewer('${ACCEPTANCE}','x')`), null);
  assert.equal(extractCorrectionAcceptance(`openDisclsViewer("${ACCEPTANCE}",'')`), null);
  assert.equal(extractCorrectionAcceptance(`openDisclsViewer('${ACCEPTANCE}0','')`), null);
  assert.equal(extractCorrectionAcceptance(`return openDisclsViewer('${ACCEPTANCE}','')`), null);
  assert.equal(extractCorrectionViewerAcceptance(`openDisclsViewer('${ACCEPTANCE}','x')`), ACCEPTANCE);
  assert.equal(extractCorrectionViewerAcceptance(`otherViewer('${ACCEPTANCE}','')`), null);
});

test('accepts one exact handler including repeated equivalent rendered anchors', () => {
  const unique = findUniqueCorrectionAnchor(
    [
      { onclick: "openDisclsViewer('19990101000001','')" },
      { onclick: `openDisclsViewer('${ACCEPTANCE}','')` },
    ],
    ACCEPTANCE,
  );
  assert.equal(unique.kind, 'unique');
  assert.equal(unique.index, 1);
  assert.equal(unique.occurrences, 1);

  assert.deepEqual(
    findUniqueCorrectionAnchor([], ACCEPTANCE),
    { kind: 'missing', termination: CORRECTION_TERMINATION.MISSING_TARGET },
  );
  const repeated = findUniqueCorrectionAnchor(
    [
      { onclick: `openDisclsViewer('${ACCEPTANCE}','')` },
      { onclick: `openDisclsViewer('${ACCEPTANCE}','')` },
    ],
    ACCEPTANCE,
  );
  assert.equal(repeated.kind, 'unique');
  assert.equal(repeated.occurrences, 2);

  assert.deepEqual(
    findUniqueCorrectionAnchor(
      [
        { onclick: `openDisclsViewer('${ACCEPTANCE}','')` },
        { onclick: `openDisclsViewer('${ACCEPTANCE}','alternate')` },
      ],
      ACCEPTANCE,
    ),
    { kind: 'duplicate', termination: CORRECTION_TERMINATION.DUPLICATE_TARGET },
  );
  assert.deepEqual(
    findUniqueCorrectionAnchor(
      [{ onclick: `openDisclsViewer('${ACCEPTANCE}','alternate')` }],
      ACCEPTANCE,
    ),
    { kind: 'missing', termination: CORRECTION_TERMINATION.MISSING_TARGET },
  );
});

test('response body attribution rejects a target-looking late default DOM', () => {
  const explicitBody = Buffer.from(
    `<table><tr><td><a title="target" onclick="openDisclsViewer('${ACCEPTANCE}','')">target</a></td></tr></table>`,
    'utf8',
  );
  const defaultBody = Buffer.from(
    `<table><tr><td><a onclick="openDisclsViewer('19990101000001','')">default</a></td></tr></table>`,
    'utf8',
  );
  assert.equal(findUniqueCorrectionTargetInResponseBody(explicitBody, ACCEPTANCE).kind, 'unique');
  assert.deepEqual(
    findUniqueCorrectionTargetInResponseBody(defaultBody, ACCEPTANCE),
    { kind: 'missing', termination: CORRECTION_TERMINATION.MISSING_TARGET },
  );
  const ambiguousBody = Buffer.from(
    `<a onclick="openDisclsViewer('${ACCEPTANCE}','')">exact</a>` +
      `<a onclick="openDisclsViewer('${ACCEPTANCE}','alternate')">alternate</a>`,
    'utf8',
  );
  assert.deepEqual(findUniqueCorrectionTargetInResponseBody(ambiguousBody, ACCEPTANCE), {
    kind: 'duplicate',
    termination: CORRECTION_TERMINATION.DUPLICATE_TARGET,
  });
});

test('response HTML scanner follows the observed quoted-anchor contract', () => {
  const html =
    `<area onclick="openDisclsViewer('${ACCEPTANCE}','')">` +
    `<a class = 'row' onclick = "openDisclsViewer('${ACCEPTANCE}','')" title='x>y'>x</a>` +
    `<a onclick="openDisclsViewer('${ACCEPTANCE}','')">second</a>` +
    `<a data-onclick="openDisclsViewer('${ACCEPTANCE}','')">not onclick</a>`;
  assert.deepEqual(extractCorrectionAnchorsFromHtml(html), [
    { onclick: `openDisclsViewer('${ACCEPTANCE}','')` },
    { onclick: `openDisclsViewer('${ACCEPTANCE}','')` },
  ]);
  const target = findUniqueCorrectionTargetInResponseBody(Buffer.from(html), ACCEPTANCE);
  assert.equal(target.kind, 'unique');
  assert.equal(target.occurrences, 2);
});

test('body read failures, invalid UTF-8, and oversize responses are no_response', () => {
  assert.deepEqual(
    classifyCorrectionResponseBody({ status: 'pending', acceptance: ACCEPTANCE }),
    { kind: 'no_response', termination: CORRECTION_TERMINATION.NO_RESPONSE },
  );
  assert.deepEqual(
    classifyCorrectionResponseBody({
      status: 'rejected',
      body: Buffer.from('ignored'),
      acceptance: ACCEPTANCE,
    }),
    { kind: 'no_response', termination: CORRECTION_TERMINATION.NO_RESPONSE },
  );
  assert.deepEqual(
    findUniqueCorrectionTargetInResponseBody(Buffer.from([0xc3, 0x28]), ACCEPTANCE),
    { kind: 'no_response', termination: CORRECTION_TERMINATION.NO_RESPONSE },
  );
  assert.deepEqual(
    findUniqueCorrectionTargetInResponseBody(
      Buffer.alloc(MAX_CORRECTION_RESPONSE_BODY_BYTES + 1, 0x20),
      ACCEPTANCE,
    ),
    { kind: 'no_response', termination: CORRECTION_TERMINATION.NO_RESPONSE },
  );
  assert.equal(decodeCorrectionResponseBody(Buffer.from('ok')), 'ok');
});

test('live target matching is independent of response snapshot index', () => {
  const onclick = `openDisclsViewer('${ACCEPTANCE}','')`;
  const currentDom = [
    { onclick: "openDisclsViewer('19990101000001','')" },
    { onclick },
  ];
  assert.deepEqual(classifyLiveCorrectionAnchors(currentDom, onclick), { kind: 'unique' });
  assert.deepEqual(classifyLiveCorrectionAnchors([], onclick), {
    kind: 'missing',
    termination: CORRECTION_TERMINATION.MISSING_TARGET,
  });
  assert.deepEqual(classifyLiveCorrectionAnchors([{ onclick }, { onclick }], onclick), {
    kind: 'unique',
  });
  assert.deepEqual(
    classifyLiveCorrectionAnchors(
      [{ onclick }, { onclick: `openDisclsViewer('${ACCEPTANCE}','alternate')` }],
      onclick,
    ),
    { kind: 'duplicate', termination: CORRECTION_TERMINATION.DUPLICATE_TARGET },
  );
});

test('output safety requires a new path under a real non-symlink directory', () => {
  assert.deepEqual(
    validateCorrectionOutputContract({
      outputExists: true,
      outputIsSymlink: false,
      parentIsDirectory: true,
      resolvedParentIsDirectory: true,
      resolvedParentMatches: true,
    }),
    { ok: false, error: 'output_must_not_exist' },
  );
  assert.deepEqual(
    validateCorrectionOutputContract({
      outputExists: true,
      outputIsSymlink: true,
      parentIsDirectory: true,
      resolvedParentIsDirectory: true,
      resolvedParentMatches: true,
    }),
    { ok: false, error: 'output_must_not_exist' },
  );
  assert.deepEqual(
    validateCorrectionOutputContract({
      outputExists: false,
      parentIsDirectory: false,
      parentIsSymlink: false,
      resolvedParentIsDirectory: false,
      resolvedParentMatches: true,
    }),
    { ok: false, error: 'parent_must_be_real_directory' },
  );
  assert.deepEqual(
    validateCorrectionOutputContract({
      outputExists: false,
      parentIsDirectory: true,
      parentIsSymlink: true,
      resolvedParentIsDirectory: true,
      resolvedParentIsSymlink: true,
      resolvedParentMatches: false,
    }),
    { ok: false, error: 'parent_must_be_real_directory' },
  );
  assert.deepEqual(
    validateCorrectionOutputContract({
      outputExists: false,
      parentIsDirectory: true,
      parentIsSymlink: false,
      resolvedParentIsDirectory: true,
      resolvedParentIsSymlink: false,
      resolvedParentMatches: true,
    }),
    { ok: true },
  );
});

test('capture.json is the final commit marker for complete or incomplete output', () => {
  assert.deepEqual(correctionPublishPlan(true), ['viewer.html', 'capture.json']);
  assert.deepEqual(correctionPublishPlan(false), ['capture.json']);
  assert.equal(shouldCleanupCorrectionOutputDirectory({ published: false }), true);
  assert.equal(shouldCleanupCorrectionOutputDirectory({ published: true }), false);
});

test('metadata failure cleanup remains required until capture.json is committed', () => {
  assert.equal(shouldCleanupCorrectionOutputDirectory(), true);
  assert.equal(shouldCleanupCorrectionOutputDirectory({ published: false }), true);
  assert.equal(shouldCleanupCorrectionOutputDirectory({ published: true }), false);
});

test('output reservation is atomic no-replace and preserves an existing directory', () => {
  const parentPath = mkdtempSync(join(tmpdir(), 'kind-correction-output-'));
  const parentFd = openSync(
    parentPath,
    fsConstants.O_RDONLY | fsConstants.O_DIRECTORY | fsConstants.O_NOFOLLOW,
  );
  const parentStat = fstatSync(parentFd);
  const base = {
    outputName: 'capture',
    parentPath,
    parentFd,
    anchoredParentPath: `/proc/self/fd/${parentFd}`,
    parentDevice: parentStat.dev,
    parentInode: parentStat.ino,
  };
  let reserved;
  try {
    reserved = reserveCorrectionOutputDirectory(base);
    assertCurrentCorrectionOutputDirectory(reserved);
    writeFileSync(join(reserved.anchoredOutputPath, 'sentinel'), 'owned', { flag: 'wx' });
    assert.throws(() => reserveCorrectionOutputDirectory(base), { code: 'EEXIST' });
    assert.equal(readFileSync(join(reserved.anchoredOutputPath, 'sentinel'), 'utf8'), 'owned');
    rmSync(join(reserved.anchoredOutputPath, 'sentinel'));
    assert.equal(cleanupCorrectionOutputDirectory(reserved), true);
  } finally {
    if (reserved) closeSync(reserved.outputDirectoryFd);
    closeSync(parentFd);
    rmSync(parentPath, { recursive: true, force: true });
  }
});

test('explicit response admission reuses the exact list POST contract', () => {
  const url = 'https://kind.krx.co.kr/disclosure/disclosurebystocktype.do';
  const expected = { fromDate: '2020-02-03', toDate: '2020-02-07', pageIndex: 1 };
  const explicit =
    'method=searchDisclosureByStockTypeEtfSub&forward=disclosurebystocktype_etf_sub' +
    '&fromDate=2020-02-03&toDate=2020-02-07&pageIndex=1';
  const matched = matchExplicitCorrectionResponse(
    { method: 'POST', url, postData: explicit },
    expected,
  );
  assert.deepEqual(matched?.formFields, [
    ['method', 'searchDisclosureByStockTypeEtfSub'],
    ['forward', 'disclosurebystocktype_etf_sub'],
    ['fromDate', '2020-02-03'],
    ['toDate', '2020-02-07'],
    ['pageIndex', '1'],
  ]);
  assert.equal(
    matchExplicitCorrectionResponse(
      {
        method: 'POST',
        url,
        postData: explicit.replace('2020-02-03', '2019-02-03'),
      },
      expected,
    ),
    null,
  );
  assert.equal(
    matchExplicitCorrectionResponse(
      {
        method: 'GET',
        url,
        postData: explicit,
      },
      expected,
    ),
    null,
  );
});

test('response tracking requires the exact target acceptance in its expectation', () => {
  const expected = {
    fromDate: '2020-02-07',
    toDate: '2020-02-07',
    pageIndex: 1,
    acceptance: ACCEPTANCE,
  };
  assert.equal(validateCorrectionResponseExpectation(expected), true);
  assert.equal(validateCorrectionResponseExpectation({ ...expected, acceptance: undefined }), false);
  assert.equal(validateCorrectionResponseExpectation({ ...expected, pageIndex: 2 }), false);
});

test('viewer URL contract is exact while query semantics remain opaque', () => {
  assert.equal(
    isExpectedCorrectionViewerUrl(
      'https://kind.krx.co.kr/common/disclsviewer.do?opaque=browser-generated',
    ),
    true,
  );
  assert.equal(
    isExpectedCorrectionViewerUrl(
      'https://kind.krx.co.kr/common/disclsviewer.do?next=%2Fcommon%2Fdisclsviewer.do',
    ),
    true,
  );

  const rejected = [
    'http://kind.krx.co.kr/common/disclsviewer.do',
    'https://evil.kind.krx.co.kr/common/disclsviewer.do',
    'https://kind.krx.co.kr.evil.example/common/disclsviewer.do',
    'https://kind.krx.co.kr:443/common/disclsviewer.do',
    'https://user:pass@kind.krx.co.kr/common/disclsviewer.do',
    'https://kind.krx.co.kr/common/disclsviewer.do/extra',
    'https://kind.krx.co.kr/common/disclsviewer.do?x=1#fragment',
    'https://kind.krx.co.kr/common/other.do',
    'not a URL',
  ];
  for (const url of rejected) {
    assert.equal(isExpectedCorrectionViewerUrl(url), false, url);
  }
});

test('viewer is complete only with exactly one select identified as mainDoc', () => {
  assert.equal(
    mainDocTermination([{ tagName: 'SELECT', id: 'mainDoc', name: '', optionCount: 1 }]),
    CORRECTION_TERMINATION.VIEWER_LOADED,
  );
  assert.equal(
    mainDocTermination([{ tagName: 'select', id: '', name: 'mainDoc', optionCount: 2 }]),
    CORRECTION_TERMINATION.VIEWER_LOADED,
  );
  assert.equal(
    mainDocTermination([{ tagName: 'DIV', id: 'mainDoc', name: '', optionCount: 1 }]),
    CORRECTION_TERMINATION.MAIN_DOC_MISSING,
  );
  assert.equal(
    mainDocTermination([
      { tagName: 'SELECT', id: 'mainDoc', name: '', optionCount: 1 },
      { tagName: 'SELECT', id: '', name: 'mainDoc', optionCount: 1 },
    ]),
    CORRECTION_TERMINATION.MAIN_DOC_DUPLICATE,
  );
  assert.equal(
    mainDocTermination([{ tagName: 'SELECT', id: 'mainDoc', name: '', optionCount: 0 }]),
    CORRECTION_TERMINATION.MAIN_DOC_EMPTY,
  );
  assert.equal(
    isCleanCorrectionTermination(CORRECTION_TERMINATION.VIEWER_LOADED),
    true,
  );
  assert.equal(isCleanCorrectionTermination(CORRECTION_TERMINATION.NO_POPUP), false);
});

test('CLI validation enforces the operator gate and exact identifier shapes', () => {
  const valid = validateCorrectionCliArgs({
    from: '2020-02-03',
    to: '2020-02-07',
    acceptance: ACCEPTANCE,
    out: '/tmp/correction-capture',
    confirm: CORRECTION_CONFIRM,
  });
  assert.equal(valid.ok, true);

  for (const [field, value, error] of [
    ['from', '20200203', 'invalid_from'],
    ['from', '2020-02-30', 'invalid_from'],
    ['to', '2020-2-07', 'invalid_to'],
    ['to', '2020-02-31', 'invalid_to'],
    ['acceptance', '2020020700005a', 'invalid_acceptance'],
    ['out', '', 'invalid_out'],
    ['confirm', 'yes', 'invalid_confirmation'],
  ]) {
    const options = {
      from: '2020-02-03',
      to: '2020-02-07',
      acceptance: ACCEPTANCE,
      out: '/tmp/correction-capture',
      confirm: CORRECTION_CONFIRM,
    };
    options[field] = value;
    assert.deepEqual(validateCorrectionCliArgs(options), { ok: false, error });
  }
  assert.deepEqual(
    validateCorrectionCliArgs({
      from: '2020-02-08',
      to: '2020-02-07',
      acceptance: ACCEPTANCE,
      out: '/tmp/correction-capture',
      confirm: CORRECTION_CONFIRM,
    }),
    { ok: false, error: 'invalid_date_range' },
  );
});
