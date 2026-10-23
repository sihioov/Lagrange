// Bounded new R2-07 reproduction; synthetic loopback only, no production requests.
import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { resolve, join, extname, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium, expect } from "@playwright/test";
import { build } from "vite";

const repository = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const evidence = resolve(process.argv[2] ?? "/");
assert(evidence.startsWith("/tmp/lagrange-recommendation-submit-"));
assert(process.argv[2], "Pass a NEW absolute /tmp evidence directory");
await mkdir(evidence, { recursive: false, mode: 0o700 });
const fixture = join(evidence, "fixture"),
	client = join(evidence, "client");
await mkdir(fixture);
await mkdir(join(evidence, "empty-env"));
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
	"const router = {refresh(){}};export function useRouter(){return router;}",
);
await writeFile(
	join(fixture, "main.tsx"),
	`
import {createRoot} from 'react-dom/client';
import {useState} from 'react';
import {OwnerBetaRunForm} from '@/components/recommendations/owner-beta-run-form';
import {RecommendationRunForm} from '@/components/recommendations/recommendation-run-form';
import {LocaleProvider} from '@/lib/i18n/client';
const configs=[{id:'00000000-0000-4000-8000-000000000101',label:'Fixture strategy'}];
window.__probe={bodyReady:0,parsed:0,aborted:0};
const originalFetch=window.fetch.bind(window);
window.fetch=async (...args)=>{
 let response;
 try{response=await originalFetch(...args);}catch(error){if(error.name==='AbortError')window.__probe.aborted++;throw error;}
 const read=response.json.bind(response);
 response.json=async()=>{
  const result=await read();
  const options=new URLSearchParams(location.search);
  const input=args[0], path=new URL(input instanceof Request ? input.url : String(input),location.href).pathname;
  const target=options.get('stage')==='csrf' ? path.endsWith('/auth/csrf') : !path.endsWith('/auth/csrf');
  if(options.get('phase')==='body'&&target){window.__probe.bodyReady++;await new Promise(resolve=>window.__releaseBody=resolve);}
  window.__probe.parsed++;return result;
 };return response;
};
function Fixture(){
 const [mounted,setMounted]=useState(true);
 const owner=new URLSearchParams(location.search).get('kind')==='owner';
 return <LocaleProvider initialLocale="en"><button id="leave" onClick={()=>setMounted(false)}>Leave</button>
 <output id="mounted">{String(mounted)}</output>
 {mounted && (owner ? <OwnerBetaRunForm configs={configs} defaultAsOf="2026-10-15" supportedAsOf={['2026-10-15']}/>
 : <RecommendationRunForm configs={configs} defaultAsOf="2026-10-15"/>)}</LocaleProvider>;
}
createRoot(document.getElementById('root')).render(<Fixture/>);
`,
);
await build({
	configFile: false,
	envDir: join(evidence, "empty-env"),
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
const server = createServer((req, res) => {
	const path = new URL(req.url, "http://localhost").pathname;
	const file = resolve(client, path === "/" ? "index.html" : path.slice(1));
	if (!file.startsWith(client + "/")) return res.writeHead(404).end();
	void readFile(file).then(
		(b) => {
			res.setHeader(
				"Content-Type",
				extname(file) === ".js" ? "text/javascript" : "text/html",
			);
			res.end(b);
		},
		() => res.writeHead(404).end(),
	);
});
const checks = [],
	foreignRequests = [],
	pageErrors = [];
let browser;
try {
	await new Promise((yes, no) => {
		server.once("error", no);
		server.listen(0, "127.0.0.1", yes);
	});
	const origin = "http://127.0.0.1:" + server.address().port;
	browser = await chromium.launch({ headless: true, timeout: 15000 });
	for (const kind of ["owner", "regular"])
		for (const stage of ["csrf", "mutation"])
			for (const phase of ["headers", "body"])
				for (const action of ["current", "unmount"]) {
					const context = await browser.newContext({ serviceWorkers: "block" });
					await context.route("**/*", (route) => {
						if (new URL(route.request().url()).origin === origin)
							return route.continue();
						foreignRequests.push(route.request().url());
						return route.abort();
					});
					const page = await context.newPage();
					page.on("pageerror", (e) => pageErrors.push(e.message));
					let seen = 0,
						login = 0,
						posts = 0,
						release;
					const gate = new Promise((yes) => (release = yes));
					await page.route("**/login", (route) => {
						login++;
						return route.fulfill({
							status: 200,
							contentType: "text/html",
							body: "<p>Fixture login</p>",
						});
					});
					await page.route("**/api/**", async (route) => {
						const request = route.request(),
							method = request.method();
						assert(["GET", "POST"].includes(method));
						if (method === "POST") posts++;
						if (method === "GET" && stage === "mutation")
							return route.fulfill({
								status: 200,
								contentType: "application/json",
								body: JSON.stringify({ csrf_token: "fixture-token" }),
							});
						seen++;
						await gate;
						return route.fulfill({
							status: 401,
							contentType: "application/json",
							body: JSON.stringify({
								error: {
									code: "SESSION_EXPIRED",
									message: "Fixture session expired",
									request_id: "fixture",
								},
							}),
						});
					});
					await page.goto(
						origin + "/?kind=" + kind + "&stage=" + stage + "&phase=" + phase,
					);
					await page.locator('button[type="submit"]').click();
					await expect.poll(() => seen).toBe(1);
					if (phase === "body") {
						release();
						await expect
							.poll(() => page.evaluate(() => window.__probe.bodyReady))
							.toBe(1);
					}
					if (action === "unmount") {
						await page.locator("#leave").click();
						await expect(page.locator("#mounted")).toHaveText("false");
					}
					if (phase === "body")
						await page.evaluate(() => window.__releaseBody());
					else release();
					// Session navigation must happen for the active form, and must not happen after unmount.
					if (action === "current") await expect.poll(() => login).toBe(1);
					else await page.waitForTimeout(700);
					const probe = await page
						.evaluate(() => window.__probe ?? null)
						.catch(() => null);
					checks.push({
						kind,
						stage,
						phase,
						action,
						login,
						posts,
						probe,
						passed: login === (action === "current" ? 1 : 0),
					});
					await context.close();
				}
	for (const kind of ["owner", "regular"])
		for (const action of ["current", "unmount"]) {
			const context = await browser.newContext({ serviceWorkers: "block" });
			await context.route("**/*", (route) => {
				if (new URL(route.request().url()).origin === origin)
					return route.continue();
				foreignRequests.push(route.request().url());
				return route.abort();
			});
			const page = await context.newPage();
			page.on("pageerror", (e) => pageErrors.push(e.message));
			let posts = 0,
				polls = 0,
				completed = 0,
				release;
			const gate = new Promise((yes) => (release = yes));
			await page.route("**/api/**", async (route) => {
				const req = route.request();
				if (req.url().endsWith("/auth/csrf"))
					return route.fulfill({
						status: 200,
						contentType: "application/json",
						body: JSON.stringify({ csrf_token: "fixture-token" }),
					});
				if (req.method() === "POST") {
					posts++;
					await gate;
					const id = "00000000-0000-4000-8000-000000000101";
					const body =
						kind === "owner"
							? { run_id: id, job_id: id, status: "PENDING" }
							: {
									id,
									as_of: "2026-10-15",
									created_at: "2026-10-16T07:00:00Z",
									provenance: {},
									status: "PENDING",
									trigger_kind: "MANUAL",
								};
					await route.fulfill({
						status: 200,
						contentType: "application/json",
						body: JSON.stringify(body),
					});
					completed++;
					return;
				}
				assert.equal(req.method(), "GET");
				polls++;
				return route.fulfill({
					status: 500,
					contentType: "application/json",
					body: JSON.stringify({
						error: {
							code: "INTERNAL",
							message: "Fixture polling stop",
							request_id: "fixture",
						},
					}),
				});
			});
			await page.goto(origin + "/?kind=" + kind);
			await page.locator('button[type="submit"]').click();
			await expect.poll(() => posts).toBe(1);
			if (action === "unmount") {
				await page.locator("#leave").click();
				await expect(page.locator("#mounted")).toHaveText("false");
			}
			release();
			await expect.poll(() => completed).toBe(1);
			if (action === "current")
				await expect.poll(() => polls).toBeGreaterThanOrEqual(1);
			else await page.waitForTimeout(700);
			const probe = await page.evaluate(() => window.__probe);
			checks.push({
				kind,
				stage: "success",
				phase: "headers",
				action,
				posts,
				polls,
				completed,
				probe,
				passed:
					posts === 1 &&
					completed === 1 &&
					probe.aborted === 0 &&
					(action === "current" ? polls >= 1 : polls === 0),
			});
			await context.close();
		}
	assert.deepEqual(foreignRequests, []);
	assert.deepEqual(pageErrors, []);
	assert.equal(checks.length, 20);
	assert(
		checks.every((check) => check.passed),
		"Only the active recommendation form may cause navigation or polling",
	);
} finally {
	await browser?.close();
	server.closeAllConnections();
	await new Promise((done) => server.close(done));
	const value = {
		checks,
		foreignRequests,
		pageErrors,
		scope:
			"New isolated React/Chromium delayed session-error reproduction only. No production/auth/provider/CI repeat.",
	};
	await writeFile(
		join(evidence, "result.json"),
		JSON.stringify(value, null, 2) + "\n",
	);
	console.log(JSON.stringify({ ...value, evidence }));
}
