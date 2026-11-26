# GitHub 정식 이미지 빌드와 시험 VM 반입 (R5-04)

소스는 공개 저장소 `sihioov/Lagrange`에 있다. 빌더가 만든 이미지, V3 manifest와 D8 빌드 증거는 별도의 비공개 GitHub 저장소에서 하나의 immutable release로 보관한다. 공개 Actions에는 전송 목록의 서명과 그 목록의 해시가 남는다. 파일을 받는 쪽은 소스 커밋, 빌드 실행 번호, 실행 시도 번호로 받을 릴리스를 선택한다.

## 게시 환경

- 비공개 보관 저장소에 첫 커밋과 기본 브랜치가 있어야 한다. 설정에서 immutable releases를 켜고 읽기 권한을 허가된 수신자로 제한한다. 과거 릴리스는 Owner가 보존하는 동안 다시 받을 수 있다. 저장소 삭제와 관리자에 의한 릴리스 삭제까지 막는 보존 계약은 별도로 관리한다.
- 공개 소스 저장소에 GitHub environment `release-delivery`를 만들고 배포 브랜치를 `main`으로 제한한다. environment variable `RELEASE_STORE`에 보관 저장소의 `owner/repo`를, environment secret `RELEASE_STORE_TOKEN`에 해당 보관 저장소 전용 자격 증명을 설정한다. 보관 저장소용 세부 권한은 `Administration: read`와 `Contents: write`다. 브랜치 제한을 적용한 뒤 자격 증명을 연결한다. 수신용 자격 증명은 같은 보관 저장소의 `Contents: read`로 제한한다.
- `main`의 해당 SHA에서 완료된 CI 실행 번호를 선택한다. 빌드 작업은 CI workflow 경로, 저장소, 브랜치, SHA, 전체 결과 및 `required`와 `paper-image-integration`의 성공을 API에서 확인한다. 실패한 작업만 재시도한 경우에는 같은 CI 실행에서 각 검사가 마지막으로 수행된 결과와 SHA를 확인한다.

[GitHub immutable releases](https://docs.github.com/en/code-security/concepts/supply-chain-security/immutable-releases) · [릴리스 설정 확인 API](https://docs.github.com/en/rest/repos/repos#check-if-immutable-releases-are-enabled-for-a-repository)

## 빌드와 보관

Owner가 소스 저장소 Actions에서 **Private release delivery**를 `main`에 수동 실행한다. 위 CI의 숫자 ID를 입력한다. 기본 실행은 호스팅 빌드, 전송물 생성, CI 대조까지 진행하며 실행기가 종료되면 이미지가 폐기된다. 완성본을 저장해 VM으로 가져올 때 `publish`를 선택한다. 이 실행에서 만든 이미지를 그대로 게시한다. 빌드와 게시에 성공하면 `lagrange-<소스 SHA>-<빌드 실행 ID>-<시도 번호>` 태그가 생성된다. 이 태그가 있는 저장소의 커밋은 보관 저장소의 커밋이다. 실제 제품 소스는 서명된 `delivery.json`의 `commit`으로 확인한다.

빌더는 한 번에 한 서비스만 만들고 실제 이미지 바이트를 검증한다. 전송 목록에 빌드·CI 실행과 파일별 SHA-256을 기록해 GitHub Actions의 증명을 붙인다. 게시 시 비공개 여부와 불변 설정을 대조하고, draft에 모든 파일을 올려 GitHub가 보고한 해시와 대조한 뒤 공개 범위가 비공개인 저장소 안에서 릴리스를 확정한다. 중단된 draft는 설치에 쓸 수 없다. 다시 실행하면 새 시도 번호가 붙고, 이전 태그와 파일은 덮어쓰지 않는다.

## 같은 배포물 수신

인터넷을 사용할 수 있는 준비 환경에서 Python 3.11 이상 및 `gh attestation verify --source-digest`를 지원하는 GitHub CLI를 준비한다. 수신 대상에 대한 읽기 권한으로 인증한다. 아래 변수는 Owner가 고른 릴리스의 값이다. 저장 공간은 다운로드 파일, 압축 해제 및 Docker 레이어를 고려해 실측한다.

```bash
python3 scripts/ci/release_delivery.py fetch "$NEW_DIRECTORY" \
  "$COMMIT" "$BUILD_RUN" "$BUILD_ATTEMPT" "$RELEASE_STORE"
```

수신기는 저장소의 실제 공개 범위, 릴리스 불변 상태, 원격 파일 목록과 SHA-256, Actions 서명, 서명한 workflow와 소스 브랜치·커밋을 검증한다. 성공한 경로에는 `delivery.json`, `delivery.sigstore.jsonl`, 검증된 `bundle/`이 있다. 과거 세트를 다시 받을 때도 전체 식별자를 지정한다. [GitHub CLI 서명 검증](https://cli.github.com/manual/gh_attestation_verify)

## 오프라인 VM 검사

준비 환경에서 `gh attestation trusted-root > trusted-root.jsonl`로 신뢰 루트를 받아 신뢰할 수 있는 경로로 VM에 설치한다. 다운로드 디렉터리와 정확한 커밋의 깨끗한 소스 checkout도 반입한다. VM에서는 root로 검증한 checkout과 독립적으로 준비한 신뢰 루트를 사용한다. 수신 파일은 root만 쓸 수 있는 별도 경로(예: `/var/lib/lagrange-receive`) 아래에 root 소유로 복사하고, 수신·bundle·출력 상위 디렉터리는 권한 0700으로 준비한다. 실제 배포물과 함께 받은 파일을 새 신뢰 루트로 지정하면 안 된다. [GitHub 오프라인 검증 절차](https://docs.github.com/en/actions/how-tos/secure-your-work/use-artifact-attestations/verify-attestations-offline)

```bash
sudo scripts/ops/verify-release-delivery-vm.py \
  --commit "$COMMIT" --received "$RECEIVED" --build-run "$BUILD_RUN" \
  --build-attempt "$BUILD_ATTEMPT" --trusted-root "$TRUSTED_ROOT" --import-dir "$IMPORT_DIR"
```

게스트는 Python 3.12 이상을 갖춘 전용 Linux/amd64 KVM 또는 QEMU VM이다. 호스팅 빌드는 운영 호스트와 같은 Docker containerd 이미지 저장소를 사용한다. 게스트에서도 같은 저장소를 사용해야 manifest의 로컬 이미지 ID를 대조할 수 있다. Git, Docker, Compose v2, zstd, curl, OpenSSL과 오프라인 증명을 검증할 수 있는 `/usr/bin/gh`를 설치한 뒤 외부 NIC와 호스트 공유 마운트를 분리한다. 첫 반입 전에 Docker 이미지·컨테이너·볼륨 목록이 모두 비어 있어야 한다.

위 명령은 빈 출력 경로를 사용해 전송 목록의 서명·파일 해시를 오프라인에서 확인하고, 같은 릴리스를 두 번 반입하며 자원을 표본 측정한다. 이어 D8 증거와 V3 이미지 ID를 확인하고, 합성 설정으로 설치 계획과 Compose 사전검사를 실행하고, Paper OFF 상태의 DB·API·웹·프록시 기동 및 종료 결과를 확인한다. 게스트의 `GUEST_RESOURCE`에는 메모리·swap·파일시스템별 가용 공간의 시작값·최솟값·종료값이 남는다. 같은 시간대 호스트의 가용 메모리·swap·디스크와 기존 서비스 상태도 대조해 VM을 수일간 유지할 수 있는지 판정한다.

운영 설치기의 `--apply`는 기존 `current`와 DB로 Paper 이력을 확인해야 하므로 빈 VM에서의 합성 기동 증거만으로 운영 설치 인수를 완료할 수 없다. 실제 비공개 원본을 사용하는 R5 네이티브 검증과 운영 배포는 [인계 문서](../handoffs/2026-11-26-github-release-delivery.md)에 정한 후속 단계다.
