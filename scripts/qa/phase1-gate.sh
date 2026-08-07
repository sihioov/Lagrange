#!/usr/bin/env bash
# phase1-gate.sh - Phase 1 invite-only multi-user release gate (POSIX/CI twin).
#
# Logic twin of scripts/qa/phase1-gate.ps1: same check list E1..E7, same
# single machine-readable verdict on stdout:
#   VERDICT: APPROVED
#   VERDICT: BLOCKED_EXTERNAL_DATA_RIGHTS
#
# APPROVED only when EVERY check passes AND the written-rights metadata
# artifact is ACTIVE AND the vendor Auth0 suite passes. Missing evidence,
# failing suites, or documented BLOCKED_EXTERNAL written-rights/vendor
# conditions => BLOCKED_EXTERNAL_DATA_RIGHTS (never a false success).
#
# Runs natively in WSL2/CI: cargo via the inside-WSL lane (CARGO_TARGET_DIR
# and DATABASE_URL honored from env), validate-policy.sh for the pre-Member
# gate (A1). The Playwright lane (E7) uses node/npx when present, else
# falls back to the PHASE1_E7_TRANSCRIPT file written by the .ps1 twin -
# without evidence the gate stays BLOCKED.
#
# Exit codes: 0 = verdict emitted; 2 = gate could not run.
#
# Twin: scripts/qa/phase1-gate.ps1
set -u

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
data_rights_dir="$root/configs/data-rights"
evidence_dir="$root/.omo/evidence"
ev_path="$evidence_dir/task-28-lagrange-station-implementation.json"
transcript_dir="$evidence_dir/task-28-transcripts"
wsl_db_url="${WSL_DATABASE_URL:-postgres://postgres:lagrange@127.0.0.1:5432/postgres}"
skip_playwright="${PHASE1_SKIP_PLAYWRIGHT:-0}"
mkdir -p "$evidence_dir" "$transcript_dir"

checks=""

add_check() { # add_check <id> <name> <result> <detail>
  checks="$checks
$1 $2 $3 $4"
  printf 'CHECK %s %s = %s  %s\n' "$1" "$2" "$3" "$4"
}

# --- inside-WSL cargo lane --------------------------------------------------
run_cargo() { # run_cargo <name> <transcript> <command...>
  local name="$1" t="$2"; shift 2
  {
    export PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/root/.cargo/bin"
    export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/root/lagrange-target}"
    export DATABASE_URL="$wsl_db_url"
    cd "$root" || exit 2
    cargo test "$@" -- --nocapture
  } >"$t" 2>&1
  return $?
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
    add_check E2 vendor-auth0 PASS "real Auth0 tenant suite green (transcript $t)"
  else
    add_check E2 vendor-auth0 BLOCKED_EXTERNAL "no Auth0 tenant/credentials on this host (suite panics BLOCKED_EXTERNAL); transcript $t"
  fi
}

# --- E3 simulator / E4 invite+MFA -------------------------------------------
test_auth0_simulator() {
  local t="$transcript_dir/E3-auth0-simulator.txt"
  if run_cargo auth0-simulator "$t" -p auth --test auth0_simulator; then
    add_check E3 auth0-simulator PASS "contract suite green (transcript $t)"
  else
    add_check E3 auth0-simulator FAIL "contract suite failed (transcript $t)"
  fi
}

test_invite_mfa() {
  local t="$transcript_dir/E4-auth0-invite-mfa.txt"
  if run_cargo auth0-invite-mfa "$t" -p auth protocol && \
     run_cargo auth0-invites "$t" -p auth invites && \
     run_cargo auth0-stepup "$t" -p auth stepup; then
    add_check E4 auth0-invite-mfa PASS "protocol/invites/stepup suites green (transcript $t)"
  else
    add_check E4 auth0-invite-mfa FAIL "invite/MFA suites failed (transcript $t)"
  fi
}

# --- E5 integrated five-user suite ------------------------------------------
test_five_user() {
  local t="$transcript_dir/E5-phase1-five-user.txt"
  if run_cargo phase1-five-user "$t" -p api-server --test phase1_gate; then
    add_check E5 phase1-five-user PASS "five-user suite green (transcript $t)"
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
  node_bin="$(command -v node 2>/dev/null || command -v node.exe 2>/dev/null || true)"
  npx_bin="$(command -v npx 2>/dev/null || command -v npx.cmd 2>/dev/null || true)"
  if [ -z "$node_bin" ] || [ -z "$npx_bin" ]; then
    if [ -n "${PHASE1_E7_TRANSCRIPT:-}" ] && [ -f "$PHASE1_E7_TRANSCRIPT" ] \
       && grep -qE "passed" "$PHASE1_E7_TRANSCRIPT" && ! grep -qE "failed|No tests" "$PHASE1_E7_TRANSCRIPT"; then
      add_check E7 playwright-phase1 PASS "phase1 e2e green via external transcript $PHASE1_E7_TRANSCRIPT"
      return 0
    fi
    add_check E7 playwright-phase1 BLOCKED_EXTERNAL "no node/npx on PATH; run phase1-gate.ps1 for the Playwright lane (or set PHASE1_E7_TRANSCRIPT)"
    return 0
  fi
  # Launch synthetic-api mock (38180) + next dev app (33000) detached.
  local web_dir="$root/apps/web"
  local mock_pid="" app_pid=""
  ( cd "$web_dir" && nohup "$node_bin" tests/e2e/support/synthetic-api.mjs >"$transcript_dir/mock.stdout.txt" 2>&1 & echo $! >"$transcript_dir/mock.pid" )
  sleep 1
  mock_pid="$(cat "$transcript_dir/mock.pid" 2>/dev/null || true)"
  ready_port() { # ready_port <port> <attempts>
    local p="$1" n="$2" i=0
    while [ $i -lt "$n" ]; do
      if (exec 3<>/dev/tcp/127.0.0.1/"$p") 2>/dev/null; then exec 3>&-; return 0; fi
      sleep 0.25; i=$((i+1))
    done
    return 1
  }
  if ! ready_port 38180 40; then
    add_check E7 playwright-phase1 BLOCKED_EXTERNAL "mock did not become ready on 38180"
    return 0
  fi
  ( cd "$web_dir" && PORT=33000 SYNTHETIC_API_ORIGIN=http://127.0.0.1:38180 \
      nohup "$node_bin" node_modules/next/dist/bin/next dev -p 33000 >"$transcript_dir/app.stdout.txt" 2>&1 & echo $! >"$transcript_dir/app.pid" )
  app_pid="$(cat "$transcript_dir/app.pid" 2>/dev/null || true)"
  if ! ready_port 33000 120; then
    add_check E7 playwright-phase1 BLOCKED_EXTERNAL "next app did not become ready on 33000"
    [ -n "$mock_pid" ] && kill "$mock_pid" 2>/dev/null || true
    return 0
  fi
  ( cd "$web_dir" && "$npx_bin" playwright test tests/e2e/phase1 >"$t" 2>&1 )
  local code=$?
  [ -n "$app_pid" ] && kill "$app_pid" 2>/dev/null || true
  [ -n "$mock_pid" ] && kill "$mock_pid" 2>/dev/null || true
  if [ $code -eq 0 ] && ! grep -qE "[0-9]+ failed|No tests found|no tests" "$t"; then
    add_check E7 playwright-phase1 PASS "phase1 e2e green (transcript $t)"
  else
    add_check E7 playwright-phase1 BLOCKED_EXTERNAL "EVIDENCE_MISSING or failed: exit=$code (transcript $t)"
  fi
}

# ---------------------------------------------------------------------------
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
