#!/usr/bin/env node
// Real React components in an isolated browser; fixture responses are not runtime evidence.
import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { dirname, extname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, expect } from "@playwright/test";
import { build } from "vite";

const repository = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const evidence = resolve(process.argv[2] ?? "/");
assert(
	evidence.startsWith("/tmp/"),
	"Pass a NEW absolute /tmp evidence directory",
);
await mkdir(evidence, { recursive: false, mode: 0o700 });
const fixture = join(evidence, "fixture");
const client = join(evidence, "client");
const emptyEnv = join(evidence, "empty-env");
await mkdir(fixture);
await mkdir(emptyEnv);
const id = "00000000-0000-4000-8000-000000000101";
const stamp = "2026-10-16T07:00:00Z";
const regular = {
	id,
	as_of: "2026-10-15",
	created_at: stamp,
	provenance: {},
	status: "PENDING",
	trigger_kind: "MANUAL",
};
const sha = `sha256:${"a".repeat(64)}`;
const owner = {
	...regular,
	audience: "OWNER_ONLY",
	capability: "PRICE_RETURN_ONLY",
	input_kind: "owner_beta_historical_price_only_v1",
	items: [],
	job_id: id,
	strategy_config_id: id,
	strategy_config_sha256: sha,
	strategy_id: "buy_and_hold",
	strategy_version: "1.0.0",
	updated_at: stamp,
	vendor_snapshot: true,
	strict_pit: false,
	action_manifest_sha256: sha,
	approval_registry_sha256: sha,
	artifact_manifest_sha256: sha,
	candidate_content_sha256: sha,
	stage5_manifest_sha256: sha,
};
delete owner.provenance;
await writeFile(
	join(fixture, "index.html"),
	'<!doctype html><div id="root"></div><script type="module" src="/main.tsx"></script>',
);
await writeFile(
	join(fixture, "tsconfig.json"),
	JSON.stringify({ compilerOptions: { jsx: "react-jsx" } }),
);
await writeFile(
	join(fixture, "navigation.ts"),
	"const router = { refresh() {} }; export function useRouter() { return router; }",
);
await writeFile(
	join(fixture, "main.tsx"),
	`
import { createRoot } from 'react-dom/client';
import { OwnerBetaRunStatus } from '@/components/recommendations/owner-beta-run-status';
import { RecommendationRunStatus } from '@/components/recommendations/recommendation-run-status';
import { LocaleProvider } from '@/lib/i18n/client';
import { ownerBetaRunSchema } from '@/lib/products/owner-beta-contracts';
import { recommendationRunSchema } from '@/lib/products/contracts';
const ownerRun = ownerBetaRunSchema.parse(${JSON.stringify(owner)});
ownerBetaRunSchema.parse({ ...ownerRun, status: 'FAILED', error_code: 'FIXTURE_FAILED' });
const regularRun = recommendationRunSchema.parse(${JSON.stringify(regular)});
recommendationRunSchema.parse({ ...regularRun, status: 'FAILED' });
const owner = new URLSearchParams(location.search).get('kind') === 'owner';
createRoot(document.getElementById('root')!).render(<LocaleProvider initialLocale="en">
  {owner ? <OwnerBetaRunStatus initialStatus="PENDING" runId=${JSON.stringify(id)} />
    : <RecommendationRunStatus poll run={regularRun} />}
</LocaleProvider>);
`,
);
await build({
	configFile: false,
	envDir: emptyEnv,
	root: fixture,
	logLevel: "error",
	cacheDir: join(evidence, "vite-cache"),
	resolve: {
		alias: {
			"@": join(repository, "apps/web"),
			"next/navigation": join(fixture, "navigation.ts"),
			react: join(repository, "node_modules/react"),
			"react-dom": join(repository, "node_modules/react-dom"),
		},
	},
	build: { outDir: client, emptyOutDir: false },
});
const server = createServer((request, response) => {
	const path = new URL(request.url, "http://localhost").pathname;
	const relative = path === "/" ? "index.html" : path.slice(1);
	const file = resolve(client, relative);
	if (!file.startsWith(`${client}/`)) {
		response.writeHead(404).end();
		return;
	}
	void readFile(file).then(
		(bytes) => {
			response.setHeader(
				"Content-Type",
				extname(file) === ".js" ? "text/javascript" : "text/html",
			);
			response.end(bytes);
		},
		() => response.writeHead(404).end(),
	);
});
const checks = [];
const foreignRequests = [];
const pageErrors = [];
let browser;
try {
	await new Promise((yes, no) => {
		server.once("error", no);
		server.listen(0, "127.0.0.1", yes);
	});
	const origin = `http://127.0.0.1:${server.address().port}`;
	browser = await chromium.launch({ headless: true, timeout: 15000 });
	const context = await browser.newContext({ serviceWorkers: "block" });
	await context.route("**/*", (route) => {
		if (new URL(route.request().url()).origin === origin)
			return route.continue();
		foreignRequests.push(route.request().url());
		return route.abort();
	});
	for (const [kind, payload] of [
		["owner", owner],
		["regular", regular],
	]) {
		const page = await context.newPage();
		page.on("pageerror", (error) => pageErrors.push(error.message));
		let requests = 0;
		let releaseThird;
		const thirdGate = new Promise((yes) => {
			releaseThird = yes;
		});
		await page.route("**/api/v1/recommendations/**", async (route) => {
			assert.equal(route.request().method(), "GET");
			const step = ++requests;
			if (step === 3) await thirdGate;
			const failedPoll = step === 1 || step === 3;
			const body = failedPoll
				? {
						error: {
							code: "INTERNAL",
							message: "Temporary fixture status failure",
							request_id: "fixture",
						},
					}
				: step === 2
					? payload
					: { ...payload, status: "FAILED", error_code: "FIXTURE_FAILED" };
			if (kind === "regular" && !failedPoll) delete body.error_code;
			await route.fulfill({
				status: failedPoll ? 500 : 200,
				contentType: "application/json",
				body: JSON.stringify(body),
			});
		});
		await page.goto(`${origin}/?kind=${kind}`);
		const error = page.locator(".form-result");
		await expect(error).toHaveText("Temporary fixture status failure");
		await expect.poll(() => requests).toBeGreaterThanOrEqual(3);
		let recovered = true;
		try {
			await expect(error).toHaveCount(0, { timeout: 600 });
		} catch {
			recovered = false;
		}
		checks.push({
			kind,
			check: "successful pending response clears prior polling error",
			passed: recovered,
		});
		releaseThird();
		await expect(error).toHaveText("Temporary fixture status failure");
		await expect(
			page.getByRole("heading", { name: "Recommendation failed", exact: true }),
		).toBeVisible();
		checks.push({
			kind,
			check:
				"terminal run failure remains visible without stale transport error",
			passed: (await error.count()) === 0,
		});
		await page.screenshot({ path: join(evidence, `${kind}-terminal.png`) });
		await page.close();
	}
	assert.deepEqual(foreignRequests, []);
	assert.deepEqual(pageErrors, []);
	assert(
		checks.every((check) => check.passed),
		"Recovered polling errors must disappear",
	);
} finally {
	await browser?.close();
	server.closeAllConnections();
	await new Promise((done) => server.close(done));
	await writeFile(
		join(evidence, "result.json"),
		JSON.stringify(
			{
				checks,
				foreignRequests,
				pageErrors,
				scope:
					"Isolated real React/Chromium with synthetic HTTP responses and a router stub; no production/auth/provider proof",
			},
			null,
			2,
		) + "\n",
	);
	console.log(JSON.stringify({ checks, evidence }));
}
