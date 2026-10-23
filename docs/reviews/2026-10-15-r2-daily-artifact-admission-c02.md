# R2 S5c-C02 — explicit protected CI fixture hierarchy

2026-10-15; R2-04/09, Wave48. Candidate2824dd0/CI38740542364 attempt1 failure is
fully recovered before changes. Sixteen cases still fail at Root::open/UnsafePath;
the exact ancestor/mode/UID/ACL cause is not printed. Prior failure and its supplement
remain immutable. This corrects the unproven test environment assumption, not a
production permission check. Source disposition is direct coordinator review only.

Owned code is artifact.rs private fixture construction and .github/workflows/ci.yml
workspace-test setup/cleanup. Rust takes LAGRANGE_DAILY_ARTIFACT_TEST_ROOT when set;
local HOME still has to pass all production checks. No /tmp fallback, bypass, skip,
shared-ancestor chmod, UID substitution or test serialization is added.

The official hosted workspace job creates a new run/attempt-specific root0755
ancestor below /opt and its runner-owned0700 child. An existing path or symlink fails
setup instead of being adopted. The exact environment path must equal the derived
run/attempt path. Four fixed diagnostic mode/UID/GID rows cover /,/opt,anchor,child;
no provider data or credential is read or printed. All existing test commands and
required gates remain unchanged. Cleanup uses only nonrecursive rmdir for those two
owned paths; nonempty leftovers fail visibly, never recursive privileged deletion.
The existing disposable DB teardown retains its always condition.

These are source changes for an isolated hosted CI fixture. No local sudo/root/
Docker/DB/helper/compiler/test/workflow command is run and no production path is
modified. Actual hierarchy setup, all26Linux behavior tests and cleanup must pass
on the new candidate before bounded acceptance. Different-UID admission enforcement,
real approved-base end-to-end and installer runtime remain later evidence.

The33088production bytes before the test module are byte-identical to2824dd0,
SHA2567c2820e4cfe4a621b7410bb7e608fb6d637f9ff777bc9b35a8eacf96e6ab1892.
All27new names,26Linuxexpected/1excluded, remain unchanged with refreshed anchors.
Scoped standalone2024rustfmt and whitespace checks only. Freeze exact source/harness/
reference pins and query all-event exact-head runs plus remote SHA before one new
hosted dispatch; never rerun/cancel/duplicate. Continuation/frontier/price-extension
preparation remains unregistered and excluded. Ralph continues through the remaining
R2 gates without inferring R3, Owner or three-trading-day completion.
