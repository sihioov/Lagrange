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
assert(evidence.startsWith("/tmp/lagrange-daily-preference-"));
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
	"const router = {refresh(){window.__probe.refreshes++;}};export function useRouter(){return router;}",
);
await writeFile(
	join(fixture, "main.tsx"),
	`
import {createRoot} from 'react-dom/client';
import {useState} from 'react';
import {StrategyDailyControl} from '@/components/strategies/strategy-daily-control';
import {LocaleProvider} from '@/lib/i18n/client';
window.__probe={bodyReady:0,parsed:0,aborted:0,refreshes:0};
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
 const [mounted,setMounted]=useState(true),[id,setId]=useState('00000000-0000-4000-8000-000000000101'),[enabled,setEnabled]=useState(false);
 return <LocaleProvider initialLocale="en">
 <button id="leave" onClick={()=>setMounted(false)}>Leave</button>
 <button id="switch" onClick={()=>{setId('00000000-0000-4000-8000-000000000102');setEnabled(false)}}>Switch</button>
 <button id="seed" onClick={()=>setEnabled(true)}>New seed</button>
 <output id="mounted">{String(mounted)}</output><output id="config">{id}</output>
 {mounted && <StrategyDailyControl configId={id} enabled={enabled} key={id}/>}
 </LocaleProvider>;
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
const checks = [], foreignRequests = [], pageErrors = [];
let browser;
const ID = "00000000-0000-4000-8000-000000000101";
try {
 await new Promise((yes, no) => { server.once("error", no); server.listen(0, "127.0.0.1", yes); });
 const origin = "http://127.0.0.1:" + server.address().port;
 browser = await chromium.launch({headless:true, timeout:15000});
 const scenarios = [];
 for (const stage of ["csrf", "mutation"])
  for (const phase of ["headers", "body"])
   for (const action of ["current", "unmount", "switch"])
    scenarios.push({outcome:"session",stage,phase,action});
 for (const outcome of ["success", "failure"])
  for (const phase of ["headers", "body"])
   for (const action of ["current", "unmount", "switch", "seed"])
    scenarios.push({outcome,stage:"mutation",phase,action});
 for (const scenario of scenarios) {
  const {stage,phase,action,outcome}=scenario;
  const context=await browser.newContext({serviceWorkers:"block"});
  await context.route("**/*", route=>{
   if(new URL(route.request().url()).origin===origin)return route.continue();
   foreignRequests.push(route.request().url());return route.abort();
  });
  const page=await context.newPage();
  page.on("pageerror",e=>pageErrors.push(e.message));
  let seen=0,login=0,patches=0,completed=0,release;
  const gate=new Promise(yes=>release=yes);
  await page.route("**/login",route=>{
   login++; return route.fulfill({status:200,contentType:"text/html",body:"<p>Fixture login</p>"});
  });
  await page.route("**/api/**",async route=>{
   const req=route.request();
   assert(["GET","PATCH"].includes(req.method()));
   if(req.method()==="PATCH"){
    patches++;
    assert(req.url().endsWith("/strategy-configs/"+ID));
    assert.deepEqual(req.postDataJSON(),{owner_beta_daily_enabled:true});
   }
   if(req.method()==="GET"&&stage==="mutation")return route.fulfill({status:200,contentType:"application/json",body:JSON.stringify({csrf_token:"fixture-token"})});
   seen++; await gate;
   const body=outcome==="success"?{id:ID,strategy_id:"buy_and_hold",strategy_version:"1.0.0",config:{},is_active:true,owner_beta_daily_enabled:true}
    :{error:{code:outcome==="session"?"SESSION_EXPIRED":"INTERNAL",message:"Fixture error",request_id:"fixture"}};
   await route.fulfill({status:outcome==="success"?200:outcome==="session"?401:500,contentType:"application/json",body:JSON.stringify(body)});
   completed++;
  });
  await page.goto(origin+"/?stage="+stage+"&phase="+phase);
  await page.getByRole("switch").click();
  await expect.poll(()=>seen).toBe(1);
  await expect(page.getByRole("switch")).toBeDisabled();
  if(phase==="body"){
   release();await expect.poll(()=>page.evaluate(()=>window.__probe.bodyReady)).toBe(1);
  }
  if(action==="unmount"){
   await page.locator("#leave").click();await expect(page.locator("#mounted")).toHaveText("false");
  }else if(action==="switch"){
   await page.locator("#switch").click();await expect(page.locator("#config")).toHaveText(ID.slice(0,-1)+"2");
   await expect(page.getByRole("switch")).toBeEnabled();
  }else if(action==="seed"){
   await page.locator("#seed").click();await expect(page.getByRole("switch")).toBeDisabled();
  }
  if(phase==="body")await page.evaluate(()=>window.__releaseBody());else release();
  await expect.poll(()=>completed).toBe(1);
  if(outcome==="session"&&action==="current")await expect.poll(()=>login).toBe(1);
  else await page.waitForTimeout(350);
  const probe=await page.evaluate(()=>window.__probe??null).catch(()=>null);
  const stale=action==="unmount"||action==="switch";
  const ui=probe?await page.evaluate(()=>{
   const button=document.querySelector('[role="switch"]');
   return {checked:button?.getAttribute("aria-checked"),disabled:button?.disabled??null,alert:document.querySelector('[role="alert"]')!==null};
  }):null;
  let passed=patches===(stage==="mutation"?1:0)&&login===(outcome==="session"&&!stale?1:0);
  if(!(outcome==="session"&&!stale)){
   passed=passed&&probe!==null&&probe.aborted===0&&probe.refreshes===(outcome==="success"&&!stale?1:0);
   if(action!=="unmount")passed=passed&&ui.disabled===false&&ui.checked===String(action!=="switch"&&(outcome==="success"||action==="seed"))&&ui.alert===(outcome==="failure"&&!stale);
  }
  checks.push({...scenario,login,patches,completed,probe,ui,passed});
  await context.close();
 }
 assert.deepEqual(foreignRequests,[]);assert.deepEqual(pageErrors,[]);
 assert.equal(checks.length,28);
 assert(checks.every(c=>c.passed),"Only the current daily control may navigate, refresh or display a save result");
} finally {
 await browser?.close();server.closeAllConnections();await new Promise(done=>server.close(done));
 const value={checks,foreignRequests,pageErrors,scope:"Isolated actual React/Chromium daily preference UI lifetime. Synthetic loopback fixtures only; no production preference mutation, authentication, provider or Daily activation."};
 await writeFile(join(evidence,"result.json"),JSON.stringify(value,null,2)+"\n");
 console.log(JSON.stringify({...value,evidence}));
}
