# R5 WP-2B-S 소스 후보·집중 검증 회수 — 2026-10-30

상태: **current producer·보호 reader·checker SOURCE/집중 검증 수용(최종46핀), G2a 전체 OPEN**. 아래 초기 후보의 검토 대기·옛 pin은 당시 이력이다.
대상은 `a30edacff987d55b5be7cca25bed7faeec589629` 위의 아래 미커밋 입력이다.
현재 야간 실행·정적 payload·실제 component UNKNOWN과 증거 보존은 [야간 진행 기록](2026-10-30-r5-overnight.md)을 따른다.

Luna/max의 최초 callback98fdfd39와 receipt 보완 callback a7bd6f3e 뒤 각각 actual idle/active turn 없음/
pending0·보고를 회수했다. 최초 결과에는 실행 전후 source pin이 없어 소스를 동결한 뒤 동일 owner가
private receipt만 보완했다. 소스 편집 없이31개 입력의 before/after/current SHA256이 일치했다.
Cargo.lock·wire schema·SQL0066/0060·실제 Daily/calendar 공개 입력을 포함하며 generic cached 결과를
재사용해 실제 검증으로 바꾼 것이 아니다.

| 명령 (모두 `cargo +1.97.1`) | 결과 |
| --- | --- |
| `test --package job-queue --locked --lib owner_beta::paper::evidence::tests` | 4 passed / 0 failed / 0 ignored / 647 filtered |
| `test --package job-queue --locked --test owner_beta_paper_admission` | 4 passed / 0 failed / 0 ignored / 0 filtered |
| `clippy --package job-queue --locked --all-targets --all-features -- -D warnings` | exit0 |
| `fmt --all -- --check` | exit0 |

실제 반복은2026-10-29 16:35:57~16:35:58UTC의 `attempt-20261029T163557Z`다.
세 Cargo child는 기존 `run_child_with_heavy_slot`·Cargo jobs2로 순차 실행했고 exit/reap까지 보존했다.
첫 sandbox lock 접근 오류는 Cargo 시작 전 실패이며 동일 명령의 정상 escalation으로 실행했다.
원래 lock/source/permission mode를 바꾸지 않았고 최초 실패·이전 모든 실패/통과 로그를 보존했다.

| 동결 후보 | SHA256 |
| --- | --- |
| `crates/job-queue/src/owner_beta/paper/contract.rs` | `e4e40c785bda5a8803bd99a96c66d72ea36f0967474674c1c75747da3f387139` |
| `crates/job-queue/src/owner_beta/paper/input.rs` | `25c78089722c12760f7e3882b1ff1eb63b3eb1ed30bdb097e3eea0c3fed14832` |
| `crates/job-queue/src/owner_beta/paper/evidence.rs` | `a5d366f20859914d1a3b6c8ef73410a89e590393e825faf6aa509bf2bdb61d0a` |
| `crates/job-queue/src/owner_beta/paper/mod.rs` | `61e364ad5ee6a66601e4786e9d0eafbcff80893a9ec889265fa1a4e748258439` |
| `crates/job-queue/tests/owner_beta_paper_admission.rs` | `1ac9e692ea7871e13ba064883325d7e5a6bcbcc3bfe0374d3a8aac373c4a970b` |
| `crates/job-queue/tests/fixtures/owner_beta_paper/valid_binding.json` | `376dd89d3b2702d15f08d47cd409ec3659775b47c04b7d2754415923aa04efee` |

coordinator가 각 현재 파일 hash, 실제 로그의 passed/failed/ignored 수, argv와 종료값을 대조했다.
원본 보고·추가 receipt는 `target/r5-paper/WP-2B-S/receipt-completion/`, 대조 결과는
`target/r5-paper/coordinator/wp2bs-receipt-verification.json`에 있다.

검사는 SOURCE/SYNTHETIC이며 public integration4는 decode/음성 경계만, 양성 unit4는 private evaluator다.
1453행 checker의 실제 보호 source/Raw/Daily/SQL/calendar/권리·TTL 의미가 올바른지는 별도 Sol/xhigh
read-only 검토 중이다. 기존 PG4는 WP-2A2 SQL component 근거이며 새 checker의 실제 PG/root 양성
실행으로 부르지 않는다. runner/API/activation 연결과 실제 source-origin·커널/VM은 미실행이다.
검토 findings를 해소하기 전 source 인수·WP-3/운영 gate를 통과시키지 않는다.


## 독립 검토 findings 인수 — 2026-10-30

Sol/xhigh callback b1f8fa69 뒤 actual idle/pending0·보고와 동결6개 pin 유지 확인을 회수했다.
coordinator가 실제 checker/기존 SQL/Raw/calendar/producer binding 코드를 대조해 F1~F5를 인수했다.
**초기 후보 소스 인수를 보류**하며 기존8개 synthetic 검사 통과가 아래 경로를 보증하지 못함을 보존한다.

| ID | 우선순위 | 확인된 문제 / 필요한 보완 |
| --- | --- | --- |
| F1 | P1 | 두 checker transaction의 READ ONLY가 기존 authority reader의 영속 control/user/rights FOR SHARE와 충돌한다. reader 잠금/권한은 유지하고 transaction 모드만 맞춘다. 같은 production SQL bytes를 쓰는 실제 PG 회귀를 준비한다. |
| F2 | P1 | 현재 witness binding/calendar/manifest·semantic 설정의 서버 고정 권위를 확인하지 않고 caller와 과거 cohort끼리만 비교한다. 현재 root 원본 binding은 임의 operator 경로의 root:root0600이며 app reader의 고정 current selector 공개 seam이 없다. caller가 고른 옛 root 파일을 현재로 인정하지 않는다. |
| F3 | P2 | 달력 discovery가 다른 entitlement의 보존 batch까지 선택해 현재 권리의 정상 증거를 거절한다. 기존 matches_entitlement selector를 적용하되 선택된 원본/보호/hash 검증은 유지한다. |
| F4 | P2 | 보존된 가장 오래된 달력까지 증명 범위를 확장한다. verified 최신3거래일로 suffix를 정하고 그 구간의 civil-day 완전성을 검증해야 한다. |
| F5 | P2 | 음수 wall age/역행 및 monotonic180초 초과를 거절하지 않는다. private trusted clock과 현재 cutoff를 전후 확인한다. |

F1의 backend 전제는 [PostgreSQL18 executor의 read-only 검사](https://raw.githubusercontent.com/postgres/postgres/REL_18_STABLE/src/backend/executor/execMain.c)와
기존 migration0060 authority의 FOR SHARE를 함께 확인했다. 실제 PG 오류25006을 재현한 것은 아니다.
Q1 admission-link hash/action batch·range 비교는 확정 public 우회와 구분해 검토 질문으로 남겼다.
원본217행 검토/trace/pin은 `target/r5-paper/WP-2B-REVIEW/`에 보존한다.


## FIX-A 집중 검증 회수와 재검토 — 2026-10-30

실제 idle/pending0·callback f389cc02 뒤40개 입력 before/after/current hash와 최종 로그/명령을 직접 확인했다. unit8/admission4·strict Clippy/fmt PASS, migration named 회귀는 no-run compile만이다. 기존 후보 중 evidence.rs만52a68264…af1a81로 바뀌었고 shared read_transaction.sql16e96578…a03b3 및 migration 회귀74837ec3…21cfc를 추가했다. 이전 후보 pin은 보존하고 새8개를 `target/r5-paper/coordinator/wp2bs-fix-a-candidate.json`에 동결했다. F4의 verified projection 범위 한정 증명을 포함해 Sol의 독립 검토를 다시 진행하고 실제 로컬 PG5는 별도 private 실행 범위로 시작했다. F2는 [current-binding 계약](../contracts/owner-beta-paper-current-binding-v1.md)의 producer/consumer 보완 전까지 blocking이다. SOURCE 인수/G2a 전체는 아직 OPEN이다.


## FIX-A 독립 재검토 회수 — 2026-10-30

callback15698100/actual idle/pending0·보고와 후보8개/보호28개의 시작/종료/현재 hash를 대조했다. F1/F3/F4/F5 및 Q1 소스 수리는 의미상 수용 가능하고 새 correctness finding은 없다. 실제 PG 검증은 별도 진행하며 F2 blocking·전체 후보/G2a 미인수를 유지한다. Projection-level mixed policy/budget/closed end, clock 경계와 action 범위의 추가 회귀 공백은 후속 보강 대상으로 보존했다. 근거는 `target/r5-paper/coordinator/wp2bs-fix-a-review-verification.json`과 원 reviewer 보고다.


## FIX-A 실제 로컬 PG5 회수 — 2026-10-30

actual idle/pending0·callback8cb4a915/보고를 회수하고 coordinator가42개 입력/동결8개 before/after/current hash, 실제 Cargo 로그5개와 argv/exit0·각1 passed/0 failed/0 ignored를 대조했다. 새 `paper_0066_checker_read_transaction_contract`는 READ ONLY의 실제25006 baseline을 assert한 뒤 같은 production SQL bytes의 최초/현재 reader 두 경로에서 SUCCEEDED를 확인했다. 기존0066 named4개도 실제 실행됐다. PG18.4의 system identifier7702147506279850028이 유지됐으며 768MiB/CPU1/pids256/tmpfs/internalnet·고정image를 확인했다.

초기 private carrier의 tmpfs inspect 가정 오류와 없는 migration table 진단 SQL 오류는 테스트 전 종료/정리 후 보존했다. 실제5명령이 모두 성공한 최종 run도 진행출력의 정규식 오분류로 원본 receipt2개/aggregate는 FAIL을 유지한다. 이를 덮지 않고 독립 reconciliation과 coordinator 원본 로그 대조로 actualPG5 PASS를 수용했다. 불필요한 테스트 재실행은 하지 않았다. 초기/최종3개 UUID project의 container/network 부재, relay PID3806873 회수/부재, loopback55439 종료 및 기존 container ID 보존도 다시 확인했다. 근거: `target/r5-paper/coordinator/wp2bs-local-pg-verification.json`.

이 증거는 synthetic root fixture를 쓰는 실제 PostgreSQL SQL0060/0066 component다. 전체 Rust checker/실제 root source/Raw/현재 binding·G2a를 증명하지 않는다. FIX-A 소스 수리/실제 PG만 수용하며 F2가 남아 전체 후보는 미인수다.

## F2 호환성 선택과 독립 reader 분기 — 2026-10-30

기존 manifest root600/넓은 release의 nested `.env` 코드를 직접 대조해 compatibility option2를 선택했다. root가 원 private 자산을 기존 보호 validator로 매 refresh 검증한 현재 capsule을 발행하고, worker의 fixed config/policy·독립 committed Raw/verified session calendar/현재 SQL 권리 검사는 유지한다. 새 mount는 current directory 하나로 한정하는 조정안이며 원600/700/rights/Owner 기준을 바꾸지 않는다. source producer가 실행 중이므로 원 formal ABI를 아직 변경하거나 steer하지 않았다. 유휴 후 적용할 수정문/patch는 coordinator private에 준비했다.

Producer가 공유 canonical fixture5개(유효4·변조19)를 동결했고 coordinator가 hash를 확인했다. FIX-A 검토/PG 회수 뒤 동일 Luna는 unchanged10-key envelope·fixed-path protected current reader만 source_evidence.rs에 구현한다. Checker/manifest projection 연결은 범위 밖이며 old source event schema/literal/검증 의미는 보존한다. Python producer와 Rust reader는 겹치지 않는다. 실제 root/OFD/RO mount·전체G2a는 별도다.


## Current reader 후보 회수·추가 시계 대조 — 2026-10-30

callback7fb53eb2/actual idle/pending0에서 new7·기존 sourcewire7·Clippy/fmt와57개 실행 입력 전후 일치·현재 정적55개 pin을 확인했다. 새 상수/section만 제거한 bytes가 원 source_evidence.rs SHA3533a9e0…e3d05와 정확히 같아 원 코드/원 SCHEMA_JSON 보존도 독립 확인했다. 소스 후보3549093b…7c0dc와 ABI/shared fixture를 동결해 Sol 검토 중이며 아직 인수하지 않는다.

coordinator는 첫 snapshot의 validate_current_timing이 각 age<180초와 read-before→after 일관성만 검사하는 경로를 추가 대조했다. root 관측 이후 첫 read 전에 clock step이 완료되면 wall40초/boot10초/mono10초 age가 모두 범위 안이고 read 중 clock은 정상일 수 있다. 관측→현재의30초 divergence를 직접 거절하는 관계가 빠져 보이는 정적 반례를 별도 기록했다. 이는 실제 보호 경로/ELIGIBLE 우회 실행 결과가 아니며 independent review와 합쳐 owner 회귀/보정을 수행할 항목이다. 기록: `target/r5-paper/coordinator/current-reader-clock-finding.json`.


## Current reader 독립 finding 회수와 formal ABI 보완 — 2026-10-30

Sol callbacke9b58000과 actual idle/pending0·candidate9/protected39 시작/종료/당시현재 pin을 대조했다. CR1/P2는 coordinator의 별도 정적 반례와 같은 누락으로 확인됐다. Root publication 뒤 reader 시작 전 wallstep/suspend가 완료된 경우 관측→현재의 clock delta 관계를 검사하지 않아5/1/1초 또는5/5/1초 age가 통과할 수 있다. 실제 root/ELIGIBLE 재현은 아니며 source 인수는 회귀/수리까지 보류한다. Q1 마지막 bytes read 후 fence/name 재확인은 확정 finding과 분리한 방어 보완으로 선택했다.

모든 관련 worker actualidle/pending0 확인 뒤 current contract amendment1을 적용했다. 문서 hash는 c3cc4d4e…291a54에서 b9bf8242…b481a로 바뀌었고 원 문서를 private에 보존했다. ten-key schema11a04725…20e64/shared fixture5 및 원 Owner/G1·권리/gate는 그대로다. 현재 capsule의 root guard가 원 manifest600/env/calendar/Daily 자산을 관측하고 worker의 독립 config/Raw/verified Calendar/SQL 검증은 유지한다. 전체 release/calendar mount와 redundant private 원본 reread만 제거한다. 의미상 layout 회귀8개는 semantic+outer hash가 모두 맞도록 준비했고 원 판정기 실행 성공으로 세지 않았다.

새 notified 두 작업: Astra 같은 Python4파일의 amendment1 보완(45분), Luna current-only reader/기존 checker+필요 public test의 CR1 RED→GREEN·Q1 마지막 fence·host경로/고정alias 수정과 전체 current checkpoint 연결(90분). 쓰기는 겹치지 않으며 실제 root/DB/Docker/서비스/clock/VM/production 실행은 범위 밖이다. 새 source 후보/검토와 G2a 실제 인수는 아직 미완료다.


## CR1/checker current 통합 후보 회수 — 2026-10-30

callbackc930295e/actualidle/pending0에서 source unit10+10·integration7+4·strict Clippy/fmt를 원 로그/argv/exit0로 대조했다. 고정 Rust/입력 전후/현재 pin과 원 source prefix3533a9e0…e3d05 exact보존도 확인했다. 새 reader0aaaa931…f854b1/checkerf7dd9cce…bc9dca는 publication 관측→before/after ages의1초 divergence를 직접 검사하고, 마지막 bytes/clock 뒤 named path·metadata·OFD를 다시 확인한다. Current capability를 checker 진입에서 열어 caller fullHOSTbinding과 비교하고 DB/Raw/Daily 읽기 전후와 최종결과 직전에 재검증한다. Host selector와고정config/policy/Raw alias를 동일경로로 요구하던 접점도 보정했다.

CR1 회귀는 old age-only 판정 모델의 수용과 수정 production helper의 거절을 비교했다. 역사 Rust binary에서 실제 RED failure를 실행한 것과 구분한다. source guard에대한 회귀이며 actualroot/clock mutation/PG 통합 증거는 아니다. 확인 기록은 `target/r5-paper/coordinator/current-integration-verification.json`이다.

Python83·layout8/48과 합쳐45개 source/static 입력을 `current-combined-candidate.json`에 동결했다. Sol의 fullpublisher lifecycle·guard/FD/mount/clock·currentreader/checker 연결 통합 독립 검토를 새 notified turn으로 시작했으며 결과 전까지 source전체/F2/G2a 인수는 보류한다.


### CC1 자식 초기화 실패 — 2026-10-30

통합 독립 검토에서 P2 CC1을 확인해 combined SOURCE 수용을 보류했다. Popen 성공 뒤 pidfd 소유권 초기화가 실패하면 자식 종료·reap가 누락된다. coordinator가 일반 UID의 전용 자식으로 ENOSYS를 한 번 주입해 살아 있는 자식을 실제 확인했다. 외부의 정확한 Popen 핸들로 종료·reap했고 /proc 부재와 FD4→4를 확인했다. 이 재현은 실제 CURRENT 철회나 root origin 증거가 아니다.

45개 후보 핀은 일치하며 CR1·호환성·checker 연결에는 추가 finding이 없다. 진행 중인 Astra의 private component 준비를 재지시하지 않고, 실제 idle/pending0·보고 회수 후 같은 source owner에게 witness와 Python 회귀의 한정 수리를 맡긴다. 원 실패·재현·정리 증거는 target/r5-paper/coordinator/cc1-probe/와 독립 검토 보고에 보존했다. G2a 전체 OPEN, Paper 활성화 미인수와08:30 종료를 유지한다.


### 2026-10-30 current/checker SOURCE 수용

기준 a30edacff987d55b5be7cca25bed7faeec589629의 미커밋 후보46개 입력을 고정했고, CC1 최종 독립 검토 callback2f6c7c72의 actual idle/pending0·46핀·24개 신뢰 정의 보존과 새 finding 없음을 확인했다. WP-2A2 current producer/보호 reader와 WP-2B checker의 SOURCE/집중 검증 단계를 수용한다. [최종 소스 수용 영수증](/data/worktrees/3puw275b/modern-anaconda/target/r5-paper/coordinator/current-source-acceptance.json)은 후보 manifest hash와 모든46개 입력을 묶는다.

Python98 직접PASS·ordinaryUID 자식 rollback3개·이전 Rust current31/Clippy/fmt·실제 PG FIX-A5 근거를 연결한다. 새 publicreader ignored 테스트는 원본과 같은 bytes로 no-run/Clippy/fmt를 통과했고 실행0이다. private carrier PC1~3는 V2의 red4/green29와 독립21핀 SOURCE 수리 수용이다. 과거 실패와 V1/기존45핀은 보존한다.

이 결과는 실제 protected root/current/OFD/public reader·RO bind 교체·full assetguard·pinned helper/Raw/DB·scheduler origin/clock/suspend/systemd·연속3거래일·Owner 운용/활성화 인수가 아니다. G2a와 R5-02 전체는 OPEN/DOING이다. 이미지 환경 probe는 container생성0에서 optionalVolumes Go-map 조회 오류로 차단됐고, coordinator의 실제 동일 query에서 원인을 재현·보정했다. 원 UUID/누적 create1회 범위를 재사용하는 notified 후속을 시작했다. Python/libc 실제 정보는 아직 미회수다.
