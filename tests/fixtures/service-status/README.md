# Synthetic Owner service-status contract fixtures

These are WP-1 review artifacts, not captured production observations. Dates,
commits, project names, opaque join labels and counts are synthetic. No market
prices, provider payloads, secrets, sessions, accounts or real user IDs are present.

- `base-input.json`: fixed evaluation clock, raw safe host DTO, DB repository read
  results and operational-window evidence. The DB objects are explicit **safe query
  result fixtures**, not a dump and not already adjudicated work states. Separate
  calendar join columns model lineage disagreement. Strings beginning `synthetic-`
  are fixture join labels; actual isolated SQL tests must replace them with locally
  generated UUIDs consistently. They never enter the public host/API schemas.
- `base-output.json`: complete expected API response, ordered by catalog.
- `cases.json`: independent input and expected JSON Pointer patches. Apply patches
  to separate deep copies of base input/output in array order. Replace requires an
  existing key; add/remove use ordinary object/array JSON Pointer semantics. A
  patch list with no changes means exact base equality. Do not derive expected
  statuses from a product reducer. All timestamps in an oracle are exact.
- `collector-vectors.json`: selected fixed command safe-field observations and
  exact expected host observation, plus typed negative cases. These do not execute
  Docker/systemd. Installer/unit binding is a separate input, not parsed from env.
- `wire-cases.json`: parser/FS/HTTP, standalone reducer, domain-boundary and UI
  lifecycle vectors. An `input.fault` is a deterministic corruption recipe; the
  corresponding downstream test must construct the actual bytes/file/HTTP event.
  This file is not evidence that a filesystem, HTTP or browser test has run.
- `validate_contract.mjs`: bounded static validation with the already installed
  local Ajv 2020 + ajv-formats. No dependency download or product import. Checks
  schema/meta-schema, materialized fixture types, IDs, source citation existence,
  state coverage, overall-priority consistency and migration inventory integrity.
- `validate_temporal_contract.mjs`: independent, narrow checks of materialized
  observation ceilings, positive stream prerequisites and exact queue/window/EOD
  boundaries. It uses integer microseconds, imports no product reducer, and does
  not manufacture expected statuses or claim runtime/cache validation.

Default validation works throughout the approved implementation waves: contiguous,
matched 0001–0057 pairs, optionally followed by the complete
`0058_service_status_reads` pair, then the complete
`0059_research_execution_status` pair. Each pair requires both `.up.sql` and
`.down.sql` with exactly the same name. Wrong reserved names, duplicate revisions
or directions, gaps, unpaired/malformed SQL files, non-regular SQL entries and any
unapproved revision fail validation. SQL contents/permissions require downstream
DB tests; this check validates filenames only.

`--baseline-reservations` additionally requires the inventory to end at 0057,
proving both reservations are still free. Run it for the baseline acceptance check;
it intentionally fails after either approved pair is added. Use default mode in
all subsequent waves; do not rename or skip migrations to make validation pass.

From repository root:

```sh
node tests/fixtures/service-status/validate_contract.mjs
node tests/fixtures/service-status/validate_temporal_contract.mjs
node tests/fixtures/service-status/validate_contract.mjs --baseline-reservations
python3 - <<'PY'
import json
from pathlib import Path

def unique(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('duplicate JSON key: ' + key)
        result[key] = value
    return result

def reject_constant(value):
    raise ValueError('non-JSON numeric constant: ' + value)

paths = list(Path('configs/operations').glob('service-status*.json'))
paths += list(Path('tests/fixtures/service-status').glob('*.json'))
for path in paths:
    json.loads(path.read_text(), object_pairs_hook=unique, parse_constant=reject_constant)
print('Strict JSON check:', len(paths), 'documents passed')
PY
```

This validates contract artifacts only. WP-2 must run actual fake-host and temporary
filesystem tests; WP-3 must run the real isolated PostgreSQL/API harness; WP-4/5
must run the controller and browser cases. None was executed by WP-1. Microsecond
boundary vectors must use exact time arithmetic downstream; JavaScript Date's
millisecond truncation is not an acceptable implementation of those domain tests.
No production state or provider request is needed to exercise any vector.

Research execution fixtures describe the last persisted attempt only (contract
§G2). A completely unrecorded later attempt cannot change projection output for
identical observed inputs. No fixture invents a failed-write observation, timestamp
or later success. Existing stale-RUNNING vectors still apply; WP-3A must separately
inject start/heartbeat/finish write failures and prove pipeline-result and fence
preservation. Independent publication facts keep their own evidence and timestamps.

Host TTL is 90 seconds inclusive, DB evidence is 30 seconds inclusive, and browser
expiry is conservative at equality. Producer heartbeat accepts age 10s, receipt
and provider event accept age 30s, producer lease requires more than 5s remaining.
At EOD grace equality stale data is degraded. An expired job lease at equality
fails. Current calendar age 36h is accepted only with exact current-day lineage;
operational-window evidence must be from the same KST date. Refer to the normative
contract for the full predicate ordering and intentional source/UI distinctions.

Published validity is conservative: an inclusive predicate can remain true at its
exact threshold while valid_until is already that same instant. Never add one
microsecond to heartbeat, receipt/event, queue-pending, recent-failure, research
heartbeat or scheduled-run boundaries. Once a threshold has already caused a
reported failure/degradation, its old success deadline does not expire a fresh DB
observation of that diagnosis. Preserve original source clocks and cap the read
at original observed_at+30s and each still-relevant transition. EOD eligibility
and grace transitions cap only EOD-dependent rows, not unrelated rows or the
underlying raw DB observation ceiling.

G1 SQL live_count is inclusive validated coverage and halted_count is its subset.
These evaluation inputs already use exclusive live/halted counts: a validated
adapter checks 0 <= SQL halted <= SQL live <= demand <=30 and converts live to
SQL live-halted exactly once. CLOSED/preopen/close-equality and invalid window or
producer conditions cannot inherit current positive coverage. Preserve demanded
identity counts and diagnostic source times. Pending quote readiness and any
positive live/halted metrics retain the accepted prerequisite deadlines, even
when the row is degraded or waiting. In particular all_demand_halted retains the
base heartbeat assurance through 01:00:09, not the generic 01:00:30 read ceiling.
The aggregate valid_until carries filtered covered-identity assurance minima;
unfiltered oldest receipt/event timestamps can still diagnose a stale identity.

The fixture-only schema requires numeric counts even for owner_binding_valid=false.
Those required placeholders and remaining template fields are ignored in that
branch: they are not returned SQL state, authenticated zero demand, or positive
proof. The public binding-failure oracle exposes null stream counts/times, while
the successful query observation can retain its 30s negative-observation ceiling.
Missing/invalid/future operational-window fixtures cannot supply validated pins;
they use this branch while retaining their independently diagnosed window reason.
Their template demand value is preserved but is not exposed as an observed count.
No schema or G1 result column is added by this fixture convention. Missing/invalid
sources retain the existing typed result and null unknown bounds; a diagnosed
bad stored clock is not a fabricated source refresh.

All 124 case IDs remain fixed. Active research-heartbeat and scheduled-run timeout
equal/after predicates also have read-only domain vectors in wire-cases.json;
the projection corpus contains no corresponding active boundary input. The
temporal checker validates those small vectors as contract assertions only.
