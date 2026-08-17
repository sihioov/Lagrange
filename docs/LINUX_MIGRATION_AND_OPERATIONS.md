# Lagrange Station — Windows → Ubuntu 이관 및 Linux 운영 가이드

> **작성 기준일:** 2026-08-14
> **대상:** Ubuntu 24.04 LTS 우선, Ubuntu 계열은 배포판별 차이를 확인하면서 적용
> **인수인계 기준:** `main`의 마지막 확인 커밋 `7eaaedc`
> **canonical remote:** `https://github.com/sihioov/lagrange.git`

이 문서는 현재 Windows 개발·QA 환경을 Ubuntu 계열 Linux 호스트로 옮긴 뒤, 같은 저장소를 안전하게 운영하고 작업을 이어가기 위한 인수인계 문서다. 실제 secret 값, 개인 경로, 계좌 자격증명은 문서·Git·일반 백업에 기록하지 않는다.

## 0. 먼저 읽을 것

이관의 목표는 개발·QA·Paper·추천 러너 운영을 Linux에서 재현하는 것이다. 다음 항목은 이 문서가 자동으로 승인하거나 활성화하지 않는다.

- `live` Compose profile과 KIS 실계좌 주문
- 라이선스가 확인되지 않은 KRX 실데이터 수집
- Auth0 vendor 테스트를 통과시키기 위한 가짜 자격증명
- 게이트 증거의 수동 수정 또는 `BLOCKED_EXTERNAL_*` 판정의 위조

이 시스템은 확인할 수 없으면 거부하는 fail-closed 시스템이다. `docs/STATUS.md`는 시점 스냅샷이므로, 이관 시점의 실제 기준은 항상 다음으로 다시 확인한다.

~~~bash
git fetch origin --prune
git rev-parse origin/main
git status --short --branch
~~~

Windows 원본은 Linux가 실제로 검증될 때까지 삭제하지 않는다. 최소 한 번의 백업·복원 확인과 서비스 health 확인이 끝나기 전에는 원본을 읽기 전용 보관한다.

## 1. 권장 Linux 배치

개발 작업 디렉터리와 systemd가 읽는 배포 디렉터리를 분리하면 작업 중인 파일이 운영 서비스에 바로 노출되지 않는다.

| 영역 | 권장 경로 | 소유·권한 원칙 |
|---|---|---|
| 개발 clone | `~/src/lagrange` | 작업자 계정이 소유 |
| systemd 배포 clone | `/opt/lagrange` | 배포 시 갱신, 서비스 사용자는 읽기 |
| 공유 데이터 | `/var/lib/lagrange/data` | `raw`는 UID/GID `10001:10001`, curated/catalog는 읽기 전용 |
| systemd 설정 | `/etc/lagrange` | `root:root`, 설정 `600` |
| systemd secret 파일 | `/etc/lagrange/secrets` | `root:root`, 파일 `0400` 또는 `0600` |
| Compose secret 파일 | `deploy/secrets/` | Gitignored, 값은 파일 하나당 한 secret |
| 백업 보관 | `/srv/backups/lagrange` 또는 외부 백업 저장소 | 암호화·정책 검증 필수 |
| recommendation 임시 영역 | `/run/lagrange-recommendation-runner` | systemd `RuntimeDirectory`가 생성 |

`deploy/compose/.env`를 사용하는 Compose 개발·운영 경로에서는 `LAGRANGE_DATA_DIR=/var/lib/lagrange/data`, `LAGRANGE_ARTIFACTS_DIR=/var/lib/lagrange/data/artifacts`처럼 절대 경로를 사용한다. Compose의 PostgreSQL volume은 PostgreSQL 18 경로 계약 때문에 `/var/lib/postgresql`에 마운트한다. Windows Docker Desktop의 named volume 디렉터리를 Linux로 파일째 복사하지 않는다.

## 2. Windows 종료 전 동결 절차

### 2.1 코드 상태를 고정한다

PowerShell에서 저장소 루트로 이동해 원격과 로컬 상태를 기록한다.

~~~powershell
Set-Location 'D:\develop\repositories\lagrange'
git fetch origin --prune
git status --short --branch
git rev-parse HEAD
git rev-parse origin/main
git remote get-url origin
git log -10 --oneline --decorate
~~~

정상 인수인계 조건은 다음과 같다.

- `git status --short`가 비어 있다.
- 보존할 변경사항은 모두 commit되어 `origin`에 push되어 있다.
- Linux에서 `origin/main`을 clone하면 필요한 소스와 문서를 얻을 수 있다.

작업 트리가 dirty라면 `git clean`으로 지우지 않는다. 먼저 변경을 정리하거나, 소유자가 명시적으로 보존할 patch·untracked 목록을 만든다.

~~~powershell
$Export = 'E:\lagrange-transfer'
New-Item -ItemType Directory -Force $Export | Out-Null
git diff --binary > "$Export\working-tree.patch"
git diff --cached --binary > "$Export\staged.patch"
git ls-files --others --exclude-standard > "$Export\untracked-files.txt"
~~~

patch와 목록은 secret·credential·대용량 데이터와 같은 위치에 섞지 않는다. 보존할 untracked 파일은 파일별로 검토하고, `.env`, `deploy/secrets` 실파일, 개인 키, `node_modules`, `.venv`, `target`은 이관 목록에서 제외한다.

### 2.2 데이터와 secret을 분류한다

현재 저장소의 Gitignore 정책상 다음은 Git에서 오지 않는다.

- `data/raw/`
- `data/curated/`
- `data/nautilus_catalog/`
- `data/phase0/`
- `target/`, `node_modules/`, `nt/.venv/`
- `deploy/secrets/`의 실값

Windows에서 내용이 아니라 목록과 크기만 기록한다.

~~~powershell
Get-ChildItem data -Directory -Force | Select-Object Name, FullName
Get-ChildItem data -Recurse -File -Force |
  Measure-Object -Property Length -Sum
Get-ChildItem deploy\secrets -File -Force |
  Select-Object Name, Length, LastWriteTime
~~~

`deploy/secrets` 목록은 이름만 인수인계한다. secret 파일 내용이나 명령행에 노출된 password를 transcript에 남기지 않는다.

### 2.3 DB를 옮길지 먼저 판단한다

Windows에서 사용한 WSL2/Compose PostgreSQL이 disposable QA DB뿐이면 Linux에서 `deploy/qa/qa-db.compose.yml`로 새로 만든다. 테스트 DB를 운영 데이터로 간주해 복사하지 않는다.

권위 있는 사용자·Paper 데이터가 실제로 들어 있다면 다음 원칙으로 별도 DB 인수인계를 한다.

1. Docker Desktop/WSL named volume을 직접 복사하지 않는다.
2. PostgreSQL 논리 백업을 만든다. password는 URL·명령행·shell history에 넣지 않고 interactive prompt 또는 `0600` `.pgpass`/secret manager를 사용한다.
3. 역할과 secret은 dump에 포함된 password hash를 복원하지 않고 Linux에서 재생성·재주입한다.
4. 새 호스트의 **격리된 빈 DB**에서 먼저 복원·검증한다.
5. 검증 전에는 운영 DB나 기존 named volume에 `--clean`, `down -v`, `dropdb`를 실행하지 않는다.

예시 형식은 다음과 같다. `<...>`를 실제 값으로 바꿀 때도 password를 문자열에 넣지 않는다.

~~~bash
umask 077
mkdir -p "$HOME/lagrange-transfer"
pg_dump \
  --format=custom \
  --no-owner \
  --no-acl \
  --file "$HOME/lagrange-transfer/lagrange.dump" \
  "postgresql://<db-user>@<db-host>:<db-port>/<db-name>"
~~~

새 DB에서 schema를 어떤 방식으로 만들지 결정해야 한다. **전체 dump를 빈 DB에 복원하는 방식**과 **새 migration을 먼저 적용한 뒤 data-only import하는 방식**을 섞지 않는다. migration-owner/admin이 선택한 방식과 `_sqlx_migrations` 상태를 기록한다. `research-schema-check`는 migration을 실행하는 서비스가 아니라 이미 적용된 schema·role·grant 계약을 검증하는 fail-closed checker다.

## 3. Ubuntu 호스트 준비

### 3.1 OS·시간대·기본 패키지

Ubuntu 24.04 LTS를 권장한다. Ubuntu derivative는 Docker와 systemd 동작, 패키지 이름, codename을 먼저 확인한다.

~~~bash
sudo apt update
sudo apt install -y \
  ca-certificates curl git rsync openssh-client unzip \
  build-essential pkg-config libssl-dev libpq-dev clang lld \
  python3.12 python3.12-venv python3.12-dev postgresql-client

sudo timedatectl set-timezone Asia/Seoul
timedatectl status
~~~

시스템 시간대는 recommendation/research worker의 16:30 KST schedule과 일치해야 한다. UTC를 저장 기준으로 사용하는 DB timestamp와 혼동하지 않는다.

### 3.2 Docker Engine과 Compose v2

Docker는 Docker Desktop이 아니라 Linux Docker Engine + Compose plugin을 기준으로 한다. 공식 apt repository를 사용한다.

~~~bash
sudo install -m 0755 -d /etc/apt/keyrings
sudo curl -fsSL https://download.docker.com/linux/ubuntu/gpg \
  -o /etc/apt/keyrings/docker.asc
sudo chmod a+r /etc/apt/keyrings/docker.asc

sudo tee /etc/apt/sources.list.d/docker.sources >/dev/null <<'EOF'
Types: deb
URIs: https://download.docker.com/linux/ubuntu
Suites: noble
Components: stable
Architectures: amd64
Signed-By: /etc/apt/keyrings/docker.asc
EOF

sudo apt update
sudo apt install -y docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
sudo systemctl enable --now docker

sudo docker version
sudo docker compose version
sudo docker run --rm hello-world
~~~

다른 Ubuntu codename/architecture를 사용하면 위 `Suites`와 `Architectures`를 호스트 값으로 바꾼다. Docker group에 작업자를 추가하면 `sudo` 없이 실행할 수 있지만, Docker daemon 접근은 사실상 root 권한이다. 보안 우선 운영에서는 `sudo docker ...`를 유지하고, 추가했다면 누가 그 권한을 갖는지 기록한다. Docker가 UFW/firewall 규칙을 우회할 수 있으므로 외부 공개 포트는 `deploy/compose/compose.yml`과 `DOCKER-USER` 정책을 함께 검토한다.

공식 참고:

- [Docker Engine on Ubuntu](https://docs.docker.com/engine/install/ubuntu/)
- [Docker Engine installation overview](https://docs.docker.com/engine/install/)

### 3.3 Rust·Python·Node·uv

저장소가 강제하는 버전은 다음과 같다.

| 도구 | 저장소 계약 |
|---|---|
| Rust | `rust-toolchain.toml`의 `1.97.1`, `rustfmt`, `clippy` |
| CPython | `.python-version`의 `3.12` |
| Node | `package.json`의 `>=24 <25` |
| NautilusTrader | `nt/pyproject.toml`의 `nautilus_trader==1.231.0` |
| uv | CI/운영 계약에서 승인한 버전으로 고정하고 `command -v uv` 경로를 기록 |

Rust는 작업자 계정에 설치한다.

~~~bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o /tmp/rustup-init.sh
less /tmp/rustup-init.sh
sh /tmp/rustup-init.sh -y
source "$HOME/.cargo/env"
rustup toolchain install 1.97.1 --profile minimal
rustup component add rustfmt clippy --toolchain 1.97.1
~~~

Node는 `nvm` 또는 조직이 승인한 버전 관리자를 사용한다. `apt install nodejs`가 24.x 범위를 벗어나지 않는지 확인하지 않고 사용하지 않는다.

~~~bash
curl -fsSL https://raw.githubusercontent.com/nvm-sh/nvm/v0.40.6/install.sh \
  -o /tmp/nvm-install.sh
less /tmp/nvm-install.sh
bash /tmp/nvm-install.sh
source "$HOME/.nvm/nvm.sh"
nvm install 24
nvm alias default 24
node --version
npm --version
~~~

uv는 공식 설치 방법 또는 조직 패키지 관리자로 설치한 뒤, repository의 `nt/uv.lock`을 `--locked`로 사용한다. systemd recommendation runner는 `/usr/local/bin/uv`를 명시하므로, 운영 배포 때에는 검증한 실행 파일을 그 경로에 설치한다.

~~~bash
curl -LsSf https://astral.sh/uv/install.sh -o /tmp/uv-install.sh
less /tmp/uv-install.sh
sh /tmp/uv-install.sh
command -v uv
uv --version
~~~

공식 참고:

- [The rustup book](https://rust-lang.github.io/rustup/)
- [nvm installation](https://github.com/nvm-sh/nvm#installing-and-updating)
- [uv installation](https://docs.astral.sh/uv/getting-started/installation/)

## 4. 저장소 clone과 개발 환경 복원

~~~bash
mkdir -p "$HOME/src"
git clone https://github.com/sihioov/lagrange.git "$HOME/src/lagrange"
cd "$HOME/src/lagrange"
git config core.autocrlf input
git fetch origin --prune
git switch main
git pull --ff-only origin main
git status --short --branch
~~~

기준 커밋을 기록한다.

~~~bash
printf 'HEAD       %s\n' "$(git rev-parse HEAD)"
printf 'origin/main %s\n' "$(git rev-parse origin/main)"
git diff --exit-code origin/main --
~~~

의존성을 lockfile 그대로 복원한다.

~~~bash
cd "$HOME/src/lagrange"
uv sync --project nt --locked
npm ci
bash scripts/check-pins.sh --manifest-only
bash scripts/check-pins.sh
bash scripts/validate-foundation.sh
~~~

`npm ci`는 root workspace의 `package-lock.json`을 사용한다. `uv lock`, `npm install`, `cargo update`를 이관 직후에 실행해 lockfile을 조용히 바꾸지 않는다. 의존성 변경이 필요한 작업은 별도 feature branch와 명시적인 commit으로 처리한다.

## 5. 데이터·secret provisioning

### 5.1 디렉터리 생성

`lagrange` systemd 계정이 아직 없다면 운영자가 먼저 생성한다. 서비스 계정으로 개발 shell을 열거나 secret을 읽는 용도로 재사용하지 않는다.

~~~bash
if ! getent group lagrange >/dev/null; then
  sudo groupadd --system lagrange
fi
if ! getent passwd lagrange >/dev/null; then
  sudo useradd --system --gid lagrange --home-dir /nonexistent \
    --shell /usr/sbin/nologin lagrange
fi
~~~

~~~bash
# First inspect the idempotent plan. No host mutation happens in this mode.
scripts/ops/provision-linux.sh --dry-run

# After reviewing the exact paths, apply only the account/directory ownership
# fence. The script never deletes, truncates, recursively copies, or generates
# secrets. Run this explicitly as root, then verify without mutation.
sudo scripts/ops/provision-linux.sh --apply
scripts/ops/provision-linux.sh --preflight
~~~

계정을 먼저 만든 뒤 `phase0` 디렉터리를 다시 소유시킨다.

~~~bash
sudo chown -R lagrange:lagrange /var/lib/lagrange/data/phase0
~~~

### 5.2 파일 데이터 이관

공인된 transfer 경로에서 Linux data root로 복사한다. Windows에서 POSIX 권한을 보존할 수 없으므로, 복사 후 Linux 권한을 Compose 계약에 맞춰 다시 설정한다.

~~~bash
sudo rsync -aH --info=progress2 \
  <transfer-root>/raw/ /var/lib/lagrange/data/raw/
sudo rsync -aH --info=progress2 \
  <transfer-root>/curated/ /var/lib/lagrange/data/curated/
sudo rsync -aH --info=progress2 \
  <transfer-root>/nautilus_catalog/ /var/lib/lagrange/data/nautilus_catalog/
sudo rsync -aH --info=progress2 \
  <transfer-root>/artifacts/ /var/lib/lagrange/data/artifacts/
~~~

Raw tree는 연구 worker UID/GID와 mode를 맞춘다. 아래 명령은 symlink를 따라가지 않는 Compose initializer와 같은 방향의 수동 복구다.

~~~bash
sudo find /var/lib/lagrange/data/raw -xdev -type l -print
sudo find /var/lib/lagrange/data/raw -xdev -type d -exec chown 10001:10001 {} +
sudo find /var/lib/lagrange/data/raw -xdev -type d -exec chmod 0750 {} +
sudo find /var/lib/lagrange/data/raw -xdev -type f -exec chown 10001:10001 {} +
sudo find /var/lib/lagrange/data/raw -xdev -type f \
  \( -name manifest.jsonl -o -name commit.lock \) -exec chmod 0640 {} +
sudo find /var/lib/lagrange/data/raw -xdev -type f \
  ! \( -name manifest.jsonl -o -name commit.lock \) -exec chmod 0440 {} +
~~~

symlink 출력이 있으면 진행을 멈추고 원본·대상 경로를 조사한다. Raw init은 외부 filesystem을 건너가거나 symlink를 따라가면 안 된다.

curated와 universe manifest는 worker가 쓰지 못해야 한다. 실제 runtime UID가 읽을 수 있는지 확인한 뒤 bind mount도 `:ro`인지 확인한다.

### 5.3 Compose 설정과 secret

Compose 설정에는 secret 값을 넣지 않는다.

~~~bash
cd "$HOME/src/lagrange"
cp deploy/compose/.env.example deploy/compose/.env
chmod 600 deploy/compose/.env
$EDITOR deploy/compose/.env
~~~

최소한 다음을 Linux 절대 경로에 맞춘다.

~~~dotenv
LAGRANGE_DATA_DIR=/var/lib/lagrange/data
LAGRANGE_ARTIFACTS_DIR=/var/lib/lagrange/data/artifacts
POSTGRES_USER=lagrange
POSTGRES_DB=lagrange
AUTH0_DOMAIN=lagrange-station.jp.auth0.com
AUTH0_CLIENT_ID=<deployment-config>
~~~

`RECOMMENDATION_DATASET_VERSION_ID`, `RECOMMENDATION_DATASET_VERSION`, `RECOMMENDATION_CURATED_VERSION`, `RECOMMENDATION_DATASET_MANIFEST_SHA256`는 승인된 immutable curated dataset을 실제로 설치한 뒤에만 채운다. 값이 없거나 dataset과 다르면 production recommendation은 거부되어야 한다. 운영에서는 `scripts/ops/validate-production-config.sh`가 이 pin, KIS entitlement reference, TLS/Auth0/DB/KIS read-only secret 파일과 live-off 조건을 함께 검사한다.

Compose 경로는 `deploy/secrets/`를 참조한다. 다음 이름은 파일명 inventory일 뿐 실제 값을 문서에 복사하지 않는다.

~~~text
postgres_password
db_app_password
db_worker_password
db_research_password
db_audit_password
session_secret
csrf_secret
auth0_client_secret
kis_app_key
kis_app_secret
kis_account_ref
tls/lagrange.crt
tls/lagrange.key
~~~

secret은 공급자·Secret Manager에서 Linux에 재발급하거나 암호화된 전용 전송으로 주입한다. 일반 `rsync`, GitHub artifact, DB dump, ordinary backup archive에 포함하지 않는다.

~~~bash
sudo install -d -m 0700 -o root -g root /etc/lagrange/secrets
# secret manager 또는 승인된 전송 절차로 한 파일씩 배치한다.
sudo install -o root -g root -m 0400 <secure-source>/auth0_client_secret \
  /etc/lagrange/secrets/auth0_client_secret
~~~

Compose를 사용할 때는 `deploy/secrets/<name>`의 실제 파일도 별도로 provision하고 `chmod 600` 이상으로 제한한다. `*.example`을 그대로 production secret으로 사용하지 않는다. `git status --short`와 `git ls-files -- deploy/secrets`로 실값이 tracked되지 않았는지 확인한다.

## 6. DB 복원과 migration 순서

이 단계는 source DB가 실제 권위 데이터를 가지고 있을 때만 수행한다. disposable QA DB라면 이 절을 건너뛰고 QA Compose를 새로 만든다.

### 6.1 새 PostgreSQL을 격리해 시작

먼저 대상 PostgreSQL만 시작하고, 외부 서비스·runner는 시작하지 않는다. 전체
운영 Compose의 build/migration/health 순서는 `scripts/ops/compose-release.sh`가
계획·preflight 후 동일하게 수행한다.

~~~bash
cd "$HOME/src/lagrange"
docker compose --env-file deploy/compose/.env \
  -f deploy/compose/compose.yml up -d postgres
docker compose --env-file deploy/compose/.env \
  -f deploy/compose/compose.yml ps postgres
~~~

### 6.2 역할과 schema

역할은 secret 파일과 별개로 migration-owner/admin 절차에서 만든다. `postgres_password`를 일반 worker/app password 대신 재사용하지 않는다. 최소한 migration·application·worker·research·audit 역할의 소유권과 RLS grant를 실제 migration 계약과 대조한다.

새 빈 DB를 migration으로 만드는 경우에는 저장소의 migration 절차를 사용하고, 동시 migration에는 유한한 lock timeout을 준다.

~~~bash
PGOPTIONS='-c lock_timeout=5s' sqlx migrate run
~~~

`sqlx` 명령이 현재 호스트에 없으면 임의의 순서로 `.sql` 파일을 실행하지 말고, 승인된 migration-owner 실행 방법을 먼저 준비한다. `research-schema-check`는 migration을 적용하지 않는다.

### 6.3 데이터 복원

전체 custom dump를 빈 DB에 복원하는 경우:

~~~bash
pg_restore \
  --no-owner \
  --no-acl \
  --dbname 'postgresql://<db-user>@<db-host>:<db-port>/<db-name>' \
  "$HOME/lagrange-transfer/lagrange.dump"
~~~

이 명령은 빈 격리 DB를 대상으로 한 형식이다. 공유·운영 DB에 `--clean`을 추가하거나 기존 schema를 지우는 작업은 별도 승인 없이는 하지 않는다. data-only dump를 사용했다면 schema migration을 먼저 완료한 뒤 import하고, 두 경로를 혼합하지 않는다.

복원 후 checker와 role 계약을 확인한다.

~~~bash
cd "$HOME/src/lagrange"
docker compose --env-file deploy/compose/.env \
  -f deploy/compose/compose.yml run --rm research-schema-check
~~~

checker가 실패하면 worker·recommendation runner를 시작하지 않는다. migration, role, grant, RLS, append-only contract의 원인을 먼저 고친다.

## 7. Compose 검증과 서비스 기동

### 7.1 항상 먼저 정적 검증

~~~bash
cd "$HOME/src/lagrange"
docker compose --env-file deploy/compose/.env \
  -f deploy/compose/compose.yml config --quiet
bash scripts/check-pins.sh
bash scripts/validate-foundation.sh
bash scripts/qa/research-worker-smoke.sh --static-only
~~~

`config --quiet`가 secret 파일 누락으로 실패하면 정상적인 fail-closed 반응일 수 있다. 값을 Compose YAML이나 shell argument에 넣어 우회하지 말고 누락된 secret을 전용 provisioning 절차로 보완한다.

### 7.2 Compose의 현재 범위를 이해한다

현재 `deploy/compose/compose.yml`은 reverse proxy, Web, API, PostgreSQL,
migration/role bootstrap, KIS research/recommendation/candidate/backtest/Paper worker의
실제 production entrypoint와 healthcheck를 정의한다. `report-worker`는 처리 계약과
producer가 없어 sleeping placeholder 대신 의도적으로 배포하지 않으며, Live profile은
credential-free simulator라 실주문 readiness를 제공하지 않는다. Compose가 올라갔다는
사실만으로 실제 KIS entitlement, 초기 dataset 승인, 또는 KIS 실거래가 준비됐다고
판단하지 않는다. 계좌·주문 secret은 이 read-only EOD 경로의 필수 조건이 아니다.

실제 runner 운영은 아래 systemd unit을 단일 owner로 사용한다. 같은 queue에 Compose recommendation runner와 systemd recommendation runner를 동시에 띄우지 않는다. Paper runner도 동일하다.

### 7.3 systemd runner 설치

개발 clone에서 빌드하고, service가 읽는 배포 clone·binary·설정을 명시적으로 갱신한다.

~~~bash
cd "$HOME/src/lagrange"
cargo build --locked --release -p api-server --bin paper-runner
cargo build --locked --release -p job-queue --bin recommendation-runner

sudo install -d -m 0755 /opt/lagrange/bin
sudo rsync -a \
  --exclude '.git/' --exclude '.omo/' --exclude '.worktrees/' \
  --exclude 'target/' --exclude 'node_modules/' \
  --exclude '.venv/' --exclude 'nt/.venv/' \
  --exclude 'deploy/secrets/' --exclude 'deploy/compose/.env' \
  --exclude 'data/raw/' --exclude 'data/curated/' \
  --exclude 'data/nautilus_catalog/' --exclude 'data/phase0/' \
  --exclude 'data/artifacts/' \
  "$HOME/src/lagrange/" /opt/lagrange/
sudo install -m 0755 target/release/paper-runner \
  /usr/local/bin/paper-runner-bin
sudo install -m 0755 deploy/runtime/paper-runner-entrypoint \
  /opt/lagrange/bin/paper-runner
sudo install -m 0755 target/release/recommendation-runner \
  /opt/lagrange/bin/recommendation-runner
sudo install -m 0755 "$(command -v uv)" /usr/local/bin/uv
~~~

recommendation child는 `/opt/lagrange/nt/pyproject.toml`, `nt/uv.lock`, `nt/strategies`를 읽고 `/usr/local/bin/uv`로 `uv run --project nt --no-sync`를 실행한다. `/opt/lagrange/nt`에서 먼저 lockfile sync를 완료하고 `.venv`가 service 사용자에게 읽히는지 확인한다.

~~~bash
sudo install -d -m 0750 -o lagrange -g lagrange /var/cache/lagrange/uv
sudo chown -R lagrange:lagrange /opt/lagrange
cd /opt/lagrange
sudo -u lagrange env HOME=/var/lib/lagrange \
  UV_CACHE_DIR=/var/cache/lagrange/uv \
  /usr/local/bin/uv sync --project nt --locked
sudo -u lagrange test -r /opt/lagrange/nt/uv.lock
sudo -u lagrange test -x /usr/local/bin/uv
sudo chown -R root:root /opt/lagrange
~~~

위 `rsync`는 `.git`, build/cache, host `.venv`, `deploy/secrets`, Compose `.env`, 그리고 대용량 data tree를 배포 clone으로 복사하지 않는다. 운영 배포 전에 이 exclude를 그대로 적용하고, `uv sync`로 `/opt/lagrange/nt/.venv`를 Linux에서 새로 만든다. systemd runner는 host-reachable PostgreSQL URL을 사용한다. 현재 Compose의 PostgreSQL은 internal-only이고 host port를 publish하지 않으므로, Compose DB를 systemd service에 연결하려고 5432를 무심코 공개하지 않는다. host PostgreSQL/external DB를 사용하거나 runner를 Compose 안에서 단일 owner로 실행하는 둘 중 하나를 선택한다.

`paper-runner.env.example`을 `/etc/lagrange/paper-runner.env`로 복사하고, 네 역할별 DB URL과 `LAGRANGE_DATASET_ROOT`를 운영 secret manager에서 주입한다. URL에 실제 password를 문서·명령행·journal에 남기지 않는다. recommendation runner 설정에는 `APP_ENV=production`, DB 연결 정보, worker `DB_PASSWORD_FILE`, 다섯 개의 immutable `RECOMMENDATION_DATASET_*` pin을 넣고, password 값 자체는 넣지 않는다.

~~~bash
sudo install -m 0600 -o root -g root \
  deploy/systemd/paper-runner.env.example /etc/lagrange/paper-runner.env
sudo install -m 0600 -o root -g root \
  deploy/systemd/recommendation-runner.env.example \
  /etc/lagrange/recommendation-runner.env
sudoedit /etc/lagrange/paper-runner.env
sudoedit /etc/lagrange/recommendation-runner.env
sudo install -m 0444 -o root -g root \
  configs/universes/kr-etf-core-v1.yaml \
  /etc/lagrange/universes/kr-etf-core-v1.yaml
~~~

`/etc/lagrange/recommendation-runner.env`의 빈 dataset pin은 승인된 immutable curated dataset 값으로 모두 채운다. 필요한 핵심 값은 `APP_ENV`, `DB_HOST`, `DB_PORT`, `DB_NAME`, `DB_USER=worker`, `DB_PASSWORD_FILE`, 그리고 다섯 개 `RECOMMENDATION_DATASET_*` pin이다. service unit이 health path와 runtime directory를 소유한다.

unit을 설치하고, DB·data·secret 검증이 끝난 뒤에만 활성화한다.

~~~bash
sudo install -m 0644 deploy/systemd/paper-runner.service \
  /etc/systemd/system/paper-runner.service
sudo install -m 0644 deploy/systemd/lagrange-recommendation-runner.service \
  /etc/systemd/system/lagrange-recommendation-runner.service
sudo systemctl daemon-reload

sudo systemctl enable paper-runner.service \
  lagrange-recommendation-runner.service
sudo systemctl start paper-runner.service
sudo systemctl start lagrange-recommendation-runner.service
~~~

상태와 로그를 확인한다.

~~~bash
sudo systemctl status paper-runner.service --no-pager
sudo systemctl status lagrange-recommendation-runner.service --no-pager
sudo systemctl is-active paper-runner.service
sudo systemctl is-active lagrange-recommendation-runner.service
sudo journalctl -u paper-runner.service -n 100 --no-pager
sudo journalctl -u lagrange-recommendation-runner.service -n 100 --no-pager
~~~

recommendation runner는 기본 16:30 KST cycle과 시작 시 eligible close catch-up을 사용한다. 자동 Paper 적용은 `auto_apply_recommendations=true`인 활성 binding에만 해당한다. health가 stale이거나 DB/data pin이 불명확하면 runner를 재시작해 통과시키지 말고 원인을 조사한다.

## 8. 이관 완료 검증

### 8.1 개발·정적 검증

~~~bash
cd "$HOME/src/lagrange"
git status --short --branch
bash scripts/check-pins.sh
bash scripts/validate-foundation.sh
docker version --format '{{.Server.Version}}'
docker compose version
~~~

개발·QA용 disposable DB를 사용할 때만 다음처럼 실행한다.

먼저 Phase 0 데이터를 생성하고, 테스트가 부를 Python 인터프리터를 정한다. 추천 계산 경로의 test binary 3개(`http_recommendations`, `recommendation_compute`, `recommendation_runner`)는 `scripts/ci/prepare_phase0.py`를 자식 프로세스로 실행하며 인터프리터를 `PYTHON` 또는 기본값 `python`에서 찾는다. **그 인터프리터에 pyarrow가 없으면 테스트 14개가 실패하는데, 오류가 Python traceback으로 나오기 때문에 Rust 회귀로 오독하기 쉽다.**

~~~bash
python -m pip install --disable-pip-version-check pyarrow==25.0.0
python scripts/ci/prepare_phase0.py --root data/phase0

docker compose -p lagrange-qa \
  -f deploy/qa/qa-db.compose.yml up -d --wait qa-db
DATABASE_URL='postgres://postgres:lagrange@127.0.0.1:55432/postgres' \
  cargo test --workspace --locked --no-fail-fast
docker compose -p lagrange-qa \
  -f deploy/qa/qa-db.compose.yml down -v --remove-orphans
~~~

`python`을 오염시키지 않으려면 별도 venv를 만들고 `PYTHON=<그 venv의 python>`을 export한다. 어느 쪽이든 `.github/workflows/ci.yml`의 `workspace-tests`와 같은 순서다 — CI는 `setup-python`이 제공하는 `python`에 pyarrow를 설치한 뒤 생성기를 돌린다.

위 QA DB는 disposable이며 운영 DB가 아니다. 장시간 테스트를 동시에 여러 개 실행해 QA tmpfs를 가득 채우지 않는다. 이 절차로 2026-08-14에 1,371개 통과·실패 0을 확인했다(`docs/STATUS.md` §2.7).

### 8.2 배포 smoke

실제 DB/secret을 사용하는 functional smoke와 static smoke를 구분한다.

~~~bash
# Raw/Compose 권한·digest·schema 계약만 확인
bash scripts/qa/research-worker-smoke.sh --static-only
bash scripts/qa/recommendation-runner-smoke.sh --static-only

# 별도 QA DB를 사용하는 실제 runner smoke
bash scripts/qa/recommendation-runner-smoke.sh
bash scripts/qa/paper-runner-smoke.sh
~~~

`scripts/qa/phase1-gate.sh`는 native Linux용으로 이식됐다. 호출자의 Cargo와
`LAGRANGE_QA_DB_PORT`를 사용하며, E2~E5는 cargo exit 0뿐 아니라 실제 실행된
test 수가 1개 이상이어야 PASS다. 실행 전에 별도 QA DB를 준비하고 실 Auth0
tenant 환경변수를 명시적으로 주입한다. E7은 저장소 루트의 npm workspace에서
의존성을 해석하므로 루트에서 `npm ci`를 실행하고 Chromium을 설치해야 한다.

Phase 1 증거와 이후 Phase 2/3/F3 증거는 같은 commit과 실행 환경에 고정해
발행한다. 환경 오류(exit 2), 누락된 evidence, 또는 다른 worktree의 포트 응답을
외부 blocker나 코드 합격으로 오해하지 않는다.

### 8.3 데이터·service 검증

- `/var/lib/lagrange/data/raw`에 symlink 또는 외부 filesystem crossing이 없다.
- Raw immutable evidence가 `0440`, manifest/lock이 `0640`, directory가 `0750`이다.
- curated·universe manifest는 service에서 read-only다.
- `research-schema-check`가 성공했다.
- `paper-runner`와 `recommendation-runner`가 동시에 두 개 실행되지 않는다.
- systemd journal에 password, Auth0 client secret, KRX/KIS credential이 없다.
- recommendation health state, queue age, last schedule attempt를 확인했다.
- Windows 원본과 Linux DB/data의 checksum·row count 차이를 기록했다.

## 9. Linux에서의 일상 작업 관리

### 작업 시작

~~~bash
cd "$HOME/src/lagrange"
git fetch origin --prune
git status --short --branch
git switch main
git pull --ff-only origin main
git switch -c feat/<short-topic>
~~~

작업 중에는 `main`을 직접 편집하지 않는다. 하나의 feature branch에는 하나의 주제를 넣고, lockfile·migration·운영 설정 변경은 commit 메시지에 명시한다.

### 작업 종료 전

~~~bash
git diff --check
cargo fmt --all -- --check
bash scripts/check-pins.sh --manifest-only
npm run lint
npm run typecheck
git status --short
~~~

검증이 끝난 변경만 commit·push한다.

~~~bash
git add <reviewed-files>
git commit -m '<type>: <short description>'
git push -u origin HEAD
~~~

`main` 반영은 fast-forward 가능 여부 또는 PR을 확인한 뒤 수행한다. force push와 `git reset --hard`는 작업자가 명시적으로 승인하지 않는 한 사용하지 않는다.

### 운영 상태 확인

~~~bash
sudo systemctl is-active paper-runner.service
sudo systemctl is-active lagrange-recommendation-runner.service
sudo journalctl -u paper-runner.service --since '1 hour ago' --no-pager
sudo journalctl -u lagrange-recommendation-runner.service --since '1 hour ago' --no-pager

cd "$HOME/src/lagrange"
docker compose --env-file deploy/compose/.env \
  -f deploy/compose/compose.yml ps
docker compose --env-file deploy/compose/.env \
  -f deploy/compose/compose.yml logs --tail=200 research-worker recommendation-runner
~~~

runner를 재시작하는 것은 queue를 없애거나 stale job을 승인하는 행위가 아니다. 재시작 전후에 lease, blocked run, health state, DB 연결을 확인한다. `docker compose down -v`는 disposable QA project 외에는 사용하지 않는다.

## 10. 백업·복원·secret recovery

일반 backup archive에는 DB·Raw·curated·artifact만 들어갈 수 있고 secret은 들어가지 않는다. secret은 archive 복원이 아니라 provider 재발급·rotation·runtime injection으로 복구한다.

~~~bash
# 먼저 정책 검증. exit 0이 아니면 복원 명령을 시작하지 않는다.
bash scripts/backup/validate-policy.sh \
  --set <backup-set-dir> --gate default

# 복원은 항상 격리 target에서 검증한다. key는 argument 대신 --key-file을 사용한다.
bash scripts/backup/restore-and-verify.sh \
  --set <backup-set-dir> \
  --sidecar <backup-sidecar.json> \
  --key-file <root-owned-0600-key-file> \
  --verdict <restore-verdict.json>
~~~

`pre-member-restore-drill.md`, `pre-live-reconcile-restore.md`, `pitr-point-in-time-recovery.md`, `secret-recovery.md`의 순서를 따른다. backup validator가 secret marker를 발견하면 archive를 복원하지 말고 quarantine·rotation·incident 기록을 수행한다.

## 11. 현재 작업의 다음 순서

Linux 이관 직후에는 아래 순서로 진행한다.

1. `origin/main` clone과 toolchain pin 확인
2. disposable QA DB와 정적 smoke로 Linux 경로 확인
3. 권위 DB/data가 있다면 격리 복원과 checksum/row count 대조
4. Paper/recommendation systemd runner를 한 owner로만 활성화
5. E7 Playwright 포함 전체 gate 재실행, 08-10보다 새로운 evidence 발행
6. Auth0 실제 vendor suite 실행 후 Phase 1 E2 evidence 갱신
7. 리밸런싱 미리보기 UI를 구현
8. KRX 권리·provider·credential·endpoint와 KIS 실계좌는 별도 운영자/소유자 승인 후 진행

현재 코드의 고정 11-ETF recommendation pipeline과 Paper rebalance preview는 `main`에 들어가 있지만, production KIS feed의 실제 credential·entitlement·초기 백필/승인 dataset과 KIS 실계좌는 여전히 외부 blocker다. read-only EOD release에는 실계좌가 필요하지 않으며, Linux로 옮겼다는 이유만으로 이 경계가 사라지지 않는다. 고정 ETF 백필 계획은 `docs/runbooks/kis-production-backfill.md`를 따른다.

## 12. 완료 체크리스트

### 원본 보존

- [ ] Windows `git status`가 clean이거나 patch/untracked 목록을 별도로 보존했다.
- [ ] `origin/main`의 commit hash를 기록했다.
- [ ] DB가 disposable인지 권위 데이터인지 판정했다.
- [ ] Raw·curated·catalog·artifact의 크기와 checksum을 기록했다.
- [ ] secret은 별도 secret manager/암호화 전송 경로로만 이관했다.

### Linux 준비

- [ ] Ubuntu version, architecture, timezone을 기록했다.
- [ ] Docker Server와 Compose plugin이 동작한다.
- [ ] Rust 1.97.1, Python 3.12, Node 24.x, uv, NautilusTrader lock이 pin과 일치한다.
- [ ] `/opt/lagrange`, `/var/lib/lagrange/data`, `/etc/lagrange` 권한을 확인했다.
- [ ] `deploy/secrets` 실파일이 Git에 들어가지 않았다.

### 복원·검증

- [ ] DB 복원 또는 새 QA DB 생성 절차를 분리해 완료했다.
- [ ] migration-owner/schema/role/checker 계약이 통과했다.
- [ ] Raw mode·UID·symlink 검사를 통과했다.
- [ ] `scripts/check-pins.sh`, foundation, research static smoke가 통과했다.
- [ ] recommendation/paper functional smoke 또는 CI 결과를 commit과 함께 기록했다.
- [ ] systemd journal과 health state에 secret 노출이 없다.
- [ ] Windows 원본을 읽기 전용으로 유지한 채 Linux가 실제 작업을 대신할 수 있다.

이 체크리스트가 끝나기 전에는 Windows 환경과 기존 DB/data를 삭제하지 않는다.
