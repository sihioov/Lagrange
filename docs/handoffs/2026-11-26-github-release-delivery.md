---
artifact_contract: "ce-handoff/v1"
created_at: "2026-11-26T02:30:52Z"
title: "GitHub에서 빌드한 정식 릴리스를 로컬 VM으로 가져오기"
summary: "R5-04의 다음 작업, 기존 빌드와 설치 사이에 빠진 단계, VM 인수 조건과 이후 거래일 관찰 순서"
keywords: ["R5-04", "GitHub Actions", "release images", "VM", "R2", "R3"]
repository: "sihioov/Lagrange"
branch: "main"
head: "c1bdfaee3bc48ce303ea995880e27abe7815e7a1"
resume_focus: "GitHub에서 정식 이미지를 한 번 빌드하고 같은 결과물을 격리 VM에 받아 재빌드 없이 검증하는 경로 구현"
---

# 다음 세션 첫 작업 — GitHub 빌드와 VM 인수

11월 26일 Owner는 **GitHub에서 정식 배포용 이미지를 만들고, 로컬에서는 완성본을 받아 쓰는 경로를 먼저 구축**하기로 했다. 첫 인수 장소는 격리된 Linux/amd64 시험 VM이다. 여기서 검증한 이미지 세트를 향후 정식 배포에도 그대로 사용한다. 이 인계 시점에는 전달 경로 구현, VM 반입, 운영 적용이 모두 남아 있다. 전체 우선순위는 [로드맵 §4.1](../ROADMAP.md#41-r2r3r5-공동-실행-순서-owner-결정-2026-11-20)에 있다.

## 어디까지 확인했나

| 구간 | 11월 26일 확인 결과 |
| --- | --- |
| GitHub에서 하는 일 | 기준 커밋의 필수 CI와 연구 smoke가 통과했다. `paper-image-integration`은 실제 이미지 13개로 격리 수명주기를 검사한다. 이 CI 실행에서 다운로드할 수 있는 산출물은 구조도 한 건이었다. 정식 이미지와 V3 manifest를 받을 곳은 아직 없다. [CI 실행](https://github.com/sihioov/Lagrange/actions/runs/45713220051) · [연구 smoke](https://github.com/sihioov/Lagrange/actions/runs/45713220072) |
| 현재 정식 배포 | 운영 중인 릴리스는 `a90353ad`다. 정식 빌더는 지금까지 호스트에서 서비스 하나씩 빌드하고 저장한 이미지 바이트를 검증한 뒤 V3 manifest를 만들었다. 설치기는 정확히 일치하는 Git 커밋과 manifest가 필요하다. Compose는 로컬 이미지 ID와 OCI revision을 확인한 다음 그 ID로 시작한다. [빌더](../../scripts/ops/build-production-images.sh) · [설치기](../../scripts/ops/deploy-production-release.sh) · [Compose](../../scripts/ops/compose-release.sh) |
| R2·R3·R5 | 8/31~11/24 운영 입력은 58/58거래일 등록, 11/25분은 정규 수집과 격리 등록까지 성공했다. R5 백업·상태·기동·수동 복구와 R2 봉인 입력 수정이 main에 들어갔다. 11/26 예약 백업은 암호화·격리 검증 후 원래 서비스가 복귀했다. 운영 관측기의 정책 불일치 표시, 새 릴리스의 인증 화면 대조, 구성된 Paper의 결합 복원은 남았다. Daily OFF, Paper CLOSED, 공동 관찰 0/3이다. [R2 등록](https://github.com/sihioov/Lagrange/pull/72) · [백업](https://github.com/sihioov/Lagrange/pull/75) · [마운트](https://github.com/sihioov/Lagrange/pull/76) · [상태](https://github.com/sihioov/Lagrange/pull/77) · [입력](https://github.com/sihioov/Lagrange/pull/78) · [수동 복구](https://github.com/sihioov/Lagrange/pull/79) |

## 구현을 어디서 시작하고 무엇으로 완료를 판정하나

1. **정식 빌드 실행 환경부터 검증한다.** 최신 main에서 후보 커밋을 고정하고, 호스팅 실행기가 정식 빌더의 전제를 충족하는지 확인한다. 현 빌더는 우선순위를 낮춘 systemd 서비스, 실제 서비스·컨테이너 상태, 커널 OOM 기록을 검사한다. 격리 실행기에 맞는 검사를 설계하고 제품 모드의 이미지 바이트·출처 검증을 유지한다. PR은 기존 검사를 실행하고, 같은 후보의 `required`와 `paper-image-integration` 성공을 확인한 신뢰된 릴리스 작업만 배포물을 게시한다. [빌드 환경 검사 및 이미지 보관](../../scripts/ops/lib/release-build-layout.sh) · [필수 CI](../../.github/workflows/ci.yml)
2. **같은 결과물을 다시 받을 수 있게 보관한다.** 13개 서비스의 정식 이미지와 V3 manifest를 한 릴리스로 묶는다. 전체 Git SHA, 플랫폼, 이미지 ID·OCI revision, 전송 아카이브의 무결성 해시(레지스트리 사용 시 해당 digest), 출처와 CI 실행을 연결한다. V3의 로컬 Docker 이미지 ID와 전송물의 digest는 각각 검증한다. 허가된 수신자만 읽을 수 있는 저장소에서 변경 방지·보존 기간·과거 버전 재수신을 검증한다. GHCR 등 구체적인 저장 방식은 구현 시 권한·보존 조건을 확인해 정한다. 수집 원본·시세, 운영 DB·환경 파일·키, 빌드 캐시와 원시 로그는 반출 대상에서 제외한다. [V3 manifest 형식](../../scripts/ops/lib/release-image-manifest.sh) · [현행 릴리스 계약](../runbooks/production-release-and-backup.md)
3. **빈 시험 VM에서 동일성을 입증한다.** 인증된 경로로 받은 배포물의 출처와 해시를 검증한 뒤 반입한다. 13개 태그·ID·revision·플랫폼을 원래 manifest와 대조한다. 일치하는 설치 소스와 보호된 manifest를 사용해 합성 설정과 Paper OFF로 설치 사전검사와 기동을 시험한다. 오프라인 기동에 필요한 고정 PostgreSQL·nginx 이미지도 준비한다. 시뮬레이션 설치에 필요한 D8 빌드 증거(`artifact.json`·`COMPLETE`)는 공개 가능 필드를 확인하고 보호된 경로로 함께 인계한다. 로컬 Cargo·Compose 빌드 횟수 0, 같은 릴리스 재반입 성공, 손상·다른 커밋·플랫폼 불일치 시 기동 전 거절을 기록한다. 이 결과를 얻으면 전달 경로를 완료로 판정한다. 운영 전환 때 같은 배포물로 설치 전 백업·복원 및 기존 릴리스 복귀 절차를 검증한다.

## 로컬 자원과 그다음 순서

11/26 표본에서 호스트 메모리는 약 14.5 GiB, 가용 메모리는 9.5~9.8 GiB, 사용 중인 swap은 약 7/8 GiB였다. 시스템 디스크와 데이터 디스크 여유는 각각 약 78 GiB, 85 GiB였고 데이터 디스크 사용률은 약 96%였다. 11/24에 시스템 전체 OOM이 두 번 있었다. KVM을 사용할 수 있다. **2 vCPU·4 GiB RAM·32 GiB 디스크**는 시스템 디스크에서 오프라인 시험을 시작할 제안값이다. 실제 메모리·swap·임시 디스크 최대 사용량과 기존 서비스 상태를 재서 수일간 쓸 수 있는지 판정한다.

- 전달 경로를 통과한 뒤 비공개 R5 검증을 준비한다. 같은 후보의 호환 `job-queue` Rust 테스트 실행 파일(libtest ELF), 고정 PostgreSQL, 지속되는 격리 DB, 허가된 비공개 원본이 필요하다. 실제 네이티브 검사는 서로 다른 네 거래일의 수집과 공유 KIS 키·정규 수집 일정 조정이 필요하다. 원본은 보호된 장소에서만 사용하며 시험 VM의 서비스·Docker·프로세스 경계는 실제로 격리한다. 매일 백업은 유지하고, 복원 검증이 성공해야 원래 실행 중이던 서비스를 복귀시킨다. 실패 시에는 수동 복구한다. [R5 운영 계약 §9.5](../contracts/owner-beta-paper-operations-v1.md#95-required-verification-matrix-not-executed-by-this-amendment)
- 이후 승인된 고정 릴리스에서 Daily를 선택하고 공동 관찰을 시작한다. 첫날부터 16:29 KST까지 예약을 확정하고 16:30~16:31에 실제 실행돼야 한다. R2·R5는 같은 연속 3거래일을 각 기준으로 검사하고 R3는 그중 조건을 충족한 장중 세션을 확인한다. 과거 58거래일과 격리 검증의 네 거래일은 이 횟수에 합산하지 않는다. Paper는 관찰 동안 OFF이며, 활성화 후 별도 연속 3거래일 인수가 남는다. 시작일은 아직 정해지지 않았다. [시장 검증 일정](../ROADMAP.md#41-r2r3r5-공동-실행-순서-owner-결정-2026-11-20)
