#!/usr/bin/env bash
# phase1-gate.sh - Phase 1 invite-only multi-user release gate (native Linux).
#
# Emits ONE machine-readable verdict on stdout:
#   VERDICT: APPROVED
#   VERDICT: BLOCKED_EXTERNAL_DATA_RIGHTS
#
# APPROVED only when EVERY check passes AND the written-rights metadata
# artifact is ACTIVE AND the vendor Auth0 suite passes. Missing evidence,
# failing suites, or documented BLOCKED_EXTERNAL written-rights/vendor
# conditions => BLOCKED_EXTERNAL_DATA_RIGHTS (never a false success).
#
# HOST: this file used to be the "inside-WSL twin". It replaced PATH with one
# ending in /root/.cargo/bin, forced CARGO_TARGET_DIR=/root/lagrange-target,
# and pointed DATABASE_URL at the inside-WSL 127.0.0.1:5432. Services now run
# on native Linux only, so all three are gone and this script uses the same
# conventions as its siblings phase2-gate.sh and phase3-gate.sh: the caller's
# cargo, $root/target, and the QA database on 127.0.0.1:${LAGRANGE_QA_DB_PORT}.
# scripts/qa/phase1-gate.ps1 is the retired Windows-era bridge (it drove this
# lane through `wsl -d Ubuntu`); it is no longer a fallback for anything here.
#
# Environment:
#   DATABASE_URL                  QA database URL (default built from the port
#                                 below); E5 is the DB-gated check
#   LAGRANGE_QA_DB_PORT           QA database port (default 55432)
#   CARGO_TARGET_DIR              cargo target directory (default $root/target)
#   LAGRANGE_AUTH0_DOMAIN         E2 needs all three. Without them the vendor
#   LAGRANGE_AUTH0_CLIENT_ID      suite panics BLOCKED_EXTERNAL by design and
#   LAGRANGE_AUTH0_CLIENT_SECRET  E2 is recorded BLOCKED_EXTERNAL, never PASS.
#                                 This gate deliberately does NOT read
#                                 deploy/secrets/auth0_client_secret itself:
#                                 injecting a credential stays the operator's
#                                 explicit act, and a gate that reads secret
#                                 files acquires a surface it does not need.
#   PHASE1_SKIP_PLAYWRIGHT=1      record E7 as EVIDENCE_MISSING without running
#   PHASE1_E7_TRANSCRIPT          external Playwright transcript to accept as E7
#
# The QA database must ALREADY be running (deploy/qa/qa-db.compose.yml). This
# gate does not start or stop it: full-system-gate.sh owns that lifecycle for
# the composite run, and a child that tore down the shared database would leave
# every later phase without one.
#
# Exit codes: 0 = verdict emitted; 2 = gate could not run.
set -u

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
data_rights_dir="$root/configs/data-rights"
evidence_dir="$root/.omo/evidence"
ev_path="$evidence_dir/task-28-lagrange-station-implementation.json"
transcript_dir="$evidence_dir/task-28-transcripts"
qa_port="${LAGRANGE_QA_DB_PORT:-55432}"
db_url="${DATABASE_URL:-postgres://postgres:lagrange@127.0.0.1:${qa_port}/postgres}"
skip_playwright="${PHASE1_SKIP_PLAYWRIGHT:-0}"

# A gate that cannot run must say so and stop: exit 2, no verdict.
#
# The tools are probed on the caller's PATH because that is now the PATH the
# cargo lane below actually uses. While this file was the inside-WSL twin the
# probe deliberately looked at /root/.cargo/bin instead, since run_cargo threw
# the caller's PATH away -- on Windows Git Bash `cargo` then disappeared, the
# failure was captured into a transcript as `cargo: command not found`, and the
# gate published a verdict built from four checks that never ran.
command -v cargo >/dev/null 2>&1 || {
  echo "ENV ERROR: cargo not found on PATH" >&2
  exit 2
}
command -v python3 >/dev/null 2>&1 || {
  echo "ENV ERROR: python3 not found on PATH (E6 restore-policy needs it)" >&2
  exit 2
}

# The QA database is REQUIRED by E5, and a gate that cannot reach it must not
# emit a verdict. phase3-gate.sh learned this the expensive way (§3.3): with a
# dead database its checks recorded `PoolTimedOut` as suite failures and it
# published DENIED, which means "a real defect" and sent someone hunting a bug
# in code that was fine. Exit 2 already means "could not run, no verdict".
#
# Unlike phase2/phase3 this gate does NOT bring the database up. It is a child
# of full-system-gate.sh, which owns the shared instance.
db_host_port="$(python3 - "$db_url" <<'PYEOF'
import sys, urllib.parse
u = urllib.parse.urlparse(sys.argv[1])
print(u.hostname or "127.0.0.1", u.port or 5432)
PYEOF
)" || {
  echo "ENV ERROR: could not parse DATABASE_URL" >&2
  exit 2
}
db_host="${db_host_port% *}"
db_port="${db_host_port#* }"
if ! (exec 3<>"/dev/tcp/$db_host/$db_port") 2>/dev/null; then
  echo "ENV ERROR: QA database unreachable at $db_host:$db_port" >&2
  echo "           start it with: docker compose -p lagrange-qa -f deploy/qa/qa-db.compose.yml up -d --wait qa-db" >&2
  echo "           (or point DATABASE_URL / LAGRANGE_QA_DB_PORT at a running one)" >&2
  exit 2
fi
exec 3>&-

mkdir -p "$evidence_dir" "$transcript_dir"

checks=""

add_check() { # add_check <id> <name> <result> <detail>
  checks="$checks
$1 $2 $3 $4"
  printf 'CHECK %s %s = %s  %s\n' "$1" "$2" "$3" "$4"
}

# --- native cargo lane ------------------------------------------------------
# run_cargo <name> <transcript> <cargo args...> [-- <libtest args...>]
#
# Returns 0 only when cargo exited 0 AND at least one test actually ran, and
# leaves that count in $run_cargo_ran. Both halves are load-bearing:
#
#   * `cargo test <filter>` that selects nothing exits 0 with "0 passed".
#     Recording that as evidence lets the gate approve a release on the
#     strength of tests that never executed (phase2-gate.sh has counted for
#     this reason since Todo 35).
#   * The caller's libtest arguments are kept SEPARATE from the cargo ones and
#     spliced in after a single `--`. The old signature appended `-- --nocapture`
#     to whatever it was handed, so E2's call produced
#         cargo test -p auth --test vendor_auth0 -- --ignored -- --nocapture
#     with two separators. libtest read the second `--` and `--nocapture` as
#     name filters, matched nothing, and exited 0 having run 0 of the 5 tests
#     -- which this gate recorded as "real Auth0 tenant suite green". The .ps1
#     twin built the same command with one separator, so the bug only ever bit
#     the POSIX lane, which is now the only lane.
run_cargo_ran=0
run_cargo() {
  local name="$1" t="$2"; shift 2
  local cargo_args=() test_args=() seen_sep=0 a
  for a in "$@"; do
    if [ "$seen_sep" -eq 0 ] && [ "$a" = "--" ]; then seen_sep=1; continue; fi
    if [ "$seen_sep" -eq 0 ]; then cargo_args+=("$a"); else test_args+=("$a"); fi
  done
  {
    export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}"
    export DATABASE_URL="$db_url"
    cd "$root" || exit 2
    cargo test ${cargo_args[@]+"${cargo_args[@]}"} \
      -- ${test_args[@]+"${test_args[@]}"} --nocapture
  } >"$t" 2>&1
  local rc=$?
  run_cargo_ran="$(grep -Eo '^test result: ok\. [0-9]+ passed' "$t" \
    | grep -Eo '[0-9]+' | awk '{s+=$1} END {print s+0}')"
  [ "$rc" -eq 0 ] || return "$rc"
  [ "${run_cargo_ran:-0}" -gt 0 ] || return 90
  return 0
}

# --- E1 written-rights artifact ---------------------------------------------
test_written_rights() {
  local found=""
  local f
  for f in "$data_rights_dir"/*.json; do
    [ -f "$f" ] || continue
    case "$f" in *.schema.json) continue ;; esac
    local hex ref lifecycle
    hex="$(python3 -c "import json,sys;d=json.load(open('$f'));print(d.get('contract_document',{}).get('document_hash',{}).get('hex',''))" 2>/dev/null)"
    ref="$(python3 -c "import json,sys;d=json.load(open('$f'));print(d.get('contract_document',{}).get('document_reference',''))" 2>/dev/null)"
    lifecycle="$(python3 -c "import json,sys;d=json.load(open('$f'));print(d.get('lifecycle',''))" 2>/dev/null)"
    if [ -z "$hex" ] && [ -z "$ref" ] && [ -z "$lifecycle" ]; then continue; fi
    found="$(basename "$f")"
    local zeros="0000000000000000000000000000000000000000000000000000000000000000"
    if [ "$lifecycle" = "ACTIVE" ] && [ "$hex" != "$zeros" ] && [ "${ref#vault://}" = "$ref" ]; then
      add_check E1 written-rights PASS "$found ACTIVE with real document hash ${hex:0:12}... and reference $ref"
      return 0
    fi
    local why="lifecycle=$lifecycle"
    [ "$hex" = "$zeros" ] && why="placeholder zeroed document hash"
    [ "${ref#vault://}" != "$ref" ] && why="unresolvable vault reference"
    add_check E1 written-rights BLOCKED_EXTERNAL "$found is NOT an ACTIVE written-rights artifact ($why)"
    return 0
  done
  add_check E1 written-rights BLOCKED_EXTERNAL "no entitlement metadata artifact in $data_rights_dir"
}

# --- E2 vendor Auth0 --------------------------------------------------------
test_vendor_auth0() {
  local t="$transcript_dir/E2-vendor-auth0.txt"
  if run_cargo vendor-auth0 "$t" -p auth --test vendor_auth0 -- --ignored; then
    add_check E2 vendor-auth0 PASS "real Auth0 tenant suite green, $run_cargo_ran test(s) (transcript $t)"
  else
    add_check E2 vendor-auth0 BLOCKED_EXTERNAL "no Auth0 tenant/credentials on this host (suite panics BLOCKED_EXTERNAL without LAGRANGE_AUTH0_*); transcript $t"
  fi
}

# --- E3 simulator / E4 invite+MFA -------------------------------------------
test_auth0_simulator() {
  local t="$transcript_dir/E3-auth0-simulator.txt"
  if run_cargo auth0-simulator "$t" -p auth --test auth0_simulator; then
    add_check E3 auth0-simulator PASS "contract suite green, $run_cargo_ran test(s) (transcript $t)"
  else
    add_check E3 auth0-simulator FAIL "contract suite failed (transcript $t)"
  fi
}

# Three suites, three transcripts. They shared one path before, each `>`
# truncating the last, so the evidence file kept only the stepup run while
# claiming all three -- and the per-suite test count below would have been read
# from that same last transcript.
#
# `--test <name>` selects the test TARGET. The bare `-p auth protocol` form used
# before passed "protocol" to libtest as a test-NAME filter, ran it across every
# target in the package, matched nothing, and exited 0 -- so this check reported
# "protocol/invites/stepup suites green" having executed 0, 0 and 4 tests. The
# targets hold 32, 15 and 5. Both twins carried the same form.
test_invite_mfa() {
  local tp="$transcript_dir/E4a-auth0-protocol.txt"
  local ti="$transcript_dir/E4b-auth0-invites.txt"
  local ts="$transcript_dir/E4c-auth0-stepup.txt"
  local total=0 ok=1
  if run_cargo auth0-protocol "$tp" -p auth --test protocol; then total=$((total + run_cargo_ran)); else ok=0; fi
  if run_cargo auth0-invites "$ti" -p auth --test invites; then total=$((total + run_cargo_ran)); else ok=0; fi
  if run_cargo auth0-stepup "$ts" -p auth --test stepup; then total=$((total + run_cargo_ran)); else ok=0; fi
  if [ "$ok" -eq 1 ]; then
    add_check E4 auth0-invite-mfa PASS "protocol/invites/stepup suites green, $total test(s) (transcripts $tp $ti $ts)"
  else
    add_check E4 auth0-invite-mfa FAIL "invite/MFA suites failed (transcripts $tp $ti $ts)"
  fi
}

# --- E5 integrated five-user suite ------------------------------------------
# DB-gated. The suite skips itself when DATABASE_URL is unset -- and a skipped
# test still counts as passed, so the count guard cannot see it. The database
# guard at the top of this file is what keeps that from becoming a PASS; the
# grep below is the second lock on the same door.
test_five_user() {
  local t="$transcript_dir/E5-phase1-five-user.txt"
  if run_cargo phase1-five-user "$t" -p api-server --test phase1_gate; then
    if grep -q "SKIP: DATABASE_URL not set" "$t"; then
      add_check E5 phase1-five-user FAIL "suite skipped itself: no DATABASE_URL inside the lane (transcript $t)"
    else
      add_check E5 phase1-five-user PASS "five-user suite green, $run_cargo_ran test(s) (transcript $t)"
    fi
  elif grep -Eq "no test target named|could not find|No tests" "$t"; then
    add_check E5 phase1-five-user BLOCKED_EXTERNAL "EVIDENCE_MISSING: phase1_gate suite not present yet (transcript $t)"
  else
    add_check E5 phase1-five-user FAIL "five-user suite failed (transcript $t)"
  fi
}

# --- E6 pre-Member restore policy gate (A1) ---------------------------------
test_restore_policy() {
  local t="$transcript_dir/E6-restore-policy.txt"
  local set_path="$root/scripts/backup/tests/fixtures/complete"
  if bash "$root/scripts/backup/validate-policy.sh" --set "$set_path" --gate premember >"$t" 2>&1 \
     && grep -q "POLICY OK.*gate premember" "$t"; then
    add_check E6 restore-policy PASS "pre-Member policy gate A1 OK (transcript $t)"
  else
    add_check E6 restore-policy FAIL "pre-Member policy gate A1 rejected (transcript $t)"
  fi
}

# --- E7 Playwright phase1 ---------------------------------------------------
test_playwright_phase1() {
  local t="$transcript_dir/E7-playwright-phase1.txt"
  if [ "$skip_playwright" = "1" ]; then
    add_check E7 playwright-phase1 BLOCKED_EXTERNAL "EVIDENCE_MISSING: skipped by PHASE1_SKIP_PLAYWRIGHT=1"
    return 0
  fi
  local node_bin npx_bin
  node_bin="$(command -v node 2>/dev/null || true)"
  npx_bin="$(command -v npx 2>/dev/null || true)"
  if [ -z "$node_bin" ] || [ -z "$npx_bin" ]; then
    if [ -n "${PHASE1_E7_TRANSCRIPT:-}" ] && [ -f "$PHASE1_E7_TRANSCRIPT" ] \
       && grep -qE "passed" "$PHASE1_E7_TRANSCRIPT" && ! grep -qE "failed|No tests" "$PHASE1_E7_TRANSCRIPT"; then
      add_check E7 playwright-phase1 PASS "phase1 e2e green via external transcript $PHASE1_E7_TRANSCRIPT"
      return 0
    fi
    add_check E7 playwright-phase1 BLOCKED_EXTERNAL "no node/npx on PATH (install Node, or set PHASE1_E7_TRANSCRIPT to a transcript produced elsewhere)"
    return 0
  fi
  local web_dir="$root/apps/web"
  # Without installed dependencies neither child can start, and the lane would
  # otherwise discover that only after binding ports and shelling out.
  #
  # Resolve the package the way the children will instead of testing a path.
  # apps/* are npm workspaces, so `npm ci` at the root hoists @playwright/test
  # into $root/node_modules and apps/web/node_modules is never created at all.
  # A directory test there reported "dependencies not installed" against a tree
  # that had just installed them -- and the remedy it printed was the very
  # command that had already been run, so the check could never clear.
  if ! "$node_bin" -e 'require.resolve("@playwright/test",{paths:[process.argv[1]]})' "$web_dir" >/dev/null 2>&1; then
    # `npm ci` belongs at the repository root: apps/* are npm workspaces and the
    # only package-lock.json is the root one, so running it inside apps/web fails.
    add_check E7 playwright-phase1 BLOCKED_EXTERNAL "EVIDENCE_MISSING: @playwright/test does not resolve from apps/web (run npm ci at the repository root, then npx playwright install)"
    return 0
  fi
  # Same hoisting rule applies to the app binary this lane executes directly:
  # apps/web/node_modules/next/... does not exist under npm workspaces. Resolve
  # it once here so a missing install is reported as such, rather than as the
  # "next dev exited immediately / port taken" symptom it produces downstream.
  local next_bin
  next_bin="$("$node_bin" -e 'process.stdout.write(require.resolve("next/dist/bin/next",{paths:[process.argv[1]]}))' "$web_dir" 2>/dev/null || true)"
  if [ -z "$next_bin" ] || [ ! -f "$next_bin" ]; then
    add_check E7 playwright-phase1 BLOCKED_EXTERNAL "EVIDENCE_MISSING: next does not resolve from apps/web (run npm ci at the repository root)"
    return 0
  fi
  # Ports are overridable because this host runs several worktrees at once and
  # the fixed pair collides between them.
  local mock_port="${PHASE1_E7_MOCK_PORT:-38180}"
  local app_port="${PHASE1_E7_APP_PORT:-33000}"
  local mock_pid="" app_pid=""

  ready_port() { # ready_port <port> <attempts>
    local p="$1" n="$2" i=0
    while [ $i -lt "$n" ]; do
      if (exec 3<>/dev/tcp/127.0.0.1/"$p") 2>/dev/null; then exec 3>&-; return 0; fi
      sleep 0.25; i=$((i+1))
    done
    return 1
  }

  # An open port is not proof that OUR child opened it.
  #
  # Both children used to be launched into a subshell whose PID was recorded
  # instead of theirs, and readiness was decided by ready_port alone. On this
  # host that combination tested a stranger's server: the mock died with
  # EADDRINUSE and next dev died with "Cannot find module", yet both ports
  # answered -- another worktree was serving them -- so the lane went on to run
  # Playwright against an application this gate never started. It failed for an
  # unrelated reason that day; nothing in the check would have noticed if it had
  # passed. `exec` makes $! the child itself, and liveness is now checked before
  # and after the port.
  ( cd "$web_dir" && SYNTHETIC_API_PORT="$mock_port" \
      exec "$node_bin" tests/e2e/support/synthetic-api.mjs ) \
    >"$transcript_dir/mock.stdout.txt" 2>&1 &
  mock_pid=$!
  sleep 1
  if ! kill -0 "$mock_pid" 2>/dev/null; then
    add_check E7 playwright-phase1 BLOCKED_EXTERNAL "synthetic-api mock exited immediately (see $transcript_dir/mock.stdout.txt; port $mock_port may be taken by another worktree)"
    return 0
  fi
  if ! ready_port "$mock_port" 40 || ! kill -0 "$mock_pid" 2>/dev/null; then
    add_check E7 playwright-phase1 BLOCKED_EXTERNAL "mock did not become ready on $mock_port (see $transcript_dir/mock.stdout.txt)"
    kill "$mock_pid" 2>/dev/null || true
    return 0
  fi

  # The spec side reads SYNTHETIC_API_ORIGIN; the app itself resolves its
  # upstream from API_INTERNAL_URL. Without the second one the app renders every
  # page against the absent real API and the whole lane 500s.
  ( cd "$web_dir" && PORT="$app_port" \
      SYNTHETIC_API_ORIGIN="http://127.0.0.1:$mock_port" \
      API_INTERNAL_URL="http://127.0.0.1:$mock_port" \
      exec "$node_bin" "$next_bin" dev -p "$app_port" ) \
    >"$transcript_dir/app.stdout.txt" 2>&1 &
  app_pid=$!
  sleep 1
  if ! kill -0 "$app_pid" 2>/dev/null; then
    add_check E7 playwright-phase1 BLOCKED_EXTERNAL "next dev exited immediately (see $transcript_dir/app.stdout.txt; port $app_port may be taken by another worktree)"
    kill "$mock_pid" 2>/dev/null || true
    return 0
  fi
  if ! ready_port "$app_port" 120 || ! kill -0 "$app_pid" 2>/dev/null; then
    add_check E7 playwright-phase1 BLOCKED_EXTERNAL "next app did not become ready on $app_port (see $transcript_dir/app.stdout.txt)"
    kill "$app_pid" 2>/dev/null || true
    kill "$mock_pid" 2>/dev/null || true
    return 0
  fi

  ( cd "$web_dir" && PLAYWRIGHT_BASE_URL="http://127.0.0.1:$app_port" \
      SYNTHETIC_API_ORIGIN="http://127.0.0.1:$mock_port" \
      "$npx_bin" playwright test tests/e2e/phase1 >"$t" 2>&1 )
  local code=$?
  kill "$app_pid" 2>/dev/null || true
  kill "$mock_pid" 2>/dev/null || true
  if [ $code -eq 0 ] && ! grep -qE "[0-9]+ failed|No tests found|no tests" "$t"; then
    add_check E7 playwright-phase1 PASS "phase1 e2e green (transcript $t)"
  else
    add_check E7 playwright-phase1 BLOCKED_EXTERNAL "EVIDENCE_MISSING or failed: exit=$code (transcript $t)"
  fi
}

# ---------------------------------------------------------------------------
echo "== Phase 1 release gate =="
echo "   cargo:    $(command -v cargo)"
echo "   target:   ${CARGO_TARGET_DIR:-$root/target}"
echo "   database: $db_host:$db_port"

test_written_rights
test_vendor_auth0
test_auth0_simulator
test_invite_mfa
test_five_user
test_restore_policy
test_playwright_phase1

hard_fail="$(printf '%s' "$checks" | grep -c ' FAIL ' || true)"
blocked="$(printf '%s' "$checks" | grep -c ' BLOCKED_EXTERNAL ' || true)"

if [ "$hard_fail" -eq 0 ] && [ "$blocked" -eq 0 ]; then
  verdict="APPROVED"
else
  verdict="BLOCKED_EXTERNAL_DATA_RIGHTS"
fi

printf 'VERDICT: %s\n' "$verdict"
printf 'EVIDENCE: %s\n' "$ev_path"
printf '%s\n' "$checks" | sed '/^$/d' | sed 's/^/  /'
if [ "$verdict" = "BLOCKED_EXTERNAL_DATA_RIGHTS" ]; then
  printf 'BLOCKED_REASONS:\n'
  printf '%s\n' "$checks" | sed '/^$/d' | grep -v ' PASS ' | sed 's/^/  - /'
  printf 'NOTE: BLOCKED_EXTERNAL_DATA_RIGHTS is the correct Phase-1 outcome when written-rights are not ACTIVE or vendor Auth0 cannot pass. Member KR-derived surfaces stay denied; Owner-only continues; no market switch and no release success claimed.\n'
fi

python3 - "$ev_path" "$verdict" "$checks" "$transcript_dir" <<'PYEOF'
import json, sys, datetime
path, verdict, checks, tdir = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4]
items = []
for line in checks.splitlines():
    parts = line.split()
    if len(parts) >= 4:
        items.append({"id": parts[0], "name": parts[1], "result": parts[2],
                      "detail": " ".join(parts[3:])})
summary = {
    "gate": "phase1", "task": 28, "verdict": verdict,
    "emitted_at": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "checks": items, "evidence_dir": tdir,
}
with open(path, "w", encoding="utf-8") as f:
    json.dump(summary, f, indent=2, ensure_ascii=False)
PYEOF
printf 'EVIDENCE_WRITTEN: %s\n' "$ev_path"
exit 0
