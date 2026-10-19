# R2 G4 host compatibility: recovered inventory and maintenance-v3 source

Status: **Wave113 survey terminal recovered; Wave114 source written, hosted validation pending.**
Scope: R2-04/R2-09. The accepted application e639169 and its image/wholeflow evidence are unchanged. The previous maintenance-v2 candidate3de6420 remains separately accepted; this new source has no execution acceptance yet.

## One actual survey, fully recovered

Wave113 승인 조회 a4는2026-10-19 05:06:05UTC 한 번 실행해12단계와 최종 상태 대조까지 회수했다. 종료는exit2/METADATA_RECOVERED_WITH_GAPS이며 유일한 gap은release lock의 실제 부재다. 잠금 파일을 만들지 않았고56유닛 두 번의 metadata가 같았다. 키metadata·자원·제한된 열린FD/maps 관측도 회수했으나 전역/미래/외부 writer 차단 증명은 아니다. 승인0543fefc는 소진됐고 추가 조회는 하지 않는다. Wave114는 실제 PG ancestry와 disabled persistent failed 유닛, 비대칭 관측된 activation 속성을 보존하는 maintenance-v3 소스를 준비했다. 기존41개와 신규14개(총55개) maintenance 회귀는 작성·직접 검토 단계이며 새 후보 hosted CI 전이다. 인수된 앱e639169/12이미지/실제 통합과 도구3de6420/기존 근거는 불변이다. G4 중단·일관 백업·복구·배포·Owner·3연속 실제거래일·C_R2는 미완료다.

The approved read-only a4 finished at05:06:05UTC. Its twelve stages include protected metadata, key metadata, resources, bounded writer FD/maps inventory and final identity rechecks. `collectionCompleted=true`, `acceptedObservation=false`, and `operatingAdmission=false` are preserved. The146299 filtered stdout bytes and59 immutable input pins were recovered; stderr was empty. No production writes, lock creation, SQL, provider calls or lifecycle operations occurred.

| Receipt | SHA256 |
|---|---|
| Wave113 owner authorization, consumed | 0543fefc26572df801fcc6bc0c7e50b18be9a0d5d215fca5c3ce97dfb9bd4937 |
| a4 terminal observation | 94f4c97f4cd427839c4a81df7cfd0771d8deb6a0cfd6da6c6f87ac33d3052cac |
| a4 attempt closure | 7e855a7638f5badc6c4d39027157dfcd50bb2b113429a8b63734d939afd79ce2 |
| Post-survey readiness | e3862a5df77f57ffe6235af51b611c01ceca40e2229a6c5affe703933ec3e6a7 |

The only gap is `/opt/lagrange/.lagrange-release.lock`, whose leaf was absent under the twice-held protected parent. It was not created. All56 unit metadata records matched the second pass. Final container IDs/names/current were compared; this was not a second complete container health/state observation. The key was root:root0600,64bytes,single-link,noACL; its bytes were not read. Available space was78929641472bytes; MemAvailable9584404KiB and SwapFree587940KiB. The scan inspected439processes/4336FDs with one vanished FD and no reported candidate. Existing-open descriptor and mapped-path observations do not fence future, container-internal or external writers.

## Three explicit compatibility changes

1. The cold-source contract gains `postgres_ancestry`: every component from `/` to the selected volume, with device,inode,uid,gid,mode. Only an explicitly pinned PostgreSQL path below `/data/docker/volumes/` can trust `/data` at1000:1000/0755. All remaining ancestors retain protected root ownership/no writable group or other/no special bits/noACL. O_NOFOLLOW, held descriptors and final name/identity checks remain. Ordinary daily and private restore roots do not inherit this allowance. The actual named local volume must match `volumes/<volume>/_data` and the exact existing container mount.
2. `unit_graph` selects the existing reciprocal check or explicit `quiescent-observed`. The latter preserves actual `Triggers` and `TriggeredBy` independently and checks each reported edge against the complete stopped inventory, with no pending job or active activation endpoint. It does not fabricate missing reverse properties. Twelve actual forward timer edges lacked reverse observations in both passes; the reason remains UNKNOWN. No live/systemd semantics are inferred from a mock.
3. Explicit `retained-disabled-failure` admits a disabled persistent failed service only with exact generation/fragment pins, failed/failed state, no job/hooks/drop-ins/reload, Restart=no, zero PIDs and recursive empty cgroup observations. Its activation sources must be disabled and stopped. Mandatory backup units cannot use this category. The original transient-terminal category and strict reciprocal mode retain their guards. Historical failed units are never reset, removed, omitted or relaunched.

The exact existing PG parent is root:root0701 and the volume root999:999/1777. **The cold capture guard still requires the immediate parent root:root0700.** No source function changes permissions. A future operating packet must bind the same parent inode, explicitly protect0701→0700 only after independent recovery is actually active, and restore0701. Shared `/data` ownership is not changed. Source pinning expresses trust in the observed owner; it is not protection against arbitrary owner/root mutation, so the separate writer fence remains necessary.

The actual failed service is `lagrange-kis-daily-66b2a8c.service`, invocation54c27089765546a9b3f524740658b507, fragmentSHA19fc57d05809f66fdbd78d1b9f1cfcd36e3c12f99f7d6a5ddacba89cb7d82d5a. The survey did not collect fresh cgroup evidence; operating admission must do so. Five active timers and the disabled KIS/research states must be restored exactly under the later ledger.

## Validation to recover

Three source files change: the file-archive root descriptor helper, maintenance entry point, and maintenance self-test. Archive bytes, member layout, encryption, fixed SQL, staging dependencies, locks, expiry, clone isolation and cleanup policies remain unchanged. No module/import/Compose structure changed, so architecture diagrams remain untouched.

All41 previous maintenance definitions are retained. Fourteen added definitions cover five unit/category/one-way graph cases and nine ancestry/contract cases. Filesystem cases use actual ownership, modes, ACLs, inode mismatch and directory replacement inside a disposable forked chroot; root checks fail instead of skipping if unavailable. Systemd observations are mocks. The existing real Docker cold-cluster suite and every prior required CI group must still pass on the new exact candidate. No project parser/helper/test/compiler was run locally and no independent review was performed.

## Remaining operating inputs and authority

One asynchronous information question is pending for an off-server key-custody identifier (never the key) and external/manual writers beyond known containers/schedules. This is not another survey approval request. Source work and hosted verification continue independently. The a4 approval has been consumed; no new production operation is authorized by this source preparation.

After full new hosted acceptance, prepare the complete bounded G4 outage/cold-five-class capture/private-clone audit/same-ID restoration packet. Independent recovery must be active before outage or exact parent permission change. Missing writer/secret custody evidence cannot be guessed. Deployment uses the accepted application, not the maintenance-tool commit. Backup transition, deployment, actual Owner use, three consecutive actual trading days and final C_R2 remain open. All prior failures, UNKNOWN nested evidence and0120 retained-volume cleanup INCOMPLETE are preserved.

추적729c2a81을05:18:32UTC active로 확인하고 인계63f45651/registry 저장 후d54eea44 삭제를 확인했다.16:30UTC 전/47번째 tick 전 갱신,17:18:32UTC 만료. Native Ralph blocked/no resume API; 동일 에이전트의 제한된 후속 추적이다.
