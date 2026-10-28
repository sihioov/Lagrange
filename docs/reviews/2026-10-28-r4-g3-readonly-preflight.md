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
