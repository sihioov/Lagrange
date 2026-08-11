# Auth0 Confidential Client Wiring Design

## Goal

Connect the first-party Lagrange Station Auth0 application to the Rust/Axum
authentication authority as a confidential OIDC client. The production path
must use Authorization Code with PKCE S256 **and** authenticate the token
exchange with the Auth0 client secret. The tenant selected for this deployment
is the Japan-region tenant `lagrange-station.jp.auth0.com`.

The implementation must preserve the existing exact redirect URI,
state/nonce, RS256/JWKS validation, invite-only admission, opaque first-party
session, and no-refresh-token contracts.

## Current Problem

`HttpOidcTransport` posts `grant_type`, `code`, `redirect_uri`, `client_id`, and
`code_verifier` to `/oauth/token`, but deliberately omits `client_secret`.
That request shape is valid for a public PKCE client. The Auth0 application is
a Regular Web Application using `Client Secret Post`, so it is a confidential
client and must authenticate at the token endpoint.

The repository already declares and mounts `auth0_client_secret`, but no
environment key identifies the mounted file and the transport cannot consume
the secret. The ADR statement that PKCE replaces confidential-client
authentication is therefore incorrect: PKCE protects the authorization code;
client authentication proves which confidential application is redeeming it.

## Chosen Approach

Keep Auth0 configured as a first-party Regular Web Application with `Client
Secret Post`. Extend only the production HTTP transport and runtime
configuration seam:

1. Read the client secret from a file path supplied as
   `AUTH0_CLIENT_SECRET_FILE`.
2. Pass the secret into `HttpOidcTransport` at construction.
3. Include `client_secret` in the form-encoded `/oauth/token` request while
   retaining `code_verifier`.
4. Keep the provider-neutral `TokenRequest` free of secret material. The
   transport owns the provider credential, so simulator and core protocol
   tests do not need a fake client secret.

This is preferred over changing Auth0 to authentication method `None`, which
would misclassify a server-side client as public, and over moving OIDC into the
Next.js frontend, which would replace the approved Rust session authority.

## Configuration and Secret Boundaries

Non-secret deployment settings remain environment variables:

- `AUTH0_DOMAIN`
- `AUTH0_CLIENT_ID`
- `AUTH0_CLIENT_SECRET_FILE`

The real client secret remains only in the gitignored
`deploy/secrets/auth0_client_secret` file. Compose mounts it read-only at
`/run/secrets/auth0_client_secret` and sets
`AUTH0_CLIENT_SECRET_FILE=/run/secrets/auth0_client_secret` for the API server.
The local ignored `deploy/compose/.env` may contain the selected Japan tenant
domain and Client ID, but never the Client Secret.

Secret-file loading trims one trailing line ending and rejects missing,
unreadable, or empty content with an error naming only the configuration key
or path. Errors, debug output, and test diagnostics must never include the
secret value. The secret is not added to the provider-neutral request structs,
database, browser, URL, Raw data, or application logs.

## Auth0 Application Contract

The Auth0 application remains configured as follows:

- ownership: first-party;
- type: Regular Web Application;
- token endpoint authentication: Client Secret Post;
- authorization flow: Authorization Code;
- ID-token signing: RS256;
- callback: `https://app.lagrange.local/auth/callback` for the current
  development contract;
- PAR and JAR: not required;
- refresh tokens: not requested or retained.

An eventual production hostname adds its exact HTTPS callback, login, logout,
and origin values without removing the exact-match development contract until
that environment is retired.

## Data Flow

1. The browser reaches the Lagrange Station login route.
2. The Rust authority redirects to the Japan Auth0 tenant with PKCE S256,
   state, nonce, client ID, and the exact callback URI.
3. Auth0 redirects an authorization code to the exact callback.
4. The Rust authority posts the code, callback URI, client ID, PKCE verifier,
   and client secret to Auth0 over TLS.
5. Auth0 returns an ID token. The server validates RS256 signature through the
   tenant JWKS plus issuer, audience, expiry, nonce, verified email, and invite
   rules.
6. The server discards provider tokens at the boundary and issues the existing
   opaque Lagrange Station session cookie.

## Failure Handling

- Missing or empty secret files fail startup/configuration before a login is
  attempted.
- Auth0 token-endpoint failures remain typed transport failures, but response
  diagnostics are bounded and sanitized so reflected credential material
  cannot reach logs or user responses.
- No fallback to an unauthenticated token exchange is allowed.
- No default or placeholder secret is accepted outside tests.

## Testing

Implementation follows test-first order:

1. Add a local HTTP token-endpoint test that fails because the current request
   omits `client_secret`, while also asserting that PKCE fields remain present.
2. Add secret-file tests for valid, missing, unreadable, and empty files and
   assert that error rendering never contains secret content.
3. Add a hostile token-endpoint response test that reflects the submitted
   secret and prove the returned error is redacted.
4. Implement the smallest transport/configuration change that turns those
   tests green.
5. Run the `api-server-auth` and `auth` suites, then formatting and strict
   Clippy for the affected workspace targets.
6. Run the ignored Auth0 vendor suite only after the operator supplies the
   Japan tenant's three `LAGRANGE_AUTH0_*` values locally. The suite must never
   print the secret.

## Documentation Changes

Update ADR-0002 to state that PKCE supplements confidential-client
authentication. Update the Compose and secret documentation with
`AUTH0_CLIENT_SECRET_FILE`, the Japan tenant provisioning shape, and the rule
that Client Secret values never enter tracked files or command history.

## Out of Scope

- Enabling Auth0 paid MFA, Roles, Organizations, custom domains, PAR, or JAR.
- Moving authentication into the Next.js process.
- Creating production DNS/TLS for the eventual public application hostname.
- Automating Auth0 Dashboard mutation or storing the Client Secret in this
  repository.
- Completing the currently stubbed production API-server executable or full
  Compose deployment in this change.
