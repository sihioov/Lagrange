# R5 WP-2A2 source witness 후보 검토 — 2026-10-29

상태: **current producer·보호 reader·checker의 SOURCE/집중 검증 수용, G2a 전체 OPEN**. 기준은 a30edac 위의 최종 46개 입력이며, 아래 최초 후보·PG4 기록은 당시 이력이다.
기준 revision은 `a30edacff987d55b5be7cca25bed7faeec589629` 위의 아래 미커밋 입력이다.
적용 계약은 [ADR-0010](../decisions/0010-owner-beta-paper-v1.md), 후속 순서는
[R5 계획](../superpowers/plans/2026-10-29-r5-paper-implementation.md)의 WP-2A2→2B다.

## 현재 인수 범위 — 2026-10-30 06:30 KST

최종 [소스 수용 영수증](../../target/r5-paper/coordinator/current-source-acceptance.json)은
기준 revision과 미커밋 입력 46개를 묶는다. coordinator가 현재 bytes와 전부 일치함을 재확인했다.
current binding 재검증 누락(F2), publication 이후 시계 차이(CR1), 자식 소유권 초기화 실패 시
정리 누락(CC1)은 소스 수리와 독립 검토를 거쳐 수용했다. 이는 실제 root origin 수용과 구분한다.

| 구분 | 확인한 증거 | 남은 범위 |
| --- | --- | --- |
| 소스·회귀 | Python 98개, Rust current 관련 31개, strict Clippy·fmt 및 독립 검토 | 동일 후보의 전체 required CI는 미실행 |
| 실제 로컬 PostgreSQL | 초기 named 4회와 FIX-A named 5회 실행 PASS, 전용 자원 정리 확인 | 중복 테스트를 포함한 실행 횟수다. root origin은 synthetic fixture |
| 일반 UID 커널·자식 정리 | timer·FD·pidfd·OFD 관련 component, 자식 초기화 rollback 3개, 별도 owned group TERM/KILL·reap 검증 | root 보호 current 경로·공개 Rust reader와의 실제 연계는 미실행 |
| 구조도 | 두 PNG 재렌더·오른쪽 잘림 해소, 실제 코드 근거 32개·최종 소스 핀 보존 | 전체 G3·설치·runtime 인수는 별도 |
| 검증 실행 파일 | ignored 테스트의 동적 ELF no-run 컴파일 PASS, 테스트 실행 0건 | 이미지 glibc 2.36과 ELF 요구 2.39가 달라 메모리 회복 뒤 정적 payload compile/link PASS, 실제 component 실행은 별도 진행 |

캐시 이미지의 Python/libc 정보는 전용 container 한 개에서 실제 확인했고, 해당 CID와 CLI 18개를
정리했다. 기존 container/network/volume ID 집합도 보존했다. 이 환경 조회는 실제 reader 실행이 아니다.
정적 payload는 최초 메모리 preflight 차단을 보존한 뒤, 가용 메모리 회복을 확인한 한정 후속에서
134.865초의 유일 Cargo no-run 컴파일을 통과했다. 실제 파일은 24,942,384bytes이며 독립 readelf와
정적 bytes parser가 외부 interpreter/dependency/GLIBC version 부재를 확인했다. 원 동적 ELF와
제품 소스 46개는 그대로다. 새 private harness는 모의53개와 독립 검토를 통과했고, 최종 파일·receipt를
연결한 모의7개 뒤 전용 컨테이너 한 개의 실제 core component 검증을 별도 범위로 시작했다.
첫 실제 core component는 SUMMARY_ORDER로 UNKNOWN을 회수했다. 컨테이너·프로세스는 정리됐고
원 inode 기록이 유실된 stage 7파일(25.1MB)은 exact-created-inode 계약에 따라 증거로 보존·인계했다.
성공한 테스트 수나 8전이 완료는 확정하지 않았다. 새 private V2 소스에서 inode 기록·진단을 보완하며
기존 실패와 입력을 보존한다. root/public reader 전체 인수와 G2a는 OPEN이다. 최초 timeout preflight 차단과 후속 watchdog 실험 결과도 보존했다.

**G2a 전체와 R5-02는 OPEN/DOING이다.** 실제 source scheduler origin, 보호 current reader·RO mount
교체, pinned helper·sealed Raw·DB 전체 연결, clock/suspend/systemd crash, 연속 실제 3거래일과
운영 활성화 인수는 아직 남았다. WP-3와 활성화의 의존 gate를 소스·PG·component 결과로 넘기지 않는다.
아래 과거 검토의 미착수·미실행·옛 pin 표는 당시 상태를 보여 주는 이력으로 보존한다.

## 회수·직접 확인

Paseo 작업자 `d9da81a9-9985-4368-b8ee-d09996182545`의 finished callback 뒤 실제 idle,
active turn 없음, pending0과 최종 보고를 회수했다. 20개 candidate source pin과 실제 bytes가 일치한다.
고정 계약/gap·WP-2A1 collector3파일·Cargo.lock도 이전 채택 hash를 유지했다.
coordinator는 역할별 helper/SQL grant, scheduler의 arm/dispatch·전용 IPC·원본/DB 연결과 보호 FS reader를
소스에서 대조하고 `python3 scripts/ops/test_owner_beta_paper_evidence.py`를 재실행해 17 pass를 확인했다.
기존 admitter readback 함수는 migration0062가 daily_input_admitter에 부여하므로 새 helper에서 호출 가능하다.

작업자 로그의 Rust fixture6, strict job-queue Clippy, fmt, 기존 daily/backfill/resume fake-host 및 DB
static/self-test PASS를 회수했다. 해당 검사의 최종 candidate pin이 일치하며 무변경 중복 heavy 실행은 하지 않았다.
Rust 결과는 pure wire/negative/schema/FS 거절 회귀이며 실제 DB·root/FD/Docker 동작의 검증이 아니다.
Daily operations는 root fixture 요구로 34 setup 실패·test body0인 BLOCKED다. PASS나 skip으로 세지 않는다.

## G2a가 남은 이유와 보완

초기 후보의 shared valid chain·변조25개·crash prefix14개는 pure parser를 검증한다.
전체 producer→relay→helper→DB→LINKED의 정상 실행이나 실제 runtime 증거는 없다.
새 migration-contract 두 테스트도 정상 terminal 연결의 완전한 positive fixture와 crate compile을 아직 확인하지 않았다.
local-integration harness는 timerfd/FD/Docker/clock lane 코드를 제공했지만 suspend·systemd crash·PG 및 전체
연계 시나리오는 미완성이다. coordinator가 첫 후보를 전체 WP-2A2 완료로 인수하지 않는 이유다.

동일 작업자·기존 파일 범위의 notified 후속으로 정상 offline orchestration 회귀, 실제 SQL join의 positive/negative
fixture, 빠진 opt-in harness 코드와 migration-contract no-run compile을 마무리한다.
source 구현만 수행하며 VM/DB/Docker/systemd/시계/provider를 실행하지 않는다. mock 결과는 SYNTHETIC이다.
실제 PG/RLS·root FS/timerfd/FD·systemd/Docker·clock/suspend/crash 결과와 실제 image/권리/입력·pre/post3거래일은
별도 NOT RUN/인수 조건으로 남는다. 기존 프로토콜 의미·provider 예산·권리·운영 guard를 약화하지 않는다.

새 구조의 `.puml` 근거/PNG는 coordinator가 최종 소스 인계 후 같은 변경에 갱신해야 한다.
현재 이 검토는 그림·전체 required CI·운영 적용 완료를 주장하지 않는다.

## 첫 후보 입력 pin

| 파일 | SHA-256 |
| --- | --- |
| `crates/job-queue/src/bin/owner-beta-runner.rs` | `f8b51cf0ff46778282ee5a320e83ba1acc6a67da443a2cd4df322d690bb70dc7` |
| `crates/job-queue/src/owner_beta/mod.rs` | `a3c5206156107f21f654968982ab3753c2d2f030db6e43e99f2a6a79f1dcbb8a` |
| `crates/job-queue/src/owner_beta/paper/mod.rs` | `068814e76ef1b10515fdcb008e4eb6f25428b5980a7070780c2699524b58a2a5` |
| `crates/job-queue/src/owner_beta/paper/source_evidence.rs` | `3533a9e0dc2f7f77221b30a8b68f5ca7472d8297bfc4c765339838e7042e3d05` |
| `crates/job-queue/src/bin/owner-beta-runner/paper_evidence.rs` | `cbd6a5daf2d113dd33ceed037e69eafe99ab4ebf532bfe95f08e098df9fdb729` |
| `crates/job-queue/tests/owner_beta_paper_source_evidence.rs` | `ad38050dce69a0f35c6c1ec046cd4358cb25ca1400597058612ccddadecc609e` |
| `crates/job-queue/tests/fixtures/owner_beta_paper_source_evidence/valid_chain.json` | `e134aad68a1bf87b5e3ed302f371b61c63ac4ca399d9278ce0b357bbfbb5daa5` |
| `crates/job-queue/tests/fixtures/owner_beta_paper_source_evidence/rejections.json` | `9859eb5a7fa627a5a19a0925c9491d7d10100f8be1b5d51ea8a99d389f48f718` |
| `docs/contracts/owner-beta-paper-source-evidence-v1.schema.json` | `74b89e42339b08b8ef0c9487a1a3db07bbd9749fbbdf7fdfa34655c6795b54b3` |
| `migrations/0066_owner_beta_paper_evidence_v1.up.sql` | `d87c36b9fc25ef47f8d296a4413f2745ee039be3d51762d5b15526c49c4d77bc` |
| `migrations/0066_owner_beta_paper_evidence_v1.down.sql` | `6cabb5151b678d2be4cfe2cd1db19e4617ee6f0e6b1afd1a07df1705cb39133a` |
| `scripts/ops/kis-daily-production.sh` | `1ff0906353b3d84bfdbdd08500484f8f8cc9a052f3a2f9591acbb9b3cbe0c756` |
| `scripts/ops/backfill-production.sh` | `9307d284e73bed5e5e7fbc71b91aea7f2d28eac498b41c03c9e4d84deed9778d` |
| `scripts/ops/lib/backfill-progress.py` | `1388c36c55189b143b38a2c46703dfb2e10518254f838297f801721deb6c9c6e` |
| `scripts/ops/lib/owner-beta-paper-evidence.py` | `3d19365bf4fd8cbb0df2ed889b55a1f18fbc0124a05f00a22f950f4667738075` |
| `scripts/ops/lib/owner-beta-daily-runtime.py` | `e20f62812a5183d956e4de8370f8e55f0d17ce1c0ad51a76f4be1484ffe2ae88` |
| `scripts/ops/install-kis-daily.sh` | `3258e6d24239e0ee37abf8f71a48018fdb30db45282a33117cd5edb7d11d9529` |
| `scripts/ops/test_owner_beta_paper_evidence.py` | `79b2ae173d7ca8ca10046ae35eccbfebe818f0c4b9f6eb625db90d968c7c83af` |
| `scripts/ops/owner-beta-paper-evidence-local-integration.py` | `1538b1cc244153da69a31911dc4d152d515c83fec99ac5e310cf2cd68eae6ef4` |
| `tests/integration/migration-contract/tests/migration_contract.rs` | `c2eba19e21b0e221e494cbac17d5436e2692e3b9f331c3e917f062408d2dc4f4` |

초기 보고와 명령별 로그는 `target/r5-paper/WP-2A2/`에 보존했다. 후속 소스 pin과 검사 결과는
별도 기록으로 연결하며 초기 후보의 미실행·차단 근거를 덮어쓰지 않는다.

## 보완 회수와 최종 source 검사 — 2026-10-29

후속 callback `498fefce-251e-45a2-865b-ac1454a9d53d`와 actual idle/active turn 없음/pending0,
`completion-followup/report.md`를 회수했다. 첫 후보 대비 정확히 네 파일만 바뀌었으며 전체20개 pin과
현재 bytes, 초기 보고/로그·고정 계약/gap·collector3·Cargo.lock의 보존을 확인했다.

production Python Scheduler/Journal/relay/helper 메서드가 test-only I/O 주입으로 실제 생성한20event/8container
chain을 Rust가 재검증하는 회귀가 추가됐다. 이는 SYNTHETIC이며 실제 wrapper process·root FS·Rust DB helper·
커널/Docker transport의 성공 증거가 아니다. PG 양성/오류 join fixture와8개 opt-in harness lane 코드가 추가됐다.
기존 역할의 실제 SQL terminal 관계를 검사하는 코드지만 PG에서 실행하지 않았다. future-slot PG bridge도
provider/FS/child/Compose/Docker/helper transport가 synthetic이며 production end-to-end 검증을 대신하지 않는다.

coordinator 검토에서 clock-step lane이 generic `Rejected`를 CANCEL_ON_SET 성공으로 처리할 수 있음을 확인했다.
격리 harness만 수정해 실제 `os.read`의 `errno=ECANCELED`만 통과시켰다. EAGAIN/EBADF/EIO와 정상 expiry는
거절하는 회귀를 추가했다. 운영 scheduler/reader/schema/SQL·시간 guard·provider 계약은 변경하지 않았다.

수정 후 coordinator가 **Python28 PASS, Rust7 PASS(ignored/filtered0)**를 실행했고20개 source pin의
실행 전후 일치를 확인했다. Rust는 기존 shared HeavySlot/Cargo2로 named job-queue test만 실행했다.
worker의 migration-contract no-run compile exit0·strict job-queue Clippy·fmt 근거를 회수했다.
해당 Rust/SQL 입력은 그대로이며 no-run의 DB 실행 수는0이다. 변경 없는 broad shell/static 검사와 heavy
Clippy를 중복 실행하지 않았다. 기존 Daily operations34 setup 실패/body0는 BLOCKED 그대로다.
개별 명령/결과와 최종20개 pin은 `target/r5-paper/coordinator/wp2a2-final-verification/receipt.json`에 있다.

| 최종 보완 파일 | SHA-256 |
| --- | --- |
| `scripts/ops/test_owner_beta_paper_evidence.py` | `176ea71880098722aaf20281cc16f823f14538b82a3c9d8e5ea6f3395a958c2a` |
| `scripts/ops/owner-beta-paper-evidence-local-integration.py` | `1b642279933aaf6c7fce95317fa117137b9dc016685ec3106ba14ab0e07dd210` |
| `crates/job-queue/tests/owner_beta_paper_source_evidence.rs` | `ac58d52aebe2ad293b2d0e05641c5673d0f92fae44a81acad3cbeebc7d856b2b` |
| `tests/integration/migration-contract/tests/migration_contract.rs` | `baaf2ccd7282186e5a12baa9e6475c2642d6f965f74c5a33cd4afeb002801670` |

**인수 범위:** SOURCE/NAMED_FOCUSED_VERIFIED. **G2a 전체는 OPEN**이다.
실제 PG/RLS·보호 FS·pinned Rust helper/sealed Raw/base·timerfd/FD/systemd/Docker/clock/suspend/crash 및
운영 pre/post3거래일을 확인하지 않았다. 마지막 PG bridge가 진짜 Rust helper transport까지 검사하지 않는
한계도 보존한다. WP-2B는 G2a 인수 전 미착수다.

현재 coordinator 문맥은 UID1000이며 지정된 `/run/lagrange-disposable-paper-vm.json`과 private VM root가 없고
`psql`도 없다. qemu/virsh 바이너리 존재를 VM 대상/권한으로 해석하지 않았다. 현재 위임은 실제 VM/DB/Docker/
서비스/시계/provider 실행을 제외하므로, 이미 승인된 disposable Linux VM 또는 hosted 대상·접근 경로 확인을
요청했다. VM marker/PG cluster/binary/image pin을 out of band로 준비한 뒤8lane과 남은 실제 helper 연결을
검증해야 한다. 현재 개발/운영 host를 fixture로 전용하지 않는다.

두 `.puml`의 R5 source 구조·실제 file:line 근거를 갱신했고 새 화살표21개의 근거 경로/행 존재를 확인했다.
이번 범위의 Docker 실행 제한 때문에 PNG는 렌더하지 않았다. PNG와 실제 source topology 렌더 검사는
G3/병합 전 필수 미완료로 남으며 기존 PNG를 새 topology 검증 근거로 제시하지 않는다.

## 로컬 PG 실제 검증 회수 — 2026-10-29

Owner의 로컬 검증 방향에 따라 WP-2A2의 PostgreSQL component만 독립 실행했다.
완료 callback `41fe6ec2-ad56-43dc-aa73-fae8987df396` 뒤 동일 작업자의 actual idle/active turn 없음/pending0,
[최종 보고](../../target/r5-paper/WP-2A2/local-pg/report.md)와 개별 명령/로그를 회수했다.
앞 절의 PG 미실행은 당시 이력으로 보존하며, 아래 네 named test의 실제 실행으로 이 부분만 해소했다.

대상은 기준 revision `a30edacff987d55b5be7cca25bed7faeec589629` 위의 미커밋 후보로,
각 receipt의 source20개 pin과 현재 bytes가 일치한다. frozen contract/gap·collector3·Cargo.lock의6개 pin과
이전 private 증거49개도 coordinator가 해시를 다시 계산해 일치를 확인했다. 제품/SQL/Rust 소스 수정은 없다.

| migration-contract의 정확한 named test | 실제 passed/failed/ignored | 확인한 범위 |
| --- | --- | --- |
| `paper_0066_catalog_roles_and_preserving_down` | 1/0/0 | 0066 적용·FORCE RLS/owner·함수 grant·직접 DML/역할 오용 거부·이력 보존 down |
| `paper_0066_replay_current_authority_and_concurrency` | 1/0/0 | 동시 reserve/replay·누락 terminal·현재 권한/Owner/epoch/cohort 거부 |
| `paper_0066_positive_terminal_and_negative_joins` | 1/0/0 | 실제 admission/reservation/job/attempt/claim/F/W와 link/readback의 정상 연결·잘못된 join/권리/replay 거부 |
| `paper_0066_wrong_persisted_attempt_is_rejected` | 1/0/0 | FK는 유효하지만 다른 job의 persisted attempt인 경우 link/read 거부 |

각각 `cargo +1.97.1 test -p migration-contract --test migration_contract <name> --locked -- --exact --nocapture --test-threads=1`을
기존 `run_child_with_heavy_slot`로 순차 실행했다. Cargo jobs2, 각73 filtered, 총4 PASS·0 FAIL/IGNORED이며
no-run/skip/0-test 성공이 아니다. 변경 없는 Python28/Rust7·Clippy/fmt 등 기존 검사는 재실행하지 않았다.
전체 hosted required/전체 migration suite도 실행하지 않았다.

실제 PG는 `18.4 (Debian 18.4-1.pgdg13+1)`, cluster ID `7702100129247383596`이었다.
cached image `postgres@sha256:3a82e1f56c8f0f5616a11103ac3d47e632c3938698946a7ad26da0df1334744a`만 사용했다.
단일 UUID project `lagrange-r5-pg-7d3bcfbb-10ac-4c09-9853-3caa5691c657`의 tmpfs512MiB·메모리768MiB·CPU1,
internal network·host bind/volume 없음과 정확한 CID/image/labels를 확인했다.
Docker가 internal network의 요청 포트를 게시하지 않아 private relay가 실제 `127.0.0.1:55439`에 bind하고
해당 단일 CID의 내부 IP:5432로만 연결했다. 다른 network/container·host route/firewall은 추가하지 않았다.
첫 tmpfs0700의 소유권 오류는 실패 자원을 정리한 뒤 UID/GID999로 맞췄다. 첫 relay의 진단 SQL 오류도
종료/부재를 확인한 뒤 고쳤다. 초기 실패·첫 cleanup·포트 관측은 private 원본 증거에 보존했다.

실제 test CID는 `c52d7dd20ceeb8631805ff96b3810773db041f4e00899716c4d6d7b458eca76f`,
network ID는 `a559ee418dcfdc58c3bf7f81f9a3c5250020e52b9e60de5e64e55046f9208ec9`였다.
검증 종료 후 exact project cleanup 성공, 소유 container/network0·relay PID3439426/3441831 부재·포트 종료를
coordinator가 독립 확인했다. fixture DB 정리·tmpfs 제거와 기존 container ID22개 보존도 확인했으며
그22개 서비스의 health나 자원 peak를 인수한 것은 아니다. 소유 pending/실행 자원은 남지 않았다.
명령·로그·해시·cleanup의 대조 결과는
`target/r5-paper/coordinator/wp2a2-local-pg-verification/receipt.json`에 보존했다.

**수용 범위는 ACTUAL_LOCAL_PG_NAMED_FOCUSED_VERIFIED, G2a 전체는 OPEN이다.**
source slot은 migration-owner가 scratch DB에 넣은 UNKNOWN/SYNTHETIC fixture이며 기존 expected calculator에
고정16:30 시간을 공급했다. 실제 새 slot의16:30..16:31 예약이나 root scheduler origin은 증명하지 않는다.
실제 pinned Rust helper/UID·sealed Raw·보호 FS·timerfd/FD·witness Docker·clock/suspend/systemd/crash와
운영 전후3거래일은 미검증이다. disposable PG Docker 실행은 source-witness Docker lane 증거가 아니다.
남은 G2a runtime 검증은 격리 조건과 소유 자원을 명시한 후속 범위로 구체화해야 하며 WP-2B는 미착수다.
이번 로컬 PG 검증·cleanup의 추가 분해나 미해결은 없다. PNG/G3 미완료와 이전 실패 기록도 유지한다.


## 야간 harness 정리 보정 — 2026-10-30

coordinator가 VM 전용 clock-cancel harness의 `finally`에서 시계 복원 오류가 발생하면
`timer.close()`를 건너뛰는 경로를 확인했다. 해당 두 파일만 수정해 복원 시도와 timer 정리를
중첩 `finally`로 분리했다. 복원 오류는 그대로 실패로 전파하며 PASS로 바꾸지 않는다.
새 회귀는 모든 clock/timer I/O를 mock하고 기존 코드에서 close 호출0으로 실제 실패한 뒤,
보정 후 전체 Python29개가 통과했다. 실제 host/VM 시계·서비스·FD lane을 실행하지 않았다.
기존 ECANCELED만 취소 성공으로 인정하는 회귀와 VM marker/변경 opt-in은 그대로 통과했다.
운영 scheduler·wire·SQL·권리/예산은 변경하지 않았다. Rust 입력은 변경하지 않아 이 보정의
Rust/PG 검사는 재실행하지 않았고 G2a 전체 OPEN을 유지한다.

| 보정 입력 | 현재 SHA256 |
| --- | --- |
| `scripts/ops/owner-beta-paper-evidence-local-integration.py` | `80bfa82b445a99fe2956097acaa12cb00c40fb0072db1d6e9f17489ac80213a2` |
| `scripts/ops/test_owner_beta_paper_evidence.py` | `f5989b5a86499fce87e6cffba059c8220e1ea5946b7946b99bbc72ac3d255d77` |

실패/성공 로그와 현재 입력은 `target/r5-paper/coordinator/overnight-harness-cleanup/receipt.json` 및 같은 디렉터리의 red/green 기록에 보존했다. 이전 source/PG 검증 시점의 pin 표와 실패는 당시 이력으로 유지한다.
