# R2 WP-4 D2a — Web prerequisites

- Checkout: `/data/worktrees/3puw275b/excited-dolphin`, HEAD `cf7a95b568a31a84324503b368f5ed7b5398ccb3` at inspection.
- Scope: prepare the ignored Web dependency tree and read the installed Next.js guides for future implementation. This is prerequisite work only; it is not a product test or runtime acceptance.
- Instructions read: repository `AGENTS.md`, `apps/web/AGENTS.md`, `apps/web/CLAUDE.md` (`@AGENTS.md`), and D2a section in `docs/superpowers/plans/2026-10-14-r2-daily-input-implementation.md`.

## Dependency preparation attempt

The exact requested command was invoked once from the repository root:

```sh
timeout -k 15s 10m env NODE_OPTIONS=--max-old-space-size=512 npm --prefix apps/web ci --ignore-scripts --no-audit --no-fund
```

It exited **1** immediately with npm error code `EUSAGE`. npm reported that `ci` requires an existing `package-lock.json` or `npm-shrinkwrap.json` for its selected project. This checkout has no `apps/web/package-lock.json`; the only repository `package-lock.json` is at the root. No alternate command or retry was attempted, and no lockfile was generated.

The manifest and actual root lockfile hashes were recorded before the attempt and after it:

| File | SHA-256 before | SHA-256 after |
|---|---|---|
| `apps/web/package.json` | `556dd43c173351d59b56f231a7d3393251680f309958d62d2cff11510d3fe0bf` | `556dd43c173351d59b56f231a7d3393251680f309958d62d2cff11510d3fe0bf` |
| `package-lock.json` (repository root) | `1932c810e7165807f24c9aa3005063c9e8dde3292ac27d8ede9c3735da5e9e76` | `1932c810e7165807f24c9aa3005063c9e8dde3292ac27d8ede9c3735da5e9e76` |

The requested path `apps/web/package-lock.json` does not exist, so it has no hash. The npm output did not contain credentials; no npm config or raw debug log was inspected or included. No browser download, test, build, server, Cargo, Docker, provider, production, or lifecycle action was run.

## Installed guide availability

The required guide tree `apps/web/node_modules/next/dist/docs/` is absent because dependency preparation failed. Thus I could not read or verify the installed Next.js `16.3.0` guides for server pages, client forms, Vitest, or Playwright. `apps/web/package.json` declares Next `16.3.0`, Vitest `4.1.10`, and Playwright `1.61.0`; those declarations do not establish that any package was installed. No documentation from another version was substituted.

**Follow-up blocker:** a later Web implementation worker needs an approved lockfile-backed dependency preparation that targets the repository's actual workspace lockfile, then must read the installed-version guides before editing Web code. This D2a assignment does not change the lockfile or authorize a second installation attempt.

## Checks and limitations

- `git rev-parse HEAD`: `cf7a95b568a31a84324503b368f5ed7b5398ccb3`.
- Pre/post SHA-256 comparison: both existing files match exactly as recorded above.
- D2a was not completed: npm installation failed; no Next guide path/version was observed inside an installed dependency tree.
- Tests/builds/runtime behavior: **not run / not verified**.
- Background work and permission waits: **none**. The npm process exited; no retry is pending.

## Corrected workspace preparation — 2026-10-14

The previous `EUSAGE` entry above is preserved as the result of the first, coordinator-specified app-prefix attempt. The coordinator then verified the root npm workspace/lockfile layout and corrected the command. This correction addresses the project selection; it is not a worker implementation failure.

From the repository root, the corrected command was executed exactly once:

```sh
timeout -k 15s 10m env NODE_OPTIONS=--max-old-space-size=512 npm ci --workspace apps/web --include-workspace-root=false --ignore-scripts --no-audit --no-fund
```

Exit status: **0** (`added 302 packages in 5s`). Dependencies are hoisted at the repository root; `require.resolve('next/package.json', { paths: [process.cwd() + '/apps/web'] })` resolved to `/data/worktrees/3puw275b/excited-dolphin/node_modules/next/package.json`. The installed versions are Next.js **16.3.0**, Vitest **4.1.10**, and `@playwright/test` **1.61.0**. There is still no app-local `apps/web/node_modules`; installed docs are available at `node_modules/next/dist/docs/` from the repository root, resolved for the Web workspace.

| File | SHA-256 before corrected attempt | SHA-256 after install |
|---|---|---|
| `package.json` (root) | `171bf43c0afc36e500432e784f103a6881382496b3244087c5c9cbdb3bf9c338` | `171bf43c0afc36e500432e784f103a6881382496b3244087c5c9cbdb3bf9c338` |
| `package-lock.json` (root) | `1932c810e7165807f24c9aa3005063c9e8dde3292ac27d8ede9c3735da5e9e76` | `1932c810e7165807f24c9aa3005063c9e8dde3292ac27d8ede9c3735da5e9e76` |
| `apps/web/package.json` | `556dd43c173351d59b56f231a7d3393251680f309958d62d2cff11510d3fe0bf` | `556dd43c173351d59b56f231a7d3393251680f309958d62d2cff11510d3fe0bf` |

### Installed Next.js 16.3.0 guides read

All paths below are exact installed files under `node_modules/next/dist/docs/`; they were read locally, without web fallback.

- `01-app/01-getting-started/03-layouts-and-pages.md`: App Router page files define routes; dynamic segment `params` is a Promise and should be awaited (216–248); Server Component page `searchParams` is also a Promise, reading it opts the page into dynamic rendering, and the page prop is preferred when search params load page data (252–282). This matches the existing async page and `run_id` detail selection pattern.
- `01-app/01-getting-started/05-server-and-client-components.md`: pages/layouts are Server Components by default; use Client Components for state, event handlers, effects, and browser APIs; keep the client boundary narrow because imports in a client module graph join the client bundle (11–33 and 98 onward). This aligns with server-side session/API reads and separate interactive forms/status components.
- `01-app/03-api-reference/02-components/form.md`: `next/form` with a string action submits GET search params and provides navigation/prefetch (82–89, 119–148). Its `onSubmit` caveat says calling `preventDefault()` overrides the component’s navigation behavior (150–164). Existing config and recommendation forms use controlled client handlers, CSRF mutation, and `preventDefault`; do not mechanically replace these with `next/form` for POST APIs.
- `01-app/02-guides/testing/vitest.md`: the guide explicitly says Vitest currently does not support async Server Components and recommends E2E for those components (7–9). It also notes bare `npm run test` watches by default (92–105); this repo instead defines `vitest run` in its test script. The guide's setup examples are not a request to alter this repo's existing config.
- `01-app/02-guides/testing/playwright.md`: E2E simulates user navigation and requires a running Next server; its standard flow builds/starts a server or uses Playwright `webServer` (134–144). No server was started and no browser was downloaded or launched here.

## Corrected follow-up verification and remaining scope

- Pre/post hashes of root `package.json`, root `package-lock.json`, and `apps/web/package.json` are identical as shown above. The first attempt's report and hashes remain intact.
- The guide path/version and constraints above are established from the installed dependency tree. No tests, typecheck, lint, build, Next startup, Playwright, or product runtime were run; install success is dependency preparation only.
- No manifest/lockfile or source change was made. No further install attempt, fallback, global config change, or Git mutation occurred.
- Unverified: actual test/build/browser execution and any runtime behavior (**none claimed**). Follow-up: implementation workers must still run the specifically authorized checks in their later scope and distinguish those results from this install.
- Background work and permission waits: **none**.
