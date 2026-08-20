import assert from 'node:assert/strict';
import test from 'node:test';

import {
  CAPTURE_CONFIRM,
  CAPTURE_RESPONSE,
  TERMINATION,
  ISSUED_PAGE_STATUS,
  captureExpectedResponse,
  captureIssuedResponse,
  classifyReceivedPage,
  closeCaptureState,
  createCaptureState,
  createPendingTaskTracker,
  issueCapturePage,
  isCalendarDate,
  isExpectedCaptureRequest,
  isExpectedResponseUrl,
  isCleanTermination,
  parseExpectedCaptureRequest,
  parseFormFields,
  settlePendingTasks,
  startCapture,
  trackPendingTask,
  terminationForAdvance,
  waitForCapturedPage,
  validateCaptureCliArgs,
} from './capture-logic.mjs';

function page(body) {
  return { body: Buffer.from(body) };
}

const expected = Object.freeze({
  fromDate: '2020-02-03',
  toDate: '2020-02-07',
  pageIndex: 1,
});
const RESPONSE_URL = 'https://kind.krx.co.kr/disclosure/disclosurebystocktype.do';

test('CLI validation requires the exact operator confirmation before browser launch', () => {
  const valid = {
    from: expected.fromDate,
    to: expected.toDate,
    out: '/tmp/kind-staging',
    confirm: CAPTURE_CONFIRM,
  };
  assert.deepEqual(validateCaptureCliArgs(valid), {
    ok: true,
    value: { ...valid, maxPages: 40 },
  });

  for (const [key, value, error] of [
    ['confirm', undefined, 'invalid_confirmation'],
    ['confirm', 'yes', 'invalid_confirmation'],
    ['from', '2026-02-30', 'invalid_from'],
    ['to', '2026-13-01', 'invalid_to'],
    ['from', '2020-02-08', 'invalid_date_range'],
    ['max-pages', '41', 'invalid_max_pages'],
  ]) {
    assert.deepEqual(validateCaptureCliArgs({ ...valid, [key]: value }), { ok: false, error });
  }
});

test('calendar validation accepts real leap days and rejects impossible dates', () => {
  assert.equal(isCalendarDate('2024-02-29'), true);
  for (const value of ['2023-02-29', '2026-02-30', '2026-04-31', '2026-13-01']) {
    assert.equal(isCalendarDate(value), false, value);
  }
});

function formFields({ fromDate = expected.fromDate, toDate = expected.toDate, pageIndex = expected.pageIndex, extra = '' } = {}) {
  return [
    'method=searchDisclosureByStockTypeEtfSub',
    'forward=disclosurebystocktype_etf_sub',
    `fromDate=${fromDate}`,
    `toDate=${toDate}`,
    `pageIndex=${pageIndex}`,
    extra,
  ]
    .filter(Boolean)
    .join('&');
}

function response(postData, body) {
  return {
    url: RESPONSE_URL,
    postData,
    body: Buffer.from(body),
    retrievedAt: '2026-08-20T00:00:00Z',
  };
}

test('the response URL must match the exact approved WHATWG URL contract', () => {
  assert.equal(isExpectedResponseUrl(RESPONSE_URL), true);
  for (const url of [
    'http://kind.krx.co.kr/disclosure/disclosurebystocktype.do',
    'https://kind.krx.co.kr.evil/disclosure/disclosurebystocktype.do',
    'https://kind.krx.co.kr/disclosure/details.do',
    'https://kind.krx.co.kr/disclosure/disclosurebystocktype.do/extra',
    'https://kind.krx.co.kr:443/disclosure/disclosurebystocktype.do',
    'https://kind.krx.co.kr:8443/disclosure/disclosurebystocktype.do',
    'https://user@kind.krx.co.kr/disclosure/disclosurebystocktype.do',
    'https://user:secret@kind.krx.co.kr/disclosure/disclosurebystocktype.do',
    'https://kind.krx.co.kr/disclosure/disclosurebystocktype.do?method=x',
    'https://kind.krx.co.kr/disclosure/disclosurebystocktype.do#fragment',
    'not a URL',
  ]) {
    assert.equal(isExpectedResponseUrl(url), false, url);
  }
});

test('only the explicit search is captured when a default response arrives first', () => {
  const captured = new Map();
  const defaultResponse = captureExpectedResponse(
    captured,
    response(formFields({ fromDate: '2026-08-19', toDate: '2026-08-20' }), 'default page'),
    expected,
  );
  const explicitResponse = captureExpectedResponse(
    captured,
    response(formFields(), 'explicit page'),
    expected,
  );

  assert.equal(defaultResponse, CAPTURE_RESPONSE.IGNORED);
  assert.equal(explicitResponse, CAPTURE_RESPONSE.STORED);
  assert.equal(captured.size, 1);
  assert.equal(captured.get(1).body.toString(), 'explicit page');
  assert.deepEqual(captured.get(1).formFields, parseExpectedCaptureRequest(formFields(), expected));
});

test('issued page state admits its own late response after pagination advances', () => {
  const state = createCaptureState({ fromDate: expected.fromDate, toDate: expected.toDate });
  assert.equal(startCapture(state), true);
  assert.equal(issueCapturePage(state, 1), true);

  const unissued = captureIssuedResponse(state, response(formFields({ pageIndex: 2 }), 'not issued'));
  const first = captureIssuedResponse(state, response(formFields(), 'page one'));
  assert.equal(unissued, CAPTURE_RESPONSE.IGNORED);
  assert.equal(first, CAPTURE_RESPONSE.STORED);

  assert.equal(issueCapturePage(state, 2), true);
  const second = captureIssuedResponse(state, response(formFields({ pageIndex: 2 }), 'page two'));
  const lateDuplicate = captureIssuedResponse(state, response(formFields(), 'page one'));
  const lateConflict = captureIssuedResponse(state, response(formFields(), 'different page one'));

  assert.equal(second, CAPTURE_RESPONSE.STORED);
  assert.equal(lateDuplicate, CAPTURE_RESPONSE.DUPLICATE);
  assert.equal(lateConflict, CAPTURE_RESPONSE.CONFLICT);
  assert.equal(state.issuedPages.get(1).status, ISSUED_PAGE_STATUS.CONFLICT);
  assert.equal(state.captured.get(1).body.toString(), 'page one');
});

test('closed capture state ignores late responses and duplicate form contracts', () => {
  const state = createCaptureState({ fromDate: expected.fromDate, toDate: expected.toDate });
  startCapture(state);
  issueCapturePage(state, 1);

  const duplicateForm = `${formFields()}&pageIndex=1`;
  assert.equal(captureIssuedResponse(state, response(duplicateForm, 'duplicate form')), CAPTURE_RESPONSE.IGNORED);
  closeCaptureState(state);
  assert.equal(captureIssuedResponse(state, response(formFields(), 'after close')), CAPTURE_RESPONSE.IGNORED);
  assert.equal(state.captured.size, 0);
});

test('draining a pending body task records its late conflict before close', async () => {
  const state = createCaptureState({ fromDate: expected.fromDate, toDate: expected.toDate });
  startCapture(state);
  issueCapturePage(state, 1);
  assert.equal(captureIssuedResponse(state, response(formFields(), 'first page')), CAPTURE_RESPONSE.STORED);
  issueCapturePage(state, 2);

  const pendingTasks = createPendingTaskTracker();
  let resolveBody;
  const body = new Promise((resolve) => {
    resolveBody = resolve;
  });
  trackPendingTask(
    pendingTasks,
    body.then((bytes) =>
      captureIssuedResponse(state, response(formFields(), bytes)),
    ),
  );

  const firstSettled = await settlePendingTasks(pendingTasks);
  assert.equal(firstSettled, false);
  assert.equal(state.failure, null);

  resolveBody('different page one');
  const secondSettled = await settlePendingTasks(pendingTasks);
  assert.equal(secondSettled, true);
  assert.equal(state.failure, CAPTURE_RESPONSE.CONFLICT);
  assert.equal(pendingTasks.size, 0);
  closeCaptureState(state);
});

test('bounded page wait drains a matched body task before checking captured state', async () => {
  const captured = new Map();
  const pendingTasks = createPendingTaskTracker();
  let resolveBody;
  const body = new Promise((resolve) => {
    resolveBody = resolve;
  });
  trackPendingTask(
    pendingTasks,
    body.then(() => {
      captured.set(1, page('page one'));
    }),
  );
  let retries = 0;
  const received = await waitForCapturedPage(
    captured,
    1,
    async () => resolveBody(),
    async () => {
      retries += 1;
    },
    () => false,
    () => settlePendingTasks(pendingTasks),
  );

  assert.equal(received, true);
  assert.equal(retries, 0);
});

test('resolved and rejected tracked tasks settle through microtasks', async () => {
  const pendingTasks = createPendingTaskTracker();
  trackPendingTask(pendingTasks, Promise.resolve('done'));
  trackPendingTask(pendingTasks, Promise.reject(new Error('expected test rejection')));

  assert.equal(await settlePendingTasks(pendingTasks), true);
  assert.equal(pendingTasks.size, 0);
});

test('a never-resolving body task marks the page incomplete without hanging', async () => {
  const pendingTasks = createPendingTaskTracker();
  trackPendingTask(pendingTasks, new Promise(() => {}));

  assert.equal(await settlePendingTasks(pendingTasks), false);
  let retries = 0;
  const received = await waitForCapturedPage(
    new Map(),
    1,
    async () => {},
    async () => {
      retries += 1;
    },
    () => false,
    () => settlePendingTasks(pendingTasks),
  );

  assert.equal(received, false);
  assert.equal(retries, 0);
});

test('the expected request requires exact fromDate and toDate', () => {
  const fields = parseFormFields(formFields({ fromDate: '2020-02-04' }));
  assert.equal(isExpectedCaptureRequest(fields, expected), false);

  const otherEnd = parseFormFields(formFields({ toDate: '2020-02-08' }));
  assert.equal(isExpectedCaptureRequest(otherEnd, expected), false);
});

test('duplicate or missing required fields do not match', () => {
  const duplicate = parseFormFields(`${formFields()}&fromDate=${expected.fromDate}`);
  assert.equal(isExpectedCaptureRequest(duplicate, expected), false);

  const conflicting = parseFormFields(`${formFields()}&fromDate=2020-02-04`);
  assert.equal(isExpectedCaptureRequest(conflicting, expected), false);

  const missing = parseFormFields(
    formFields().replace(`toDate=${expected.toDate}&`, ''),
  );
  assert.equal(isExpectedCaptureRequest(missing, expected), false);

  const missingPage = parseFormFields(formFields().replace(`pageIndex=${expected.pageIndex}`, ''));
  assert.equal(isExpectedCaptureRequest(missingPage, expected), false);
});

test('malformed form encoding and pageIndex do not match', () => {
  assert.equal(parseExpectedCaptureRequest(`${formFields()}%E0%A4%A`, expected), null);
  for (const pageIndex of ['0', '-1', '1.0', 'not-a-page', '9007199254740992']) {
    const fields = parseFormFields(formFields({ pageIndex }));
    assert.equal(isExpectedCaptureRequest(fields, expected), false, `pageIndex ${pageIndex}`);
  }
  assert.equal(isExpectedCaptureRequest(parseFormFields(`${formFields()}&method=other`), expected), false);
});

test('ordered decoded fields preserve duplicate names and order', () => {
  const fields = parseFormFields('method=a&tag=one+two&tag=two%2Bthree&empty=&method=b');
  assert.deepEqual(fields, [
    ['method', 'a'],
    ['tag', 'one two'],
    ['tag', 'two+three'],
    ['empty', ''],
    ['method', 'b'],
  ]);
});

test('same request metadata and body is a harmless duplicate, but a conflict fails closed', () => {
  const captured = new Map();
  const first = captureExpectedResponse(captured, response(formFields(), 'first'), expected);
  const duplicate = captureExpectedResponse(captured, response(formFields(), 'first'), expected);
  const conflict = captureExpectedResponse(captured, response(formFields(), 'different'), expected);

  assert.equal(first, CAPTURE_RESPONSE.STORED);
  assert.equal(duplicate, CAPTURE_RESPONSE.DUPLICATE);
  assert.equal(conflict, CAPTURE_RESPONSE.CONFLICT);
  assert.equal(captured.get(1).body.toString(), 'first');
});

test('a matched duplicate with different request metadata fails closed', () => {
  const captured = new Map();
  const first = captureExpectedResponse(captured, response(formFields({ extra: 'optional=one' }), 'same'), expected);
  const conflict = captureExpectedResponse(captured, response(formFields({ extra: 'optional=two' }), 'same'), expected);

  assert.equal(first, CAPTURE_RESPONSE.STORED);
  assert.equal(conflict, CAPTURE_RESPONSE.CONFLICT);
});

test('a capture conflict stops the bounded retry path', async () => {
  const captured = new Map();
  let retries = 0;
  const received = await waitForCapturedPage(
    captured,
    1,
    async () => {},
    async () => {
      retries += 1;
    },
    () => true,
  );

  assert.equal(received, false);
  assert.equal(retries, 0);
});

test('a delayed response gets one retry before no_response', async () => {
  const captured = new Map();
  const attempts = [];
  let retries = 0;
  const received = await waitForCapturedPage(
    captured,
    2,
    async (attempt) => attempts.push(attempt),
    async () => {
      retries += 1;
      captured.set(2, page('second page'));
    },
  );

  assert.equal(received, true);
  assert.deepEqual(attempts, [0, 1]);
  assert.equal(retries, 1);
});

test('two bounded waits without a response classify as no_response', async () => {
  const captured = new Map();
  let attempts = 0;
  let retries = 0;
  const received = await waitForCapturedPage(
    captured,
    2,
    async () => {
      attempts += 1;
    },
    async () => {
      retries += 1;
    },
  );

  assert.equal(received, false);
  assert.equal(attempts, 2);
  assert.equal(retries, 1);
  assert.equal(TERMINATION.NO_RESPONSE, 'no_response');
});

test('a missing paging control is an incomplete terminal', () => {
  const termination = terminationForAdvance(null);

  assert.equal(termination, TERMINATION.ADVANCE_CONTROL_MISSING);
  assert.equal(isCleanTermination(termination), false);
});

test('duplicate page 41 proves exactly 40 stored pages complete', () => {
  const captured = new Map();
  for (let pageIndex = 1; pageIndex <= 40; pageIndex += 1) {
    captured.set(pageIndex, page(`page ${pageIndex}`));
  }
  captured.set(41, page('page 40'));

  const termination = classifyReceivedPage({ captured, pageIndex: 41, maxPages: 40 });

  assert.equal(termination, TERMINATION.CLAMPED_DUPLICATE);
  assert.equal(isCleanTermination(termination), true);
  assert.equal(captured.size, 40);
  assert.deepEqual([...captured.keys()], Array.from({ length: 40 }, (_, i) => i + 1));
});

test('a distinct page beyond the bound proves truncation and is not retained', () => {
  const captured = new Map([[40, page('page 40')], [41, page('page 41')]]);

  const termination = classifyReceivedPage({ captured, pageIndex: 41, maxPages: 40 });

  assert.equal(termination, TERMINATION.PAGE_BOUND_REACHED);
  assert.equal(isCleanTermination(termination), false);
  assert.deepEqual([...captured.keys()], [40]);
});
