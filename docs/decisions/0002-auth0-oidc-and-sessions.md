# ADR-0002: Auth0 invite-only OIDC with opaque first-party sessions

Status: approved (implemented by Todo 22)
Date: 2026-08-06

## Context

Lagrange Station is invite-only (FR-AUTH-001) with OIDC authentication and
role-based permissions (FR-AUTH-002), user isolation (FR-AUTH-003), and
Owner step-up authentication for sensitive actions (FR-AUTH-004). The design
contract (System Design §14.1) requires Authorization Code + PKCE, exact
redirect URI, state+nonce, JWKS/issuer/audience/expiry validation, single-use
invites on verified email, immutable `(iss, sub)` identity, an opaque
`__Host-lagrange_session` cookie hashed in `web_sessions`, CSRF synchronizer
tokens, short sessions with re-login (no browser refresh tokens), and Owner
MFA/fresh-auth via `auth_time`/`amr`.

## Decisions

### D1: Hand-rolled minimal OIDC/session core over a heavy framework

`crates/auth` implements PKCE S256, RS256 JWT/JWKS validation, opaque session
hashing, and CSRF verification directly over small primitives already pinned
by the workspace lock (`ring` 0.17 for RSA verify + constant-time, `sha2`,
`base64`, `rand`, `subtle`, `url`, `serde`). No `oauth2`/`openidconnect`/JWT
framework dependency: the wire contract is well-specified (RFC 7636, RFC 7519,
RFC 7517, OIDC Core) and the attack surface shrinks to ~700 lines we own and
test. The Auth0 tenant is exercised through the same contract by the
simulator (below) and later by the `vendor`-tagged suite.

### D2: Authorization Code + PKCE S256, exact redirect, state+nonce

- Verifier: 64 random alphanumeric chars (RFC 7636 alphabet, 43..=128);
  challenge = Base64url-NoPad(SHA-256(verifier)). The verifier never leaves
  the server; the browser redirect carries only the challenge.
- The configured `redirect_uri` is emitted verbatim in `/authorize` and in
  the token exchange (exact-match, no user-supplied redirects).
- `state` (callback CSRF) and `nonce` (ID-token binding) are 32 random bytes
  hex, unique per request, stored server-side as a single-use `PendingAuth`
  record; replay of a consumed state is denied and audited.
- PKCE S256 replaces the client secret (Auth0 best practice); `client_secret`
  is deliberately absent from the transport.

### D3: Identity is the immutable `(issuer, subject)` pair, never email

Invites address a normalized email (trim + lowercase), but redemption binds
the identity to `(iss, sub)` captured from the verified ID token. Email
profile changes at the provider keep the same internal user; a second
subject with the same email cannot reuse the consumed invite. Internal
user ids are random `usr_<hex>`, never the email.

### D4: Invite-only onboarding, fail-closed roles

`email_verified` is required; invites are single-use with an expiry; expired/
reused/unverified/mismatched denials are typed and audited. The role comes
from the ID-token `roles` claim (member/owner, owner dominating); when the
claim is silent the invite role applies; a non-empty claim set naming no
known role DENIES (fail-closed - no guessing, no silent fallback).

### D5: Opaque first-party session cookie, hashed at rest

`__Host-lagrange_session` = 32 random bytes (base64url, 43 chars), with
`Secure; HttpOnly; SameSite=Lax; Path=/` and NO `Domain` (host-only per the
`__Host-` prefix rules). Only the SHA-256 of the value is stored; the raw
value is the bearer. Sessions are SHORT (30 min, no sliding renewal) and
there are NO browser refresh tokens: expiry means re-login at the provider,
which keeps `auth_time`/`amr` meaningful for step-up. Every login mints a
brand-new value (session fixation impossible by construction); logout
revokes and clears the cookie.

### D6: Session persistence seam (Todo 3 BLOCKED)

`web_sessions` is a Todo-3 migration table that does not exist yet.
`auth::sessions::SessionStore` is the typed async trait contract (opaque
value hashed before storage; lookup/revoke/expiry; ownership binding to the
internal user); the tested in-memory implementation ships with Todo 22 and
the PostgreSQL implementation lands with Todo 3. The same seam applies to
`InviteStore`, `UserStore`, and `PendingAuthStore`.

### D7: CSRF synchronizer tokens

Each session is minted with a random token (delivered once over TLS via the
login response header or the session-authenticated `GET /auth/csrf`); the
SHA-256 is stored on the session record and verification is constant-time
(`subtle`). Mutations (`/auth/logout`, `/auth/invites`) require the header
echo; missing/wrong tokens are denied and audited. Rotation on demand
invalidates old tokens.

### D8: Owner step-up via `auth_time`/`amr`

Sensitive Owner actions require the Owner role, an `amr` containing `mfa`,
and `auth_time` within 15 minutes (`require_owner_step_up`). Any missing or
stale signal denies with a typed code; the session is short, so the recovery
path is a fresh re-login at the provider - no refresh tokens, no silent
allow.

### D9: Provider tokens never reach the browser

`TokenResponse` captures only `id_token`; `access_token`/`refresh_token`
from the provider are structurally absent. The browser holds only the
opaque session cookie and a per-session CSRF token. `auth`/`api-server-auth`
never write provider tokens to Next.js, browser storage, or URLs.

### D10: Contract verification without a tenant (BLOCKED_EXTERNAL)

No Auth0 tenant/credentials exist on the build host, so the full contract is
proven by `auth::simulator` - a fake OIDC provider speaking the same wire
contract (single-use auth codes bound to verifier + exact redirect, RS256
ID tokens served through a JWKS endpoint, fixed 2048-bit test key). The
`vendor`-tagged suite (`crates/auth/tests/vendor_auth0.rs`) is `#[ignore]`d,
fails loudly when forced without `LAGRANGE_AUTH0_*` env vars, and is
required before any production release gate - never silently skipped.

## Consequences

- `crates/auth` gained deps (ring/sha2/base64/rand/hex/subtle/url/chrono/
  email_address/async-trait) - all already pinned by the workspace lock.
- `apps/api-server/auth` (package `api-server-auth`) owns the Axum 0.8
  router and the reqwest (rustls) OIDC transport; `crates/auth` stays
  framework-free.
- Todo 23/25 build on this session/CSRF contract; Todo 3 replaces the
  in-memory stores behind the documented traits.
