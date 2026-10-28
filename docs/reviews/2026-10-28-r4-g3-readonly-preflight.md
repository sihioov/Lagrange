# R4-06 실제 입력 읽기 전용 사전 점검: 2026-10-28

검증 후보는 `a006e5ee683b4e36bdb7f89924a01def50e240d5`다. Owner가 실제 승인 입력과 운영 설정의 읽기 전용 사전 점검을 허용하고, workspace의 `target/r4-expansion-implementation/g3-readonly-preflight.py`를 `sudo python3`로 직접 실행한 출력을 제공했다. 아래 root 읽기 결과는 Owner 제공 실행 증거이며 coordinator가 root로 재실행한 결과는 아니다. 서비스·DB·운영 파일 변경은 수행하지 않았다.

| 확인 항목 | 결과 |
| --- | --- |
| 체크인 V3 승인 레지스트리 | 고정 SHA-256 `5d3aa2b354d8c0c51d0d7d029e9fd3f92e0570fe074262bfc900829a5a2bb707` 일치 |
| 실제 승인 입력 루트 | `/var/lib/lagrange/data/artifacts/historical-price-beta-root` 존재, mode `0750`; 승인 candidate 디렉터리 mode `0700` |
| `manifest.json` 파일 | 2,189 bytes, SHA-256 `254a9ec22783d5955bbe69034144bb7a82d1278ff7e0d579453981aae0530cc7` 일치 |
| `bars.ndjson` 파일 | 7,672,914 bytes, SHA-256 `23354c54708d7a458aadc4f2cb2fca77469969e91256b6d3b119101ebae98ffe` 일치 |
| 현재 설치 릴리스 | `a6da63c134fa471ff91e28a32c4b6a2ad9c08ba7`; 검증 후보와 다름 |
| 후보 설치 | `/opt/lagrange/releases/a006e5ee683b4e36bdb7f89924a01def50e240d5` 및 해당 config·receipt·manifest 부재 |

사전 점검 스크립트의 `/etc/lagrange/compose.env`와 `/etc/lagrange/release-manifests/<candidate>.manifest`는 실제 설치 파일 위치가 아니었다. 따라서 이 두 경로의 `absent`를 운영 설정 전체의 부재로 해석하지 않는다. `compose-release.sh`의 기본 경로를 확인하고 coordinator가 직접 메타데이터를 읽었다. 현재 릴리스의 `deploy/compose/.env`는 root:root `0600`, 8,446 bytes이고 `.lagrange-release-manifest`는 root:root `0600`, 2,597 bytes다. 둘 다 일반 파일이고 현재 coordinator 사용자에게 읽기 권한이 없다. 내용과 해시는 확인하지 않았다. 스크립트는 이 실제 설치 경로를 검사하도록 수정했다.

파일 바이트 해시 일치는 확인했다. 이 점검은 sealed Rust reader 실행, 파일 소유권·전체 안전 정책, 개별 Owner의 기존 권리와 후보 config 간 바인딩, 실제 DB 상태, 설치·API drain·rollback·백업/복원 또는 Owner 사용 검증을 수행하지 않았다. `rights_legal_approval=not_established_by_this_check`는 이 스크립트의 검증 범위를 뜻하며 기존 승인 철회나 새 법적 동의 요구를 뜻하지 않는다. [기존 실제 입력 확인 기록](2026-10-24-r4-approved-artifact-location.md)과 [동일 SHA G2 성공](2026-10-28-r4-g2-validation.md)은 보존한다.

다음은 기존 승인과 실제 설정의 독립 pin·Owner 바인딩을 회수해 후보 실행/설치 절차에 연결하고, 허용된 실행 범위에서 실제 reader와 운영 복구를 검증하는 것이다. 현재 읽기 전용 사전 점검 승인을 운영 설치·서비스 전환 권한으로 확대하지 않는다. R4-06과 R4는 **DOING**, G3는 **OPEN**이다.

## 후속 실제 입력 실행

Owner의 후속 진행 지시에 따라 운영 상태를 변경하지 않는 격리 실행을 수행했다. 성공한 CI `40639806731` attempt1의 artifact `12797413115`를 회수하고, 아카이브 155,254,608 bytes의 GitHub SHA-256 `d771b52792dc0c2089c0b5e0ff4ef64f1af43988ca4f08217583f8485a59ff2b`, 정확한 네 파일, 두 바이너리 해시·ELF 대상과 provenance의 소스 해시 73개를 후보의 Git blob에 대조했다. 새 컴파일이나 기존 CI 반복 실행은 하지 않았다.

실제 입력은 UID/GID `10001:10001`, 네트워크 `none`, 읽기 전용 root·입력·실행 파일, 전체 capability 제거, no-new-privileges, memory 512 MiB·CPU 1·PID 32 조건의 별도 컨테이너에서 읽었다. DB·provider·publication·공식 daemon은 실행하지 않았다. 실행별 정확한 ID·명령·종료와 결과는 private `target/r4-validation-20261028/g3-candidate-a006e5e/`에 있다.

| 실제 실행 | 확인 결과 |
| --- | --- |
| Sealed V3 reader와 기존 v1 기본 전략 | 2017-06-22~2026-08-28의 2,252 평가 세션, 서로 다른 실행 ID로 두 번 계산, 두 비용 결과 해시 반복 일치, exit0 |
| v2 다섯 기본 전략의 최근 20 세션 | 2026-07-31~2026-08-28, 다섯 전략 각각 두 실행, 각 비용 결과 해시 반복 일치, 열 개 child 모두 exit0 |
| v2 다섯 기본 전략의 장기간 | 2017-08-02~2026-08-28, 2,223 평가 세션, 다섯 전략 각각 두 실행, 각 비용 결과 해시 반복 일치, 열 개 child 모두 exit0 |

기존 v1 illustrative hash는 `sha256:2c0c8636c39c39ba2a11bf2e11f077104ca2d316971878f0d3ea0adf9afbbff5`, zero-cost hash는 `sha256:42178695d11b2098096dcc80e7669686e03a561198addd990610cd7ec4da178a`다. 해당 실행은 13:45:50~13:54:25 UTC에 완료됐다. 최초 coordinator의 `docker start --attach` 관찰만 180초에 timeout됐고 컨테이너는 계속 계산했다. 그 실패를 보존하고 기존 child의 1,200초 한도를 근거로 같은 ID를 회수했다. 계산 재시작이나 추가 v1 실행은 없었으며, 마지막 종료는 exit0/OOM false다. 최근 20 세션 v2 결과도 독립 실행 ID의 canonical 금융 결과를 비교했다. 이는 Owner 권한 검증을 대체하지 않는다.

완료된 실행의 임시 컨테이너는 모두 정확한 ID로 정리했다. 앞뒤 API·PostgreSQL·Owner V2의 ID·이미지·시작 시각·restart·running·OOM 메타데이터가 일치했다. 운영 API는 이미 `unhealthy`이며 기존 이미지에 새 drain 계약이 없다는 관측을 유지한다. 실제 운영 설정·권리 바인딩·공식 startup·설치·운영 복구·Owner 인수는 아직 OPEN이다.

장기간 평가 시작은 고정 달력과 최소 준비 구간의 달력 하한에서 도출해 명시적으로 요청했다. 각 Rust child가 실제 승인 bar에서 다섯 전략의 factor 준비 상태를 다시 계산하고 요청과 바인딩을 검증한 뒤 수용했다. Python 제어기는 요청·격리·해시 비교만 담당하고 금융 결과를 재구현하지 않았다. 생성한 실행 ID는 이 격리 검증의 식별자이며 실제 Owner나 운영 config의 대체 값이 아니다.

| 장기간 전략 | Illustrative canonical SHA-256 | Zero-cost canonical SHA-256 |
| --- | --- | --- |
| Buy & Hold | `e90ebe8be5063eeb597e86824827a7ada1cc2b888c13d86f44d2bb485aca1ccd` | `ebc0e768a56990e5b55fed2922f8724f4a5fe71f8feb75c63506cbd49d5a4462` |
| Trend Following | `6f34d8edf0f7f744888c1a12686531d112a851923e2c2d2be38f477a30b38437` | `fa4d04b6d0f9f61a3426447bca9e0fc76403ee936c56886fe75524d1d3725468` |
| Relative Momentum | `ef706a41cdd6534528e1fea72e8239f1f831fc6ea5636527fd4a8d5fbea90fda` | `f9632bf4b9880c63b8c49700e304b68371bbb64da547a518a616a2123de64bce` |
| Dual Momentum | `a8ea44c64613db1e212d1c7be0e6ac3232b4cac7a1f7e9567cc59549d7f413cf` | `2fbbaef11086a368e8386b244acb6152d9f402d38b696bc146279573a2b7b2c8` |
| Inverse Volatility | `8fd5064b2203b56b3ae768567ec1dacf8926e517a1792af6e42a543f45ac36bc` | `08f6ef1bbd1ce218f93da0692e9e5900e32cbba71971306d52e7b870f3ee61a8` |

같은 SHA의 connection proof artifact `12796754883`도 회수했다. 아카이브 92,438,241 bytes와 GitHub SHA-256 `76458fdb44416ca9bb9fbd5296a5fc47d66dfa114d9f591c4a895809e2902e5f`, 정확한 두 파일, 바이너리·ELF·출처 및 후보 Git blob 921개 소스 해시가 일치했다. 해당 driver는 아직 실행하지 않았다.

기존 API의 공개 상태 경로만 읽었다. `/healthz`는 HTTP200/status ok, `/readyz`는 HTTP503/status not_ready였다. 대기 감사 행은 0, audit worker는 alive=true/stale=false이며 연속 실패와 총 실패가 각각 7이다. 현재 소스의 `next_consecutive_failures`는 비어 있는 poll로 실패를 지우지 않고 실제 성공한 전달 뒤에만 연속 실패를 해소한다. 따라서 단순 카운터 초기화나 상태 표시 완화로 운영 인수를 대신하지 않는다. 원인·실제 전달·복구와 새 이미지의 drain 계약은 별도 운영 증거로 확인해야 한다. 이 조회로 서비스나 DB 상태를 변경하지 않았다.

## 후보 빌드 준비

원래 worktree의 검증된 코드 바이트를 바꾸지 않고 `a006e5e` 전용 로컬 checkout을 준비했다. 공식 이미지 빌드 `--plan` 및 `--preflight`는 exit0이다. 이미지 빌드·설치는 아직 실행하지 않았다. 일반 shell umask 때문에 새 checkout의 simulation entrypoint가 `0775`여서 첫 plan이 거절됐고, 이 파일의 메타데이터만 계약의 `0755`로 맞춰 통과했다. 첫 Compose preflight는 비활성 capture 서비스의 `RESEARCH_ENTITLEMENT_SHA256` 보간 누락으로 실패했고, prebuild 호출에만 `prebuild-disabled`를 주어 통과했다. 공개 예제 env는 빌드 사전 검사 전용이며 운영 설정이나 실제 권리 값으로 쓰지 않는다. 두 최초 실패와 보정 후 결과를 함께 보존한다.

Owner가 요청한 72시간 sudo의 OS 설정은 최초 사전 점검 스크립트에 포함되지 않았다. 이를 정정하는 `target/r4-expansion-implementation/enable-r4-sudo-72h.sh`를 준비했다. root 소유 gateway와 그 한 경로의 sudoers 규칙만 설치하고, 각 호출에서 고정 만료 시각을 확인하며 호출 환경을 제한한다. Bash 구문과 생성 Python의 비-root 거부·만료 거부·상대 명령 거부·상태·유효 실행 다섯 경우는 검증했다. root 설치 전에는 이 권한이 생겼다고 주장하지 않는다.

## 2026-10-29 보호된 운영 준비

Owner가 실행한 72시간 sudo 설치 후 root gateway의 UID0 실행과 2026-10-31 14:23:53 UTC 고정 만료를 직접 확인했다. 이후 진행 지시로 필요한 G3 운영 준비를 계속하며 provider·계좌·주문 권한은 확대하지 않는다. 과거 읽기 전용 관측은 당시 범위로 보존한다.

실제 현행 릴리스는 `a6da63c134fa471ff91e28a32c4b6a2ad9c08ba7`다. 보호된 env SHA-256은 `425c74e2a9a2437a76d4e8fb5b63ab3c237a9aa5cd6eaf750ef28d099f03c2fd`, manifest는 `54bbee6f378f63069b1fd54a946b5db4cc6bed31b69112f174b8eb3bc04e2331`이며 root:root0600이다. 실제 Daily Owner와 DB owner role을 대조했고, `repo://docs/decisions/0005-kis-personal-use-entitlement.md`의 채택된 문서 해시 `5904a9c5ee00af734c762e877227761ab23391acf65220715f90d08b61947ea9`가 현행 Daily config·ACTIVE DB 권리의 bare hash와 일치한다. 기존 권리의 날짜는 2016-08-29~9999-12-31이며 `krx_eod_bars`/`backtest`를 포함한다. env의 오래된 research 해시 및 `/etc`의 오래된 별도 문서는 이 바인딩으로 대체하지 않았다. 새 법적 권리를 생성한 것은 아니다. 자격 증명은 출력·증거 파일·명령 인수에 기록하지 않았다.

실제 DB는 migration62까지 성공이며 simulation table은 없다. Daily control은 OFF/epoch0, 해당 Owner의 running·queued 작업은0이다. admin의 control 조회 거부를 보존하고 별도 migration_owner의 read-only repeatable-read 조회로 확인했다. 새 권한 부여나 migration은 아직 하지 않았다.

API의 누적 감사 실패를 표시만 지우지 않고, 쿠키 없는 OIDC callback의 실제 transaction-missing 거부 요청 한 번을 보냈다. 이 분기는 Auth0 전송 전에 종료된다. 응답403/`OIDC_TRANSACTION_MISSING`, 감사 backlog1→0, 연속 실패7→0과 readiness503→200 및 Docker healthy를 확인했다. 총 실패7은 그대로이며 API를 재시작하지 않았다. 이 보안 감사 한 건은 정상 거부 요청의 실제 결과이며 임의 DB 감사 행을 삽입하지 않았다.

공식 빌드는 `lagrange-r4-a006e5e-build.service`에서 root 보호 checkout, Cargo jobs2·Compose parallel1, Nice10·idle I/O·CPU/IO weight20으로 진행한다. 서비스별 기존 자원·health·OOM gate를 유지한다. 처음 두 이미지 bootstrap/migrate가 완료됐고 API 공통 Rust artifact가 컴파일 중이다. 빌드 완료나 설치 성공은 아직 주장하지 않는다.

기존 Research와 Daily reader는 이미 정지 상태였고 그대로다. 실행 중이던 Owner V2 reader의 첫 TERM drain은 exit1로 gate가 거절했다. 같은 이미지의 명시적 복귀와 healthy를 확인한 뒤 새 세대에서 한 번 더 drain하여 exit0을 관측했다. 첫 exit1 원인은 확인되지 않았으며 실패 영수증은 보존했다. 현재 빌드는 정확한 두 번째 정지 세대와12시간 유효 root attestation에 바인딩돼 있다. 빌드 중 리더를 임의 재시작하지 않으며 종료·회수 후 원래 restart policy와 동작 복귀를 확인해야 한다.

기존 백업 설정의 유지보수 도구는 더 오래된 application commit에 바인딩돼 첫 사전 검사가 실패했다. 현재 릴리스의 Git source만으로 별도 immutable maintenance package를 설치하고 현행 application commit을 바인딩했다. 후보 a006의 simulation-aware 도구도 별도로 보존하며 pre-migration schema62 백업에 혼용하지 않는다. 기존 백업·타이머·설정은 변경하지 않았다. root filesystem의 여유가 기존30GiB 하한보다 작아, 여유가 있는 `/data`의 새 root0700 저장소를 보호된 `/srv/backups/lagrange-r4-validation`에 bind mount했다. 전역 fstab은 바꾸지 않았으며 재부팅 후 경로는 재확인이 필요하다. config `backup-before-upgrade-03.conf`의 `--check`는 exit0/PREFLIGHT다. 실제 encrypted backup·격리 restore는 컴파일 후 순차 실행할 예정이며 아직 NOT RUN이다.

비공개 근거는 `target/r4-validation-20261028/g3-candidate-a006e5e/`의 설정 조회·감사 복구·active operations와 `/etc/lagrange/release-manifests/r4-a006e5e/`의 서로 다른 drain·복귀·백업 검사 영수증이다. 초기 설치는 기존 API 이미지에 새 HTTP-drain-v1 capability가 없다는 점을 별도로 해결해야 하며, 과거 exit0을 fresh handler 완료 증거로 바꾸지 않는다. 오전9시 한도 전에 실제 진행 중 작업과 리더 복귀를 회수한다. R4-06/R4 DOING, G3 OPEN을 유지한다.

초기 전환용 보호된 OFF 설정의 첫 검사는 기존 operational session window가 오늘 날짜에 유효하지 않아 거절됐다. 전환용 새 설정에만 intraday quotes OFF를 명시했고 기존 운영 설정·달력은 바꾸지 않았다. 다음 검사에서 simulation worker의 새 전용 secret copy 누락이 검출돼, 기존 worker 비밀의 동일 바이트를 새 서비스 UID/GID10001·파일0440·디렉터리0750으로 복사한 후 production config 검사가 PASS했다. 기존 다른 secret과 DB 자격 증명은 변경하지 않았다. 이는 실제 서비스 전환이나 새 시세 수신을 허용하는 증거가 아니다.

기존 API 계약 때문에 첫 전환은 admission-disabled OFF bridge와 그 뒤 configured release를 구분하는 조건부 절차로 준비한다. 검증된 `a006e5e` 뒤 문서만 변경한 `eb48ea2`의 기존 CI `40713268701`과 Research smoke `40713287917`을 각각 한 번 dispatch했다. 두 실행은 진행 중이며 아직 수용하지 않았다. 후속 clean root checkout만 준비했으며 두 운영 Rust 빌드를 동시에 시작하지 않았다. OFF bridge는 보호된 simulation config가 있는 호환 rollback 대상의 대체물이 아니므로 실제 이력 보존·rollback 인수는 계속 OPEN이다.

초기 API maintenance 명령은 실행하지 않은 상태로 실제 Compose 확장을 검토했다. 첫 두 검사는 비활성 range 프로필의 `RANGE_RAW_BATCH_ID` 보간 누락으로 거절됐다. 초기 첫 stderr 전체는 보존되지 않아 UNAVAILABLE로 유지한다. 설치 wrapper에 이미 있는 `compose-config-disabled` process-only sentinel과 비어 있는 profile 목록을 사용해 거부 원인을 해소했다. 비교 도구가 image에 baked된 commit을 Compose의 명시 environment에서 찾은 한 번의 KeyError도 보존했다. 이미지의 실제 baked 환경을 포함하고 Compose secret을 read-only bind로 함께 비교한 최종 검사는 PASS다. API의 동일 image/UID/read-only root 및 bind13개를 확인했고 admission disabled·quotes OFF만 전환 예정이며 실제 TERM·교체·current 변경은 하지 않았다.

설정이 없는 OFF bridge로 이력 보존 rollback을 대신하지 않기 위해, 이 운영 기록을 별도 문서 후보로 저장한다. 제품 source는 `a006e5e`와 동일하게 유지하며 해당 configured 후보의 CI·공식 이미지·receipt는 독립적으로 확인한 뒤에만 사용할 수 있다. 현재 진행 중 CI·이미지의 성공을 앞당겨 기록하지 않는다.

## 실제 백업 DB 바인딩 거절과 후속 후보

설정 사전 검사와 별개로, 현재 릴리스용 유지보수 패키지의 실제 read-only DB audit를 호출했다. SQL audit 자체는 성공했지만 `bound_database`는 `DAILY_BACKUP_CONTROL_BINDING`으로 거절했다. 실제 control은 OFF/activation_epoch0이며 승인0·verified frontier0·실행0이다. Owner·policy·trigger는 일치하고 최초 시작일만 DB `2026-10-21`과 보호된 설치 config `2026-10-22`로 다르다. 원 audit digest는 `0024d15cbf424f811e3afdedbd93a3a2a81aeb289a56c46792f52f3b9895e908`이다. 이 read-only 관측으로 DB나 설정을 수정하지 않았다. 정상 Daily backup의 동일 날짜 guard를 우회하지 않고, 두 상태를 보존하는 별도 사전 백업 계약 또는 별도 Daily 최초일 재결정 중 Owner 선택을 요청했다. 답변 전 실제 backup·migration·설치 분기는 보류하고 독립 빌드·검증을 계속한다.

문서 후보 `83941da08bc627a4c4e61e944f375ab024796b92`는 기존 제품 source와 동일함을 확인하고 원래 검증 브랜치에 push했다. CI `40717826954`·smoke `40717832340`은 각각 한 번 dispatch해 앞선 실행 뒤 pending 상태다. 최초 equality pathspec의 shell quoting 실패는 수정 후 empty diff로 확인했다. root checkout의 최초 사용자 worktree clone 실패 원 stderr는 UNAVAILABLE이며, 정확한 common Git 디렉터리의 한정 신뢰로 clean checkout을 준비했다. 전역 Git 신뢰나 wildcard는 사용하지 않았다. 한정 Git 읽기 다섯 건에서 `GIT_OPTIONAL_LOCKS=0` 대신 config 항목을 사용한 절차 이탈을 보존하고 이후 명령은 명시 env로 실행했다.

`eb48ea2` CI에서는 Web의 기본 lane153개 중152 PASS/1 FAIL, market-stream14 PASS와 정리 완료를 확인했다. 실패는 첫 viewport의 server-seeded chart 요청에서 `response.ok()`가 false인 assertion이며 실제 HTTP 상태·원인은 UNAVAILABLE다. root cause를 flaky로 단정하지 않았다. 플러그인에서 전체 원 job log187,120 bytes·SHA-256 `d28eaf4d9c2ffda26869f7c540354059ed5614a73ea835398030604bf7c7e383`를 회수했다. CLI 로그 읽기는 진행 중 run이라 거절됐고, rerun은 플러그인 integration403 및 CLI run-in-progress403으로 미시작이다. run 종료 후 한정 Web retry 한 번을 검토하며 전체 CI를 성공으로 수용하지 않는다.

## 공식 이미지 완료와 기존 리더 복귀

`a006e5e` 공식 빌드는13개 서비스 모두 성공했고 systemd의 MainPID0·SubState exited·Result success·ExecMainStatus0을 회수했다. V3 manifest SHA-256은 `ee3fcc919d1ee2964e7b4e643ea3f52b70cbb01e0f9c074b4522ca8395a668ab`다.13개 실제 로컬 image ID와 OCI revision을 대조했고 API의 pinned image에는 `org.lagrange.api.http-drain-contract=v1`이 있다. D8 receipt SHA-256 `9ec370b6ed0282ec89c0819176da966109357d30b3df520fc2a99605b2f544b7`, 실행 파일 SHA-256 `058cd2156f3e47e01c51cff1accc6bc287ad9f0c2bfb96eb71a0e02242b28468`와 COMPLETE가 일치했다. 첫 회수 glob은 producer·bundle의 동일 receipt 사본 두 개를 하나로 가정해 실패했으며, 실제 canonical producer 경로와 bundle byte 일치를 확인해 회수했다. 새 빌드나 원본 덮어쓰기는 하지 않았다.

빌드 종료 및 Cargo/rustc 부재를 확인한 뒤, 기존 Owner V2의 정확한 container ID와 기존 image·Created·정지 세대를 대조해 명시적으로 시작했다. 시작 응답은 exit0/같은 ID였고 2026-10-28 15:53:35 UTC에 running/healthy·OOM false·기존 restart policy unless-stopped를 확인했다. 새 StartedAt은15:53:30 UTC다. Research·Daily나 다른 컨테이너를 시작하지 않았다. 기존 reader drain attestation은 과거 세대 기록으로 보존하며 이후 빌드에 재사용할 수 없다. 보호된 근거는 `official-build-recovery.json`과 `reader-final-restoration-start.json`/`reader-final-restoration-result.json`이다.

`eb48ea2`의 동일 SHA Research smoke `40713287917`은 completed/success가 됐다. CI는 아직 진행 중이고 Web 실패를 수용하지 않았다. 새 문서 후보 `83941da`의 smoke는 진행 중, CI는 앞 실행 뒤 pending이다. 실제 backup·migration·설치·공식 daemon·rollback·Owner 인수는 여전히 OPEN이다.

01:00 KST 후속 회수: `eb48ea2` smoke `40713287917`과 `83941da` smoke `40717832340` 모두 실제 checkout SHA·static PASS·functional PASS·정리 original_exit0/cleanup_exit0을 확인했다. 각 프로젝트 컨테이너·볼륨·네트워크 및 context image의 remaining=NONE과 각 job의 private builder 제거 성공을 대조했다. 원본 log는 각각327,523 bytes/SHA-256 `efa2a6029193d8eafb7cc954672ce1b9b33d2e38df5388fc0ef2cccd1da0638b`,224,667 bytes/`20685b1af6a3d7d4cfedfaaf4f5a53fd59d23ca9881aa1b28852099736061cf2`로 새 private 경로에 회수했다. 재실행은 없었다. 첫 후보 workspace도 PASS이며 R4 전용 빌드는 계속 실행 중이다. Web 실패와 전체 CI 미수용, 백업 계약 선택 대기·migration/설치 미실행 상태는 유지한다.

## Owner 채택: 별도 미활성화 체크포인트

Owner는 DB와 설치 설정의 두 날짜를 각각 보존하는 별도 사전 백업·복원 경로를 선택했고, 야간 종료 한도를08:30 KST로 앞당겼다. 정상 Daily `bound_database`의 날짜 일치 guard는 AST가 동일하다. 새 `preactivation-backup`/`preactivation-verify`는 schema62·R4 부재·Daily OFF/epoch0·Daily 실행/승인/history 부재·Owner/policy/trigger 일치와 두 명시 날짜를 요구한다. 이전 immutable manifest에 바인딩된 legacy worker도 성공적으로 정지한 세대만 관측하며 DB control 수정·source 정리·서비스 재개는 호출하지 않는다. 별도 marker/state/prefix로 정상 Daily backup과 구분하고, 원 DB control 날짜와 원 config 파일/해시를 각각 암호화해 보존한다. 기존 체크포인트 자동 pruning은 없다.

두 구현/회귀 파일을 coordinator가 inline으로 처리했다. 기존 R4 계획의 native agent/새 분석 작업자 금지와 두 파일 소규모 수정 원칙을 따른다. 첫 happy-path 회귀는 구현 전 누락 함수 AttributeError로 실패했으며, 구현 후 기존 파일의 root 회귀94개와 simulation backup10개가 PASS했다. 이94개에는 새 guard/cohort/실패 순서·CLI pin 검사7개와 실제 OpenSSL·파일 archive/복원2개가 포함된다. root archive 검사의 DB 복원은 fixture seam이므로 실제 PostgreSQL 인수로 주장하지 않는다. 실제 PG dump·격리 restore는 다음 운영 실행에서 별도로 확인한다. Python AST·diff 공백 검사도 통과했다.

`ce-simplify-code`의 reuse/quality/efficiency 관점을 inline 적용했다(새 native reviewer는 실행하지 않았다). 기존 dump/encryption/file/PG restore primitive를 재사용하고, 추가된 중복 project-policy 읽기는 제거했다. 새 source-observation controller를 정상 disable/repair/resume controller와 분리한 것은 의도적이며 이 구분을 합치지 않았다. 모든 원래 root ancestry·공간·owned verifier cleanup guard를 유지한다.


## 2026-10-29 01:35–01:50 KST — 실제 checkpoint와 OFF bridge

별도 유지보수 commit `03cad03c38f19372b64e64d83b336895450e153f`를 현재 application `a6da63c`에 바인딩해 설치했다. tool manifest는 `sha256:c9b69202770b1eb69052930e7af03ab59e9e7369652dc5cb38a7a462dd790546`다. 실제 암호화 checkpoint `backup-preactivation-20261028T163508Z-a6da63c134fa471ff91e28a32c4b6a2ad9c08ba7`의 publication 전 실제 network-none PostgreSQL 복원이 성공했다. 저장된 COMPLETE/cipher digest를 다시 읽는 독립 `preactivation-verify`도 exit0/`PREACTIVATION_VERIFIED_ISOLATED_DISABLED`다. 두 systemd 작업은 자식이 종료된 MainPID0/exited/success로 회수했다. DB10/21과 config10/22, Daily OFF/epoch0, Daily audit digest `0024d15cbf424f811e3afdedbd93a3a2a81aeb289a56c46792f52f3b9895e908`가 그대로다. 기존 백업·타이머·정상 Daily guard는 변경하지 않았다.

기존 API는 정확한 세대를 재관측하고 TERM/동일 세대 exit0을 확인한 뒤, 원래 이미지·13개 read-only bind를 유지한 admission-disabled 설정으로 ordinary Compose replacement했다. 건강한 새 maintenance 세대의 접수 차단을 기존 simulation controller가 독립적으로 확인했고, 공식 installer가 `a006e5e`에 대해 `LEGACY_OFF_ACTIVATED` 경로로 원자적 current 전환을 완료했다. 설치 로그의 최종 PASS는 manifest V3이지만 기존 출력 문자열 `installed-v2`를 그대로 보존한다.

초기 전환은 source maintenance API가 아직 이전 commit인 점과 기존 Daily worker가 더 오래된 immutable `e639169`에 속하는 점을 분리했다. 원 config/DB와 정지 worker 소유권을 각각 확인하는 root coordinator 인계에서 기존 API를 다시 정확히 정지하고, 새 manifest의 exact image ID로 공식 db-role-bootstrap와 db-migrate one-shot을 실행했다. migration63/64/65와 strategy catalog5 sync가 exit0이다. 새 API/Web/proxy는 ordinary Compose로 순차 기동해 healthy이며 API의 image-bound `http-drain-contract=v1`도 확인했다. 기존 V2 리더는 이 bridge 기동에서 교체하지 않았고 Daily/research/simulation은 OFF다. 이 유한 초기 인계를 전체 compose wrapper의 성공이나 이전 API의 새 handler-drain 증거로 부르지 않는다. SQLx65행·worker SELECT true·admin SELECT false·worker DML false를 별도 실제 read-only 조회로 확인했다.

작은 coordinator 오류도 구분해 보존했다. 첫 정산 조회는 tenant RLS에 맞지 않는 전역 `row_security=off`라 실패해, bounded users/actor 순회로 고쳤다. 이후 원장 확인에서 채택되지 않은 admin SELECT까지 잘못 기대한 조회는 실패했고, worker-only 계약의 실제 bool 결과로 정정했다. 원 helper가 숨긴 SQL 오류 stream은 UNAVAILABLE다. GitHub CLI에 지원하지 않는 `attempt` JSON 필드를 요청한 한 건도 새 조회로 정정했으며 서버 변경은 없었다.

`eb48ea2` 후속 이미지는 새로 정지한 V2 세대를 사용해 공식 one-service/Cargo2/Compose1 경로로 빌드한다. 준비 코드의 `/current` symlink read는 trusted-reader가 거절해 정확한 immutable 경로로 고쳤다. 첫 새 attestation의 `+00:00` timestamp도 canonical `Z` 문법에 맞지 않아 compiler 시작 전 거절됐고, 원 파일/로그를 남긴 채 새 attestation·새 unit으로 재개했다. 원래 unless-stopped/maxretry0와 정확한 V2 image/ID를 보존하며, 빌드 종료/최종 인계 전에 명시적으로 복귀시켜야 한다.

새 유지보수 commit의 전체 CI `40728981056`은 별도 verification ref에서 한 번 dispatch했다. 이는 기존 같은 ref의 pending Web retry를 교체하지 않는다. eb48의 원 Web 실패와 단 한 번의 attempt2, 839의 진행 중 CI는 계속 미수용이다. 두 successor smoke의 기존 성공은 유지한다.

현재 source/사전 backup·restore/초기 DB·OFF bridge 기동 증거와 실제 simulation 사용·이력 보존 rollback·post-R4 backup/restore·Owner 인수는 구분한다. schema62 checkpoint는 이후 simulation 이력을 담은 backup의 대체물이 아니다. 정상 combined backup은 기존 두 날짜와 오래된 Daily worker 소유권을 여전히 독립적으로 다뤄야 하며, 그 gate를 우회하거나 완료로 표시하지 않는다. 최신 Owner 종료 한도는 **2026-10-29 08:30 Asia/Seoul**이다.


## 2026-10-29 — configured build and inactive Daily/R4 capture source

`eb48ea2` official13-image build is complete, MainPID0/exited/success. Actual13
image IDs and OCI revisions, immutable API drain-v1 and D8 executable/receipt/
COMPLETE match. Its manifest SHA-256 is `6c8f3f7b4c2dbe276622825e9f4429b6c274b59a495b48b637e2dabb7c6e3881`;
D8 receipt `ed4b1d239c2a0154be2e277b6de84fc475dc38955e383025da2153c4150ae79a`,
binary `5150ebd4837c7bd11c67c0ed46d18f6c7ede4c3706353afb372edeaf6a5c4463`.
The full protected config binding was prepared from these actual pins and the
previously verified Owner/rights/input, with explicit bounded resources.
No configured installation or worker activation follows from preparation.
Only after reaping that build and reobserving the exact stopped cohort did the
compatible `83941da` build start. Original V2 restoration remains owed.

The Owner-selected preservation contract now has a separate R4-aware capture
entrypoint for the same inactive Daily state. It includes full simulation
history instead of treating the earlier schema62 checkpoint as sufficient.
Normal `bound_database`, simulation state validator, normal simulation/Daily
routes and schema62 preactivation guards are AST-identical. The distinct format
and state carry both dates and original config hash, and reuse full archive,
encryption, actual API/worker fence, settlement and combined isolated PG audits.
Only the explicit entrypoint differs from the immutable application; all other
Python controller dependencies must match byte-for-byte. No source Daily write,
repair, resume, history omission or automatic pruning is added.

The root suite passes102 tests; existing simulation archive/routing suite passes10.
New cases cover explicit pins, full identity/date state, rejected active/history
or tampered bindings, actual restored config bytes, tool dependency mismatch,
actual controller success/failure ordering and distinct publication/inventory.
One initial new fixture used untrusted `/tmp`, correctly failed trusted ancestry,
and was corrected to the suite's protected `/var/lib` fixture parent. Its full
original stream was tool-truncated and is not reconstructed; subsequent logs are
private. These tests do not claim a real production R4 backup or a newly executed
PG restore; those require the prepared configured worker and actual runtime
operation. Hosted final CI remains separate.


Initial OFF bridge inspection exposed an observer-binding cycle: its current
commit differs from the older service-status receipt, while the normal
observer installer requires admission enabled and therefore rejects the
intentional OFF env. The separate maintenance installer now has an explicit
`--maintenance-off` path that accepts only quotes OFF plus all four Owner-beta
modes disabled; owner-only observer visibility and every manifest/current/unit/
transport/Paper check remain. Default behavior is unchanged. The actual env is
not rewritten, and only the existing observation timer can be activated. The
30 installer regressions pass, including source-env preservation and rejection
of each non-OFF mode. Actual application of this observer receipt remains a
separate operation. Configured successor production preflight now PASS; one
prior call used the validator's `--env-file` option on the wrapper by mistake,
was rejected before any action, then used the wrapper's documented process-local
`LAGRANGE_ENV_FILE` instead.


Actual OFF observer handoff: installed only the new a006 KIS units with
`--install-disabled`, confirmed loaded/inactive/disabled and no service start.
The old observer had already failed exit74 after the current-release switch;
normal installer activation capture rejected that failed state twice. The
coordinator stopped its timer, archived the old policy/history and exact failed
state, and explicitly acknowledged only this stopped observer's failed flag.
No application audit counter or DB state changed, and no success observation
was invented. The pinned479 maintenance installer then applied the a006 all-OFF
receipt successfully. A fresh actual collector run completed exit0 at02:19 KST;
only the observation timer is active. The next release handoff will pause this
observer before changing current and rebind normally afterward. Prior failures
and receipts remain protected. Maintenance46/479 are separately bound to their
intended eb48/a006 applications; installed application bytes are unchanged.


## Compatible build final gate and storage recovery

`83941da` full CI40717826954 completed/success. Its first official production
build completed12 images and D8 Cargo compiled successfully in90.545s, but the
post-producer gate rejected `container-health-invalid`. The exact NT1 readiness
observation reported artifact-volume free space below its unchanged1GiB
threshold with queued backtests0; this was not a compiler failure or OOM.
The reader was explicitly restored to the same original ID/image, original
unless-stopped/maxretry0 and actual healthy state at17:29:39 UTC. That restart
invalidates every earlier stopped-generation attestation.

Root space was about932MiB; historical `/etc/lagrange/release-manifests` outputs
account for about139GiB and were retained. A first bounded unused Docker builder
cache prune reported30.34GB on `/data/docker` and therefore did not fix ROOT;
this distinction is preserved. Clearing only the user's downloaded npm package
cache recovered ROOT to3,260,538,880 bytes, and the actual existing NT1 readiness
became true/healthy without changing thresholds, resetting readiness or
restarting that worker. No release image, DB, provider input or historical output
was removed. Broad disk-usage output was tool-truncated; only metadata was read
and no full original stream is reconstructed.

A fresh official recovery build uses NEW output storage on a protected volatile
bind `/srv/lagrange-r4-build-recovery` backed by a new root0700 Data directory.
It does not move old outputs or change fstab/Docker daemon configuration. It
reobserved the healthy source, explicitly drained its new reader generation,
and launched with new unit/state/attestation through the unchanged serial
Cargo2/Compose1 and service/resource gates. Reuse of eligible cache and final
manifest acceptance still belong to the official builder. Recovery completion,
small persistent control pin recovery, reader restoration, configured runtime,
full R4 backup and rollback remain to be verified. The bind must be reconciled
after reboot; installed app/config/receipt will not depend on a volatile cache.

### Configured 설치와 실제 Compose 인자 계약 (2026-10-29)

`eb48ea2` 공식 설치 재시도는 exit0이고 current가 해당 immutable release다. 첫 시도의 `SIMULATION_INSTALL_FILE`은 생성기 umask가 공개 receipt/COMPLETE 모드를0400으로 만든 원인으로 확인했으며 bytes는 유지하고 입력 두 파일을0444로 정정했다. 남은 partial staging은 보존했다. API는 installer의 정상 drain 뒤 정지 상태다.

실제 설치 Docker Compose의 create 도움말에는 `--no-deps`가 없고 up에는 `--no-start`·`--no-deps`가 있다. 공식 daemon을 처음 만들기 전에 host controller의 해당 호출만 up/no-start로 고쳤다. 의존 서비스 기동·빌드·pull을 금지하고 기존 정확 ID의 별도 start와 pending 소유권/정리 검사는 그대로 둔다. 운영 회귀23개 PASS; 새 host maintenance package를 app revision과 별도 pin으로 식별하며 installed source는 수정하지 않는다. 실제 daemon 성공을 아직 뜻하지 않는다.

### Configured 서비스 실제 기동 및 host 도구 보정 (2026-10-29)

공식 eb48 래퍼는 두 번째 실행에서 exit0/COMPOSE_RELEASE PASS다. 기존 recommendation/candidate/NT1/NT2/V2의 정확한 세대를 restart=no·TERM·wait0으로 종료한 뒤 새 immutable 이미지의 API/Web/작업자를 기동했다. 첫 실행은 기존 V2 read-only rollout process-local 확인 인자 누락으로 거절됐고 기존 작업자들을 명시적으로 healthy 복귀했다. 두 번째는 기존 승인 범위의 확인 값을 주었으며 Research는 계속 정지, Daily 자동 실행도 disabled다.

첫 simulation create 시도는 실패했고 protected pending으로 남았으며 실제 컨테이너 부재를 기존 reconcile-start가 확인한 뒤 해제했다. controller가 버린 최초 Compose stream은 UNAVAILABLE이다. 별도 config-only 진단은 disabled RANGE_RAW sentinel 누락, 이어서 base/overlay security_opt 중복을 실제 Compose에서 재현했다. 새 host 도구는 기존 배포 래퍼와 같은 disabled sentinel과 자체 pin된 overlay의 !override를 사용한다. 수정 모델 config --quiet PASS, 운영 회귀23개 PASS이며 설치된 app bytes는 그대로다.

별도 inactive-Daily/full-R4 backup 경로는 signal 전에 기존 C4 protected handoff journal을 기록하고 정지 세대를 유지하도록 보완했다. 정상 Daily·simulation backup 경로는 바꾸지 않는다. app controller와 새 host controller의 호환은 두 고정 SHA 쌍 하나만 추가 허용하고 나머지 dependency bytes는 동일해야 한다. 일반 사용자로 잘못 실행한 root fixture 실패를 보존했으며 올바른 root fixture105개 PASS다. 실제 post-R4 history backup/restore와 simulation 작업 성공·rollback·Owner 인수는 아직 OPEN이다.

### 실제 시작 거부의 디렉터리 권한 계약 (2026-10-29)

두 번째 시도는 정확 ID437e4c93의 daemon이 exit9로 종료했고 pending은 cleanup_uncertain=true다. config parent의 실제 tar metadata만 메모리에서 읽어 root:root0755, config leaf root:10001 0440을 확인했다. 실행기의 고정 parent 계약은 root:10001 0750이다. 원래 이미지는 이 parent를 생성하지 않고 file mount만 요청했다. secret에는 CR/LF가 없고0440이며 원래 거부 원문은 value-free다.

별도 host overlay에4KiB root:10001 0750 tmpfs parent를 추가하되 기존0440 readonly config file bind를 유지한다. nonroot UID10001·network-none·read-only 동일 이미지의 한정 실제 probe에서 parent0:10001:750·leaf0:10001:440, 두 경로 모두 쓰기 불가와 exit0를 확인하고 해당 임시 컨테이너만 정리했다. 정상 runtime policy는 parent의 정확 uid/gid/mode/size도 검사한다. 실패 세대는 깨끗한 종료로 간주하지 않는다. 기존 pending과 실패 metadata를 보존하고 정확한 정지·실패 소유 세대만 대상으로 한 별도 coordinator replacement disposition 뒤 정상 reconcile/fence/settlement를 다시 수행한다.

### 실제 DB baseline 기대값 모순과 검증 범위 정정 (2026-10-29)

보호 parent 보정 뒤 세 번째 공식 시도 역시 exit9·cleanup_uncertain=true다. 동일 image/config/UID와 제한을 쓰는 한정 diagnostic container에서 자격증명 read/write를 제외한 syscall 추적을 실행하고 해당 임시 자원만 정리했다. 실제 다섯 차례 PostgreSQL connection까지 진행했고 서버 ERROR/FATAL은0이었다. 별도 read-only catalog 진단은 assert_candidate_publication_settlement:security를 반환했다. migration63 첫 정의는 SECURITY DEFINER인데 ROOT_SPECS는 false였다. SQLx 원장과 DB 권한을 바꾸어 잘못된 기대값에 맞추지 않는다.

기대값을 true로 보정하고 정책 fingerprint를 canonical delta JSON의 SHA-256 d0bb075587893e458eab5ec94601fe4d416738f38ff80e9a53ba218f64473853으로 갱신했다. delta는 이전567470f4 정책·migration63 authority·해당 routine true 및 나머지 불변 조건으로 구성된다. 기존 PG test는13개 projection 형태/타입과 command를 검증했으며 전체 observer 성공까지 증명하지 않았다. 이전 전체 관찰 성공으로 읽힐 수 있는 문장은 이 한계로 정정한다. 기존 검증 증거를 삭제하거나 runtime 성공으로 재해석하지 않는다. 동일 scratch PG test에 전체 baseline 성공과 SECURITY INVOKER 실제 변이 거절을 추가했다. Rustfmt/diff는 PASS, 신규 Rust 테스트·CI·공식 이미지·실제 daemon 및 Owner 인수는 아직 OPEN이다. 현재 API/Web/V2는 건강하며 Research/자동Daily는 OFF, 실패한 simulation은 restart=no로 정지 보존한다.

### 초기 실패 상태의 한정 복구 (2026-10-29)

추가 precondition 시도 initial-failure-recovery-02는 기존 self-handoff를 확인해 변경 전에 거절했다. 후속03은 source/target/current가 모두 동일 eb48이고 같은 binding인 초기 self-handoff에만 한정했다. 실제 API v1의 정확한 세대를 정상 종료하고 simulation 이력 EMPTY 및 DB settlement를 재확인했다. 원래 exit9/PID0 세대·두 pending 원본을 보존한 후 ordinary Compose의 새 attempt label로 기동하지 않는 생성 교체만 수행했다. 엄격한 기존 C2 policy와 새 created/never-started 상태를 확인한 뒤 exact pending helper로 두 레코드를 해제했다. 실패를 clean exit0으로 다시 분류하지 않았고 실제 daemon 성공을 주장하지 않는다.

같은 API ID/이미지를 명시적으로 원래 unless-stopped 정책으로 복귀해 healthy를 확인했다. 이전 API fence는 과거 세대 증거다. 현재 simulation은 기동하지 않은 created/OFF이고 새 소스 검증을 기다린다. 근거는 root private /etc/lagrange/release-manifests/r4-eb48ea2/initial-failure-recovery-03이다. 신규3ac05d1은 CI40744933714와 동일 SHA Research smoke를 각각 한 번 요청했으며, 별도 새 소스·복귀 finally를 갖춘 공식 빌드 감독은 아직 실행하지 않았다.

### 신규 회귀 컴파일 보정 (2026-10-29)

3ac05d1 CI40744933714 Clippy는 신규 테스트의 Result::expect_err가 성공 타입의 Debug를 요구해 E0277로 실패했다. 비공개 observation에 Debug를 추가하지 않고 matches! 오류 패턴 assertion로 오류만 검사하도록 한 줄을 보정했다. Python·format·policy PASS와 최초 원본 Clippy log는 보존한다. 전체 PG baseline·Rust 검증은 이 실패 때문에 수용하지 않는다. 불필요한 중복 실행을 줄이기 위해 해당 CI와 같은 SHA smoke40746581267의 남은 작업 취소를 요청했고 실제 정리/최종 상태는 별도 회수한다. 운영 빌드는 시작하지 않았다.

### 전체 PG baseline의 시간 제한 검증 보정 (2026-10-29)

256c9aa CI40747223108은 Clippy/Python/format/policy/Web이 PASS였지만 실제 full trigger baseline에서 SetupResultInvalid로 거절됐다(PG job123024126518). 원본343,941 bytes·SHA256 1fff0625f782a1c9e31d322f0abdc526ea511dcc4fb070901f1bc502a6aacc62를 보존했다. 따라서 실제 처음 거절된 단계는 timeout setup이며, 앞서 확인한 SECURITY DEFINER 모순은 독립된 추가 차단이다. 어느 하나를 유일 원인으로 확정하지 않는다.

PostgreSQL은123000ms를123s,120000ms를2min으로 정규화한다. Setup과 처음/최종 context를 양의 정수 단위(ms/s/min/h/d)의 checked 밀리초 값으로 검증하도록 고쳤다. 정확한 요청 시간은 같아야 하고0·음수·분수·공백·지원하지 않는 단위·overflow·다른 시간은 거절한다. SQL projection·typed binds·budget·read-only transaction·cleanup 계약은 그대로다. 단위별 정상/거절과 전체 fake setup/context 회귀를 추가했다.

또한 선택된 scratch PG test의 worker는 legacy fixture 기본 INHERIT였으나 운영 bootstrap은 NOINHERIT를 명시한다. 이 한 개 disposable test에서 실제 운영 속성으로 맞추고 성공 후 원래 fixture 속성을 복원한다. 운영 DB role/grant는 바꾸지 않는다. 신규 후보의 실제 full-PG/최종CI 전까지 인수는 보류하고, 이전256 CI와 동일 SHA smoke40747236237의 남은 실행은 취소 요청했다. 운영 빌드는 아직 실행하지 않았다.

새 timeout 회귀를 기존 R4 전용 빌드의 엄격한 named-test 재고에 한 번 등록했다. bash -n 및 정의/등록 정적 대조는 PASS다. 로컬 CI-contract unittest는 PyYAML이 없어 import 단계 실패로 미실행이며 stub이나 dependency 변경을 하지 않았다. 실제 전체 Python/CI 계약 검사는 기존 hosted job이 수행한다. 먼저591aa8f의 full-PG 의미 검증 결과를 확인하고, 이 등록을 포함한 최종 후보에 전체 CI를 실행한다.

### 보정된 전체 PG 검증과 최종 후보 (2026-10-29)

591aa8f CI40751576806의 PostgreSQL validation 및 sanitized evidence 단계가 실제 SUCCESS다. 원본 job log107,265 bytes·SHA256 0ebfbb155bc4917d7b6aebde8a5a48568501ac7d9ae45f802588bb86d13f7339를 비공개 보존했다. `jobqueue-r4-trigger-metadata-pg`는 EXIT0/SKIPfalse이고 `r4_trigger_metadata_pg_projection:ok`다. 이 SHA의 테스트 본문은 전체 migrated observer 성공과 실제 SECURITY INVOKER 변이 거절을 무조건 실행하므로 projection 형태만 확인한 이전 증거와 구분된다. 성공 raw log의 별도 FULL_BASELINE marker는 현 workflow가 내보내지 않아 UNAVAILABLE이며 재구성하지 않는다. 다른 모든 PG suite도 EXIT0/SKIPfalse다.

같은591의 Python/policy는 새 timeout 회귀의 strict inventory 누락으로 실패했다. 목록을 한 번 등록한9a6b6b30741184c0049a73a4b72b5168c07d29a0를 원격 검증 ref에 올리고 CI40753752320 및 smoke40753764155를 각각 한 번 요청했다. 이전591의 남은 작업은 취소 요청했으며 전체 CI PASS로 표현하지 않는다. 로컬에서는 기존 정확한 inventory unittest 한 개를 stdlib AST loader로 실행해 PASS다. PyYAML이 없는 전체 모듈 import 실패는 그대로 보존한다.

이 기록을 추가한 docs-only 후보는 올바른 runtime 소스와 동일한 호환 rollback 후보로 준비한다. 실제 같은 SHA의 전체 CI/smoke와 공식 이미지, 두 후보 사이 이력 보존 rollback 전까지 호환 수용이 아니다. 새9a6 root 소스와 공개 prebuild.env는 Data의 보호 경로에 준비했지만 아직 공식 빌드를 시작하지 않았다. 현재 API/Web/PG/수동 sealed worker/V2는 healthy, simulation은 미기동 created/restart=no다. DB/config 날짜와 권한을 변경하지 않았고08:30 종료, 실제 runtime·작업·복구·post-R4 backup·Owner 조건을 유지한다.

### Corrected 후보의 전체 CI 수용과 공식 빌드02 (2026-10-29)

312f4c8401e7922063561381045f2568641008bf의 CI40753963009는 format·Python·Clippy·policy·Web·workspace·PostgreSQL·R4 전용 빌드와 required가 모두 SUCCESS다. 같은 SHA의 PG31개 suite는 exit0/SKIPfalse이며 전체 trigger baseline 성공 및 실제 SECURITY INVOKER 변이 거절 named test도 ok다. R4 전용 빌드는 verifier4·child9·provenance14·runner7·private process290·CLI9·process4·connection proof6의 실제 PASS marker와 두 SHA별 artifact 업로드를 확인했다. smoke40753970379도 checkout SHA·static/functional PASS·harness cleanup0·project 자원 NONE 및 `lagrange-smoke-40753970379-1 removed`를 회수했다. 비공개 최종 CI 수용 JSON SHA256은2017a57ad7db8d3c6bc39759d8a00d2d9e433c0d0ab5cd77ecf17e740104530f다. 이는 격리 검증 수용이며 실제 daemon이나 Owner 사용 성공이 아니다.

프로그램 소스가 동일한9a6b6b3의 최초 workspace는 기존 intraday mock pipeline 두 건이 실패했다. 원본683,324 bytes·SHA256 105394114fc9b7f75e114b4602d75581f409069524a7118fd65c2bd0a73511fe를 보존했다.312에서는 두 테스트가 모두 성공했지만 최초 실패의 최종 eligibility/clock 값이 없어 원인은 미확정이다. flaky나 제품 오류로 확정하지 않고 테스트/제품 guard는 바꾸지 않았다.9a6의 다른 검증과 smoke 성공은 보존하고, 전체 workflow 종료 후 실패 workspace job을 한 번만 재검증 요청해 attempt2/job123085325086이 진행 중이다. 최초 실패는 삭제하거나 새 성공으로 대체하지 않는다.

312의 첫 공식 build supervisor는 정지 증명을 pretty JSON으로 저장해 정상 검증기의 canonical sorted compact JSON+LF 검사에서 거절됐다. 레이아웃 초기화 exit1로 컴파일은 시작되지 않았고, 감독 종료/MainPID0 뒤 원래 manual sealed worker 및 V2의 정확한 ID·unless-stopped 정책·현재 healthy 복구와 compiler 부재를 확인했다. 이 실패와 원래 증명/로그는 root private `r4-312f4c8`에 그대로 보존한다.

별도 `build-02` 제어/출력·unit을 사용하고 저장 형식만 공식 canonical 형식으로 보정했다. 과거 증명을 live proof로 재사용하지 않고 복귀한 새 리더 세대를 다시 관찰·정상 drain0한 뒤 fresh attestation을 발급했다. 실제 공식 레이아웃 검증을 통과해 `lagrange-r4-312f4c8-build-02.service`가13개 이미지 순차 빌드를 시작했다. Cargo2/Compose1, 낮은 CPU/I/O 우선순위와 정상 자원 gate, 컴파일러 회수 후 두 원래 리더의 독립적인 복구를 유지한다. 새 manifest 출력은 보호된 Data의 `/srv/lagrange-r4-build-recovery/312f4c8/build-02/`이고 installed eb48 bytes는 미변경이다. 실제 빌드 완료·새 설치·daemon·작업·이력 rollback·post-R4 backup/restore·Owner는 OPEN이며08:30 종료를 유지한다.

### Corrected 공식13개 이미지 회수와 두 번째 순차 빌드 (2026-10-29)

312f4c8 build02는 MainPID0/SubState exited/Result success/exit0이며 compiler 부재를 확인했다. 실제13개 image ID·OCI revision이312f4c8과 일치하고 API 이미지의 immutable drain-v1 capability도 확인했다. manifest SHA256은ad8a7cd1fe520d4993057969e054b8bb3b4fc8cb0000037601d819f2fcfd1d16, D8 receipt는dd54298f2f0dcb55a47eb3596be59903d011087978121cb10d88389cec1a11bf다. 실제 producer binary81,401,192 bytes의 SHA256 f8c749da8c5420ea7751636bc42922b8e5789840909b4768617734407bd17782와 COMPLETE 바인딩을 대조했다. 두 source reader의 정확한 ID/이미지·원래 unless-stopped 정책·현재 healthy 복구도 확인했다. root private `r4-312f4c8/official-build-recovery.json`에 보존한다.

완료한 실제 manifest/D8와 기존 eb48의 채택된 Owner·권리·입력·limits를 사용한 protected config·env·Daily 입력 생성은 전체 바인딩 PASS다. DB2026-10-21/config2026-10-22와 Daily OFF를 유지하며 아직 설치/활성화하지 않았다. 이 입력은 실제 새 빌드의 pin이며 값이나 권리 승인을 새로 만들지 않았다.

9a6b6b3의 한정 재검증은 같은 SHA·소스 변경 없이 workspace 두 실패가 모두 ok이고 CI40753752320 attempt2의 required 포함9개 결과가 모두 SUCCESS다. 성공한 다른 same-SHA 체크와 artifact는 정상 job 재실행이 보존한 실제 원 실행 증거이며 새로 전부 실행했다고 표현하지 않는다. 최초 실패 원인은 미확정이다. 재검증 원문704,121 bytes·SHA256 ad047c2a6b84f4a03f84090dd95e15962cc79284d002c8da449eeca4ed53ac7c와 최초 실패를 함께 보존했고 추가 재시도는 없다. 최종 수용 JSON SHA256은1ab0b934abc4cb393585281d24eb8ba8dd4a1849807c6e7679c64890e4ff1916이다.

312의 종료/복구 후, current를 eb48로 유지한 채9a6의 별도 build02 감독을 시작했다. 앞선 정지 증명은 재사용하지 않고 현재 리더 세대를 다시 drain하며, 종료 후 두 source reader의 독립 복구가 필요하다. Root control·Data 출력·unit을 후보별로 분리했고 production Rust는 겹치지 않는다. 실제 current 전환은 이 빌드와 복구가 끝난 뒤 수행한다. 별도 제한된 API 검증 코드는 준비만 했으며 실제 session/config/job은 아직 만들지 않았다. 향후 operator session은 빈 amr·OIDC/MFA 주장 없음·메모리 bearer와 최종 revoke를 사용하고 human Owner 사용 인수와 구분한다. 실제 runtime·작업·이력 rollback·post-R4 backup/restore·Owner와08:30 종료는 OPEN이다.


## 2026-10-29 06:40 KST: corrected 두 공식 빌드·실제 설치와 worker 역할 전제

9a6b6b3도 공식13개 이미지/OCI/API drain-v1/D8/manifest를 회수했다. manifest SHA256은87fde0ee12dc15ea13da16ec8d4b251c836a308451d141c631d67eea0839905c, D8 receipt c2e907b4a1ae653c66fa5913c64b26672a40eb697389f7e6afc9d8d1ff169b51, 실제 binary81,401,584 bytes·SHA256 c840685d95d525f0b12072fac68d0e8dc17e62ab72b617b1e09a5e6acf3501b4다. 감독 exit0/MainPID0·compiler 부재·두 source 리더의 원래 정책/healthy 복구를 확인한 뒤 두 번째 protected config 바인딩도 PASS다. 반복 빌드는 없다.

정상 installer가312f4c8을 설치했고 KIS unit은 disabled로만 설치했다. 기존 관측 policy/history를 보존해 정상 receipt를 rebind한 뒤 공식 release wrapper가 exit0/COMPOSE_RELEASE PASS다. simulation은 별도 시작에서8b975e1a 세대가 exit9/PID0/OOMfalse/restartno로 실패했다. 원래 exit9와 primary/cleanup uncertainty pending을 보존한다.

connect/exit만 기록하고 read/write/send/receive 버퍼를 제외한 한정 동일 이미지 진단은 PG 연결5개 뒤 exit9였다. 진단 전후 실제 simulation history EMPTY, 원래 실패 세대 불변, 소유 probe 정리0을 확인했다. 실제 worker의 원래 SQL을 RO/RR transaction에서 조회한 결과 context의 worker_inherits_roles=true가 strict 검증과 반대였다. 관계10·root17/settings25·forward21/adjacent1/reverse27·UPDATE OF2/constraint13과 역할 멤버십0은 기대 개수에 일치했다. scratch full-observer 회귀가 NOINHERIT fixture를 사용한 것은 실제 운영 속성의 증명이 아니며 이 전제 공백을 정정한다. SQL 조회는 성공했고 처음 multiline JSON을 line 단위로 읽은 JSONDecodeError는 원문을 재조회하지 않고 raw_decode stream 파싱으로 보정했다. 몇 건의 잘못 추정한 소스 경로 검색 실패도 제품 실행 성공으로 표현하지 않는다.

멤버십 변경을 잠근 transaction에서 정확한 worker OID/기존 비특권 속성·멤버십0을 확인하고 NOINHERIT로만 제한했다. 전체 table/routine/schema/database ACL 및 역할 멤버십 digest는635fe53f5487416c49abb6e32ce0fcf94ea7dc79f976ec99574e9bb558c5dd94로 전후 동일하다. 새 GRANT/REVOKE·멤버십·비밀번호·다른 role flag·Daily 날짜/이력 변경은 없다. 기존 bootstrap은 worker의 INHERIT 속성을 보존하므로 새 설치도 이 strict 전제를 사전에 확인해야 한다. observer 조건은 완화하지 않는다.

현재312의 실제 target APIv1 generation을 새로 fence하고 simulation history EMPTY와 DB settlement를 독립 확인한 한정 초기 실패 복구를 수행했다. 원래 EB→312 교체 기록과 failed-start pending을 별도 보존했고, ordinary Compose create-only replacement로 새 never-started created/restartno 세대를 만든 뒤 strict runtime policy와 fresh fence/settlement를 다시 확인해 정확한 두 pending만 해소했다. 원래 exit9를 clean0으로 바꾸어 부르지 않으며 실제 이력이 있는 일반 cross-commit 복구를 허용하는 절차가 아니다. 같은 target API의 정확한 ID/image·원래 restart 정책·healthy를 명시 복구했다. 재기동02와 실제 작업/rollback/post-R4 backup/Owner 결과는 별도 회수한다. 비공개 근거는 root r4-312f4c8의 initial-corrected-deploy-01, simulation-start-01, startup-connect-diagnostic-01, worker-role-hardening-01, initial-failure-recovery-01이다.


공식 simulation 재기동02는 exit0/MainPID0/SIMULATION_OPERATION STARTED다. 실제49d2496547d9dfcb9f4b2a9919fd2936e5f0672a36a7e89dc59244871332fd13은 정확한312 image/OCI·running/healthy/restartno, 두 pending은 부재다. 정상 startup 관찰과 root health가 실제 성공했으며 작업 성공은 아직 별도다. 임시 operator 코드 첫 실행은 API가 붙지 않은 DB-only 내부망을 선택한 guard에서 session/config/job 생성 전에 거절됐다. 실제 API의 project-bound internal edge망/정확 endpoint만 선택하도록 별도02 코드/로그를 만들었으며 네트워크 구성은 바꾸지 않았다. 두 번째 실행이 정상 API 작업을 수행 중이고 최종 session revoke/결과 쌍을 회수하기 전에는 완료로 표시하지 않는다. 새 앱의 직접 inactive-simulation backup config는 preflight PASS지만 실제 backup/restore는 아직 미실행이다.


## 2026-10-29 07:05 KST: 실제7개 이력·왕복 전환과 보존 backup 보정

전체기간v1 e8ab8a43-e971-4565-91b8-ca6a445ae70a는2252평가세션/두 프로필 공개가 성공했다. 취소 준비 시 이미 해당 작업은 정상 완료·operator 프로세스는 다음v2의422 거절 후 종료돼 실제 TERM/취소 신호는 보내지 않았다. 거절된v2는 Python의 느슨한1==1.0 비교가 기존 BuyAndHold integer1 config를 골랐기 때문이다. 입력 snapshot은 정확한 기본float1.0과 hash를 구분하므로 검증 코드를 직렬화 타입까지 비교하도록 고치고 정상 API로 canonical 기본 config를 새로 만들었다. 기존 Owner config와 제품 guard는 수정하지 않았다.

전체기간 금융 G2 통과 증거는 그대로 유지한다. 운영 연결의 유한 실행은 실제 승인 bars에서 추출한 마지막20세션(2026-07-31~2026-08-28) 동일 구간으로 v1 trend1개와v2 BuyAndHold/Trend/RelativeMomentum/DualMomentum/InverseVolatility5개를 수행했다.6개 모두 SUCCEEDED·각 두 whole-profile hash와 API pair visibility를 확인했고 operator03 unit exit0/MainPID0다. 이 짧은 구간을 전체기간 runtime 성능 검증으로 과장하지 않는다. 별도2252세션v1까지 실제7개 이력이 생겼다. 두 operator session은 normal finally revoke 후 정확한 자체 session ID만 대상으로 DB의 revoked_at을 독립 확인했다. 기존 사용자 credential/session을 읽거나 OIDC/MFA/human Owner 사용 인수를 주장하지 않는다.

정상 installer/APIv1 fence/공식 wrapper/daemon을 통한312→9a6 upgrade 및9a6→312 rollback은 각각 exit0/MainPID0다. 각 전후 simulation/전체 Daily audit 값은 정확히 같다. simulation digest는sha256:e348f449d61d0346c2889e0b8abc1fed0fac09f2cf4c18c445b00c2347dc8ef9이며7개 작업/쌍 이력을 보존했다. 현재는312다. 원본 리더·상태 관측기는 자연 종료한 성공 수집 상태에서 timer만 먼저 일시 정지해 전환 중 signal failure를 만들지 않았다.

첫 post-R4 inactive-Daily/simulation backup은 manual sealed Beta의 정확 세대 TERM/wait0 뒤 실제 APIv1/simulation drain0·독립 settlement를 확인했으나 최종 거절됐다. 정상 backup으로 게시하지 않았고 root inactive-simulation 저장소는 비어 있다. 원래 CLI는 DAILY_BACKUP: FAILED만 출력해 첫 내부 예외의 구체 코드는 UNAVAILABLE다. 실제 전체 historical input에는 승인V3 b0fb82...와 기존 승인V2 kis-historical-price-only-beta/v2/candidate-sha256=0877d42...가 함께 있다. 기존 strict verifier는 simulation_input에 V3 세 항목만 허용하므로 이 보존 이력을 받을 수 없는 확정 blocker가 있으며, 정확한 유효 archive 회귀에서 SIMULATION_OPERATION_APPROVAL을 재현했다. 최초 fixture 정렬 오류는 클래스 순서로 보정한 뒤 실제 거절 경로를 확인했다.

기존 embeddedV2 registry SHA256 4111f51d945a48a7559b22863cc4ed2eae9c760d5ac9288e554aefe5575e3380과 원 V2 manifest afd0735d.../5,401,077-byte bars e67d0bd7...를 직접 대조했다. 새 입력이나 법적 권리를 만들지 않고 이미 승인된 이력만 보존한다. 별도 backup entrypoint의 inactive verifier가 두 전체 input alias를 동일하게 요구하고, 정확한 기존V2 registry/manifest/전체bars hash·크기·mode 및 V3 기존 바인딩을 검증하도록 보정한다. 임의 sibling·다른 bytes/registry·누락·alias 불일치는 거절하며 정상 simulation verifier와 Daily guard는 그대로다. 다른 Python controller는 바꾸지 않아 별도 maintenance package와 실제 application의 기존 exact-dependency 계약도 유지한다. value-free OperationError는 기존 대문자 code 제한 안에서만 진단한다.

기존105개 root fixture를 nonroot로 실행한 오류(61 failures/9 errors)는 보존하고 실제 root로 올바르게 재실행해105 PASS다. 새 합성 archive suite는17 PASS, 실제 source/text diff 및 AST 검사는 PASS다. 실패한 backup을 통과로 바꾸지 않고, 실제 stopped handoff/독립 DB 감사 불변을 확인한 별도 명시 복구로 API/simulation/manualBeta/V2를 healthy와 원래 정책으로 복구했다. installed312 bytes는 편집하지 않는다. 한정 ops maintenance 후보의 기존 전체 CI와 실제 재백업/격리 복원은 아직 OPEN이며08:30 종료·human Owner 구분을 유지한다.


## 2026-10-29 07:35 KST — post-R4 encrypted capture and first isolated restore

별도 도구 `756c3422bdaa2a7e6eff78bf4b1368f079b3ed8c`의 CI40777497812는 workspace/Python/policy/Clippy/Web/format/PostgreSQL/required 모두 SUCCESS다. ops-only 유지보수라 선택 R4 binary build는 SKIPPED이며 실제 애플리케이션312의 기존 full-R4/smoke/공식13개 이미지는 별개로 유지한다. policy 원문208,332 bytes/SHA256 `8fca5e7c4e8d1e6ac0ead2769853e3733db108b88074a2a836524d3f96cb90e4`에서 root105·simulation17 PASS를 회수했다. 최초 원문 저장 명령은 argument-list-too-long으로 프로세스 생성 전에 거절됐고, 한정 chunk로 원 decoded bytes를 보존했다.

756 immutable tool manifest는 `sha256:702ae2c31d7e47e44343269dbd496746fa1354ed793e00b24e82a2c5a50d681f`다. 현재312와 새 config만 바인딩했고 설치 application bytes·날짜·기존 backup/timer는 바꾸지 않았다. `lagrange-r4-post-history-backup-02.service`의 첫 단계는 실제 암호화 capture와 격리 PostgreSQL restore 후 exit0/INACTIVE_SIMULATION_VERIFIED_DISABLED다. 완성 집합 `backup-inactive-simulation-20261028T223556Z-312f4c8401e7922063561381045f2568641008bf`의 COMPLETE가 있다. 이것은 두 번째 독립 복원까지 PASS했다는 의미가 아니다. 첫 verifier의 live polling은 관측 전에 종료돼 직접 inspect 결과를 발명하지 않는다. 정상 tool의 실제 restore/cleanup 성공과 첫 단계 exit0 증거를 유지한다.

두 번째 독립 verify는 exit1/generic FAILED였고 unit은 MainPID0/failed다. `_handoff_read`의 실제 switched source=target312 기록에서 원 main의 `inspect_transition`을 읽기 전용으로 호출하면 SIMULATION_OPERATION_PENDING이 재현된다. 원 프로세스의 세부 exception stream은 UNAVAILABLE로 유지한다. 단순 no-pending 우회·journal 삭제·low-level Storage 호출 대신, inactive verify 전용 context가 기존 `_handoff_context`의 전체 pin/START_PENDING 검증과 `_handoff_stopped`의 정확한 API/worker 세대·DB settlement를 전후 호출한다. journal은 검증 중 동일해야 하며 소비하거나 수정하지 않는다. 다른 backup/verify/activation guard는 유지한다. 신규5개 regression은 구현 전 실패했고 구현 후 simulation22개가 PASS다. 실제 두 번째 복원과 명시 정상 복귀·human Owner는 아직 OPEN이다.

root105 최초 재검사는 private 로그 생성을 위한 parent umask077을 fixture subprocess에 상속해1 failure/23 errors였다. fixture가 가정하는 umask022를 subprocess에만 복원한 재검사105개는 PASS(2.497s)이며 실제 tool의 private umask077·보안 검사는 바꾸지 않았다. 원 실패 stdout/stderr도 별도 보존한다.
