# Auth0 Confidential Client Wiring Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Authenticate the Rust/Axum Auth0 authorization-code exchange with the Japan tenant's client secret while retaining PKCE and preventing secret disclosure.

**Architecture:** A focused `config` module loads the secret exclusively from `AUTH0_CLIENT_SECRET_FILE` into zeroizing memory. `HttpOidcTransport` owns that secret and sends it only as `client_secret` in the TLS token request; the provider-neutral OIDC request remains secret-free. Compose exposes the mounted secret path, while tracked environment examples retain placeholders and the real local values remain ignored.

**Tech Stack:** Rust 1.97.1, Axum 0.8, Reqwest 0.12, Tokio, `zeroize`, Docker Compose, Auth0 OIDC Authorization Code + PKCE S256.

---

## File Map

- Create `apps/api-server/auth/src/config.rs`: load and own the Auth0 client secret without exposing it through errors or `Debug`.
- Create `apps/api-server/auth/tests/http_oidc_transport.rs`: black-box token endpoint and secret-file contract tests.
- Modify `apps/api-server/auth/src/lib.rs`: export the config module, inject the secret into `HttpOidcTransport`, submit it to Auth0, and suppress untrusted token error bodies.
- Modify `apps/api-server/auth/Cargo.toml`: add direct `zeroize`, test filesystem, and local-server features.
- Modify `deploy/compose/compose.yml`: tell the API server where Docker mounted the Auth0 secret.
- Modify `deploy/compose/.env.example`: keep non-secret Auth0 interpolation variables documented.
- Modify `deploy/secrets/README.md`: document provisioning and the `_FILE` boundary.
- Modify `docs/decisions/0002-auth0-oidc-and-sessions.md`: correct the PKCE/client-authentication decision.
- Create ignored `deploy/compose/.env`: select the operator-provided Japan tenant and Client ID locally.

### Task 1: Secret file boundary

**Files:**
- Create: `apps/api-server/auth/src/config.rs`
- Create: `apps/api-server/auth/tests/http_oidc_transport.rs`
- Modify: `apps/api-server/auth/src/lib.rs:18`
- Modify: `apps/api-server/auth/Cargo.toml`

- [ ] **Step 1: Add the test-only dependency and write failing secret-file tests**

Add this dev dependency:

```toml
[dev-dependencies]
tower = { version = "0.5", features = ["util"] }
http-body-util = "0.1"
tempfile = "3"
```

Create `apps/api-server/auth/tests/http_oidc_transport.rs`:

```rust
use api_server_auth::config::{AUTH0_CLIENT_SECRET_FILE, ClientSecret};
use std::fs;

#[test]
fn client_secret_file_rejects_missing_empty_and_non_file_inputs_without_values() {
    let root = tempfile::tempdir().expect("temporary secret root");
    let marker = "auth0-secret-must-never-render";

    let missing = ClientSecret::from_file(root.path().join("missing"))
        .expect_err("missing secret must fail")
        .to_string();
    assert!(missing.contains(AUTH0_CLIENT_SECRET_FILE));
    assert!(!missing.contains(marker));

    let empty_path = root.path().join("empty");
    fs::write(&empty_path, "\r\n").expect("write empty secret fixture");
    let empty = ClientSecret::from_file(&empty_path)
        .expect_err("empty secret must fail")
        .to_string();
    assert!(empty.contains(AUTH0_CLIENT_SECRET_FILE));
    assert!(!empty.contains(marker));

    let directory = ClientSecret::from_file(root.path())
        .expect_err("directory is not a secret file")
        .to_string();
    assert!(directory.contains(AUTH0_CLIENT_SECRET_FILE));
    assert!(!directory.contains(marker));
}

```

Do not derive or implement `Debug` for `ClientSecret`. The hostile transport
response test in Task 3 directly proves rendered errors do not disclose its
value.

- [ ] **Step 2: Run the test and verify RED**

Run:

```powershell
cargo test -p api-server-auth --test http_oidc_transport --no-fail-fast
```

Expected: compilation fails because `api_server_auth::config` does not exist.

- [ ] **Step 3: Implement the minimal zeroizing file loader**

Add the direct dependency:

```toml
[dependencies]
zeroize = "1.8"
```

Export the module near the imports in `src/lib.rs`:

```rust
pub mod config;
```

Create `apps/api-server/auth/src/config.rs`:

```rust
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

pub const AUTH0_CLIENT_SECRET_FILE: &str = "AUTH0_CLIENT_SECRET_FILE";

pub struct ClientSecret {
    value: Zeroizing<String>,
}

impl ClientSecret {
    pub fn from_env() -> Result<Self, ClientSecretError> {
        let path = env::var_os(AUTH0_CLIENT_SECRET_FILE)
            .ok_or(ClientSecretError::MissingPath)?;
        Self::from_file(path)
    }

    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, ClientSecretError> {
        let path = path.as_ref();
        let mut value = fs::read_to_string(path).map_err(|source| ClientSecretError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        if value.ends_with("\r\n") {
            value.truncate(value.len() - 2);
        } else if value.ends_with('\n') {
            value.pop();
        }
        if value.trim().is_empty() {
            return Err(ClientSecretError::Empty {
                path: path.to_path_buf(),
            });
        }
        Ok(Self {
            value: Zeroizing::new(value),
        })
    }

    pub(crate) fn expose(&self) -> &str {
        self.value.as_str()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientSecretError {
    #[error("{AUTH0_CLIENT_SECRET_FILE} is required")]
    MissingPath,
    #[error("{AUTH0_CLIENT_SECRET_FILE} cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{AUTH0_CLIENT_SECRET_FILE} contains an empty secret at {path}")]
    Empty { path: PathBuf },
}
```

- [ ] **Step 4: Run the test and verify GREEN**

Run:

```powershell
cargo test -p api-server-auth --test http_oidc_transport --no-fail-fast
```

Expected: the secret-file test passes.

- [ ] **Step 5: Commit the secret boundary**

```powershell
git add apps/api-server/auth/Cargo.toml apps/api-server/auth/src/config.rs apps/api-server/auth/src/lib.rs apps/api-server/auth/tests/http_oidc_transport.rs Cargo.lock
git commit -m "feat(auth): load Auth0 client secret from file"
```

### Task 2: Authenticated token exchange

**Files:**
- Modify: `apps/api-server/auth/tests/http_oidc_transport.rs`
- Modify: `apps/api-server/auth/src/lib.rs:388-436`
- Modify: `apps/api-server/auth/Cargo.toml`

- [ ] **Step 1: Enable the local test server and write the failing form test**

Expand the existing Tokio dependency features:

```toml
tokio = { version = "1", features = ["rt", "rt-multi-thread", "macros", "net"] }
```

Append these imports and helpers to the transport test:

```rust
use auth::oidc::{OidcTransport, TokenRequest};
use axum::extract::Form;
use axum::http::StatusCode;
use axum::routing::post;
use axum::Router;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

async fn token_server(
    status: StatusCode,
    response_body: &'static str,
) -> (String, Arc<Mutex<Option<HashMap<String, String>>>>) {
    let received = Arc::new(Mutex::new(None));
    let captured = received.clone();
    let app = Router::new().route(
        "/oauth/token",
        post(move |Form(form): Form<HashMap<String, String>>| {
            let captured = captured.clone();
            async move {
                *captured.lock().expect("capture lock") = Some(form);
                (status, response_body)
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind token server");
    let address = listener.local_addr().expect("token server address");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve token endpoint");
    });
    (format!("http://{address}/oauth/token"), received)
}

fn token_request() -> TokenRequest {
    TokenRequest {
        code: "authorization-code".to_owned(),
        redirect_uri: "https://app.lagrange.local/auth/callback".to_owned(),
        client_id: "lagrange-client".to_owned(),
        code_verifier: "pkce-verifier".to_owned(),
    }
}
```

Append the test:

```rust
#[tokio::test]
async fn token_exchange_posts_client_secret_and_pkce_verifier() {
    let root = tempfile::tempdir().expect("temporary secret root");
    let secret_path = root.path().join("auth0-client-secret");
    fs::write(&secret_path, "confidential-value\r\n").expect("write secret fixture");
    let secret = ClientSecret::from_file(&secret_path).expect("load client secret");
    let (token_url, received) = token_server(
        StatusCode::OK,
        r#"{"id_token":"header.payload.signature"}"#,
    )
    .await;
    let transport = api_server_auth::HttpOidcTransport::new(
        token_url,
        "http://127.0.0.1/unused-jwks",
        secret,
    );

    transport
        .exchange_code(&token_request())
        .await
        .expect("token exchange succeeds");

    let form = received
        .lock()
        .expect("capture lock")
        .take()
        .expect("token form captured");
    assert_eq!(form.get("grant_type").map(String::as_str), Some("authorization_code"));
    assert_eq!(form.get("client_secret").map(String::as_str), Some("confidential-value"));
    assert_eq!(form.get("code_verifier").map(String::as_str), Some("pkce-verifier"));
    assert_eq!(form.get("redirect_uri").map(String::as_str), Some("https://app.lagrange.local/auth/callback"));
}
```

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```powershell
cargo test -p api-server-auth --test http_oidc_transport token_exchange_posts_client_secret_and_pkce_verifier -- --exact
```

Expected: compilation fails because `HttpOidcTransport::new` accepts only two
arguments and the transport owns no client secret.

- [ ] **Step 3: Inject and submit the client secret**

Replace the transport definition and constructor with:

```rust
/// Production OIDC transport: authenticated token exchange + JWKS fetch.
/// PKCE S256 protects the authorization code while the client secret
/// authenticates this confidential server-side application.
pub struct HttpOidcTransport {
    pub token_url: String,
    pub jwks_url: String,
    client_secret: config::ClientSecret,
    client: reqwest::Client,
}

impl HttpOidcTransport {
    pub fn new(
        token_url: impl Into<String>,
        jwks_url: impl Into<String>,
        client_secret: config::ClientSecret,
    ) -> Self {
        Self {
            token_url: token_url.into(),
            jwks_url: jwks_url.into(),
            client_secret,
            client: reqwest::Client::builder()
                .user_agent("lagrange-station-api-server")
                .build()
                .expect("reqwest client builds"),
        }
    }
}
```

Replace the token form with:

```rust
.form(&[
    ("grant_type", "authorization_code"),
    ("code", request.code.as_str()),
    ("redirect_uri", request.redirect_uri.as_str()),
    ("client_id", request.client_id.as_str()),
    ("client_secret", self.client_secret.expose()),
    ("code_verifier", request.code_verifier.as_str()),
])
```

- [ ] **Step 4: Run the focused test and verify GREEN**

Run:

```powershell
cargo test -p api-server-auth --test http_oidc_transport token_exchange_posts_client_secret_and_pkce_verifier -- --exact
```

Expected: one test passes and the captured form contains both confidential
client authentication and PKCE.

- [ ] **Step 5: Commit authenticated exchange**

```powershell
git add apps/api-server/auth/Cargo.toml apps/api-server/auth/src/lib.rs apps/api-server/auth/tests/http_oidc_transport.rs Cargo.lock
git commit -m "fix(auth): authenticate Auth0 token exchange"
```

### Task 3: Reflected-secret error containment

**Files:**
- Modify: `apps/api-server/auth/tests/http_oidc_transport.rs`
- Modify: `apps/api-server/auth/src/lib.rs:426-439`

- [ ] **Step 1: Write the failing hostile-response test**

Append:

```rust
#[tokio::test]
async fn token_exchange_error_never_renders_reflected_client_secret() {
    let root = tempfile::tempdir().expect("temporary secret root");
    let secret_path = root.path().join("auth0-client-secret");
    let marker = "reflected-secret-value";
    fs::write(&secret_path, marker).expect("write secret fixture");
    let secret = ClientSecret::from_file(&secret_path).expect("load client secret");
    let (token_url, _) = token_server(StatusCode::UNAUTHORIZED, marker).await;
    let transport = api_server_auth::HttpOidcTransport::new(
        token_url,
        "http://127.0.0.1/unused-jwks",
        secret,
    );

    let rendered = transport
        .exchange_code(&token_request())
        .await
        .expect_err("Auth0 denial must fail")
        .to_string();

    assert!(rendered.contains("401"));
    assert!(!rendered.contains(marker));
}
```

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```powershell
cargo test -p api-server-auth --test http_oidc_transport token_exchange_error_never_renders_reflected_client_secret -- --exact
```

Expected: assertion fails because the current transport includes the complete
untrusted response body in `TransportError`.

- [ ] **Step 3: Stop reading unsuccessful token bodies**

Move the status check before body extraction:

```rust
let status = body.status();
if !status.is_success() {
    return Err(TransportError(format!("token exchange http {status}")));
}
let text = body
    .text()
    .await
    .map_err(|e| TransportError(format!("token exchange body: {e}")))?;
auth::oidc::TokenResponse::from_json(&text).map_err(|e| TransportError(e.to_string()))
```

- [ ] **Step 4: Run the transport test file and verify GREEN**

Run:

```powershell
cargo test -p api-server-auth --test http_oidc_transport --no-fail-fast
```

Expected: all secret-file, token-form, and reflected-secret tests pass.

- [ ] **Step 5: Commit error containment**

```powershell
git add apps/api-server/auth/src/lib.rs apps/api-server/auth/tests/http_oidc_transport.rs
git commit -m "fix(auth): redact Auth0 token endpoint failures"
```

### Task 4: Deployment and decision documentation

**Files:**
- Modify: `deploy/compose/compose.yml:129-145`
- Modify: `deploy/compose/.env.example:1-13`
- Modify: `deploy/secrets/README.md:1-40`
- Modify: `docs/decisions/0002-auth0-oidc-and-sessions.md:31-42`

- [ ] **Step 1: Wire the mounted secret path in Compose**

Add under the existing Auth0 variables for `api-server`:

```yaml
      AUTH0_DOMAIN: ${AUTH0_DOMAIN:-}
      AUTH0_CLIENT_ID: ${AUTH0_CLIENT_ID:-}
      AUTH0_CLIENT_SECRET_FILE: /run/secrets/auth0_client_secret
```

- [ ] **Step 2: Document non-secret and secret provisioning**

Keep `.env.example` values as placeholders and add this comment:

```dotenv
# Auth0 first-party Regular Web Application (Client Secret Post).
# Domain and Client ID are non-secret. The Client Secret belongs only in
# ../secrets/auth0_client_secret and is mounted through AUTH0_CLIENT_SECRET_FILE.
AUTH0_DOMAIN=your-tenant.auth0.com
AUTH0_CLIENT_ID=your-client-id
```

Add a `## Auth0 confidential client` section to `deploy/secrets/README.md`
that states:

```markdown
## Auth0 confidential client

Create a first-party Regular Web Application using Client Secret Post,
Authorization Code, PKCE S256, and RS256 ID tokens. Store the exact Auth0
Client Secret as the sole line of `auth0_client_secret`; never copy it into an
environment file or command argument. Compose mounts the file read-only and
sets `AUTH0_CLIENT_SECRET_FILE=/run/secrets/auth0_client_secret`.

The non-secret tenant domain and Client ID belong in
`deploy/compose/.env`. The current operator-selected tenant is hosted in the
Auth0 Japan region. PAR, JAR, refresh tokens, and additional credentials are
not part of this deployment contract.
```

- [ ] **Step 3: Correct ADR-0002**

Replace the incorrect D2 bullet with:

```markdown
- PKCE S256 protects the authorization code from interception. Because the
  Auth0 Regular Web Application is a confidential client, the token exchange
  also authenticates with `client_secret` using Client Secret Post. The
  secret is read from a mounted file and never enters browser state, URLs,
  logs, database rows, or provider-neutral request metadata.
```

- [ ] **Step 4: Validate the static deployment contract**

Run:

```powershell
rg -n "AUTH0_(DOMAIN|CLIENT_ID|CLIENT_SECRET_FILE)|auth0_client_secret" deploy/compose/compose.yml deploy/compose/.env.example deploy/secrets/README.md
docker compose --env-file deploy/compose/.env.example -f deploy/compose/compose.yml config --no-interpolate --quiet
git diff --check
```

Expected: all three Auth0 keys and the secret mount are present; Compose
parses without resolving real secret files; Git reports no whitespace errors.

- [ ] **Step 5: Commit deployment documentation**

```powershell
git add deploy/compose/compose.yml deploy/compose/.env.example deploy/secrets/README.md docs/decisions/0002-auth0-oidc-and-sessions.md
git commit -m "docs(auth): wire confidential Auth0 deployment"
```

### Task 5: Local Japan-tenant selection and verification

**Files:**
- Create ignored: `deploy/compose/.env`
- Operator creates ignored: `deploy/secrets/auth0_client_secret`

- [ ] **Step 1: Create the ignored non-secret local environment file**

Copy `deploy/compose/.env.example` to `deploy/compose/.env` and set exactly:

```dotenv
AUTH0_DOMAIN=lagrange-station.jp.auth0.com
AUTH0_CLIENT_ID=YZ4T7g575IohtS1HsltlFAiU7AlyUUuI
```

Retain the other example deployment values unchanged for local development.

- [ ] **Step 2: Confirm ignored-file boundaries**

Run:

```powershell
git check-ignore -v deploy/compose/.env deploy/secrets/auth0_client_secret
git status --short
```

Expected: both real configuration paths are ignored and neither appears in
Git status.

- [ ] **Step 3: Ask the operator to provision the secret locally**

The operator reveals the Client Secret in Auth0 Dashboard and writes it as the
only line of `deploy/secrets/auth0_client_secret`. The value is never pasted
into chat, a tracked patch, a command argument, or a test log. Stop live
verification if the file is absent; never synthesize an Auth0 credential.

- [ ] **Step 4: Run affected Rust verification**

Run:

```powershell
cargo fmt --all -- --check
cargo test -p api-server-auth --all-targets --no-fail-fast
cargo test -p auth --all-targets --no-fail-fast
cargo clippy -p api-server-auth -p auth --all-targets --all-features -- -D warnings
```

Expected: formatting is clean; all tests pass; Clippy emits no warnings.

- [ ] **Step 5: Run the external Auth0 vendor gate without echoing the secret**

Run from a PowerShell session after the ignored secret file exists:

```powershell
$env:LAGRANGE_AUTH0_DOMAIN = 'lagrange-station.jp.auth0.com'
$env:LAGRANGE_AUTH0_CLIENT_ID = 'YZ4T7g575IohtS1HsltlFAiU7AlyUUuI'
$env:LAGRANGE_AUTH0_CLIENT_SECRET = (Get-Content -Raw 'deploy/secrets/auth0_client_secret').TrimEnd("`r", "`n")
try {
  cargo test -p auth --test vendor_auth0 -- --ignored --nocapture
} finally {
  Remove-Item Env:LAGRANGE_AUTH0_DOMAIN, Env:LAGRANGE_AUTH0_CLIENT_ID, Env:LAGRANGE_AUTH0_CLIENT_SECRET -ErrorAction SilentlyContinue
}
```

Expected: the real tenant publishes a non-empty RS256 JWKS and accepts the
registered client/callback authorization request. No output contains the
Client Secret. If Auth0 behavior exposes an existing vendor-test defect,
diagnose it separately rather than weakening or silently skipping the gate.

- [ ] **Step 6: Record final state**

Run:

```powershell
git status --short --branch
git log -5 --oneline
```

Expected: only intentional tracked commits are present; ignored Auth0 files do
not appear; the implementation, containment, and deployment commits are in
history.
