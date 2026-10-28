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
