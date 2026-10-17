# R2 functional smoke fixture correction — Wave89

R2-04/09 remain DOING. Owner renewed the instruction to continue through final
R2 acceptance on 2026-10-17 KST. Wave88 required CI succeeded, but its separate
functional smoke failed before image builds/publication: the disposable Bash
fixture omitted `daily_input_admitter`, required by migration0060. See the
[preserved full recovery](2026-10-17-r2-integrated-candidate-ci.md).

The coordinator should have continued this bounded correction after recovering
the two runs. Ending that package left the next authorized work unstarted.
Self-heartbeat0257875e now covers continuation of the remaining R2 dependency
chain, every five minutes until17:40:44UTC, with renewal before expiry. This is
not a claim that the native Ralph goal resumed: its stored state is still
`blocked`, and the available goal tools provide no resume operation. A heartbeat
on the same daemon cannot protect against daemon outage.

The two executable changes are:

- [Bash smoke](../../scripts/qa/research-worker-smoke.sh): create the seventh
  disposable role with LOGIN/NOINHERIT/NOSUPERUSER/NOCREATEDB/NOCREATEROLE/
  NOREPLICATION/NOBYPASSRLS, CONNECT and public USAGE only. This matches the
  existing [bootstrap](../../deploy/db/bootstrap-roles.sh) contract. Schema
  CREATE remains exclusive to migration_owner. Migration0060, production
  bootstrap, workflow and production permissions are unchanged.
- [CI contract tests](../../scripts/ci/test_ci_contract.py): seven behavioral
  regressions exercise the actual extracted cleanup functions with command
  stubs: success, failed cleanup after success, original failure preservation,
  unknown absence query, leftover resources, and both probe-failure cases.
  They invoke no Docker or database. The existing policy job discovers them.

Cleanup commands now report their actual exit codes and continue remaining
cleanup after a failure. Project-label queries distinguish absence, presence
and failed/unknown queries without printing resource names or command output.
The six probe volumes have their own removal result. An original nonzero smoke
exit is preserved; cleanup failure after otherwise successful work returns1.
The context image is removed only after its successful creation. The final
functional PASS marker alone remains insufficient without successful cleanup
and a successful job conclusion.

Coordinator reviewed the complete two-file diff and compared all1149 prior
input pins:1147 unchanged, exactly these two changed. New tests are
WRITTEN_UNEXECUTED. No local tests, shell/project validators, compilers, Docker,
SQL or provider operations were run. This is source review, not compiler or
execution acceptance, and no additional independent reviewer is claimed.
The PowerShell smoke path is outside this Linux workflow correction.

Next: freeze this commit and all carried inventories, query every existing run
at that exact head, verify remote SHA, and dispatch one required CI plus one
functional research smoke if absent. Recover full logs/checkouts/named results
and cleanup, keeping old failures immutable. Then continue actual image/host
preparation within the existing authority. Actual root access, candidate images,
approved-base host flow, G3/G4, Owner use, three consecutive trading days and
final C_R2 remain open. R3 is not being started.
