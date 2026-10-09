// Bounded, provider-free contract validation. No product code or test harness runs.
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import { fileURLToPath } from 'node:url';
import { createRequire } from 'node:module';
const require = createRequire(import.meta.url);
const Ajv = require('ajv/dist/2020').default;
const addFormats = require('ajv-formats');
const here = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(here, '../../..');
const args = process.argv.slice(2);
assert(args.length === 0 || (args.length === 1 && args[0] === '--baseline-reservations'),
  'usage: node tests/fixtures/service-status/validate_contract.mjs [--baseline-reservations]');
const baselineReservations = args.includes('--baseline-reservations');
const read = p => JSON.parse(fs.readFileSync(p, 'utf8'));
const schema = read(path.join(root, 'configs/operations/service-status.schema.json'));
const catalog = read(path.join(root, 'configs/operations/service-status-catalog.json'));
const input = read(path.join(here, 'base-input.json'));
const output = read(path.join(here, 'base-output.json'));
const cases = read(path.join(here, 'cases.json')).cases;
const wire = read(path.join(here, 'wire-cases.json')).cases;
const collectors = read(path.join(here, 'collector-vectors.json')).cases;
const ajv = new Ajv({ allErrors: true, strict: true });
addFormats(ajv);
assert(ajv.validateSchema(schema), JSON.stringify(ajv.errors));
ajv.addSchema(schema);
const validators = Object.fromEntries(['hostSnapshot','hostObservation','apiResponse','catalog','evaluationInput'].map(k => [k,ajv.compile({$ref:`urn:lagrange:service-status:v1#/$defs/${k}`})]));
function validate(kind, doc, label) {
  const v = validators[kind];
  assert(v(doc), `${label}: ${JSON.stringify(v.errors)}`);
}
export function applyPatch(doc, ops) {
  const r = structuredClone(doc);
  for (const op of ops) {
    assert(['replace','add','remove'].includes(op.op));
    const parts = op.path.split('/').slice(1).map(k=>k.replaceAll('~1','/').replaceAll('~0','~'));
    assert(parts.length && parts.every(k=> !['__proto__','constructor','prototype'].includes(k)));
    let p = r;
    for (const k of parts.slice(0,-1)) { assert(Object.hasOwn(p,k), op.path); p=p[k]; }
    const k=parts.at(-1);
    if (op.op !== 'add') assert(Object.hasOwn(p,k),op.path);
    if (op.op==='remove') { if(Array.isArray(p)) p.splice(Number(k),1); else delete p[k]; }
    else p[k]=structuredClone(op.value);
  }
  return r;
}
const ids=catalog.services.map(s=>s.id);
assert.equal(new Set(ids).size,22);
assert.deepEqual(input.host_read.snapshot.observations.map(s=>s.id),ids.filter(id=>!['market-stream','intraday-calendar'].includes(id)));
validate('catalog',catalog,'catalog');validate('hostSnapshot',input.host_read.snapshot,'base host');validate('evaluationInput',input,'base input');
const seen=new Set(),states=new Set(),workStates=new Set(),overall=new Set();
const priority=['disabled','healthy','waiting','unknown','degraded','failed'];
function checkOutput(doc,label) {
 validate('apiResponse',doc,label);
 assert.deepEqual(doc.services.map(s=>s.id),ids,label);
 let max=0;
 for (const s of doc.services) {
   states.add(s.status);workStates.add(s.work_status);
   if(s.id!=='research-worker') max=Math.max(max,priority.indexOf(s.status));
   if(s.status==='healthy') assert(s.observed_at && s.valid_until, `${label}: unbounded healthy ${s.id}`);
   if(s.status==='unknown') assert.equal(s.valid_until,null,`${label}: unknown validity ${s.id}`);
   assert.equal(s.kind,catalog.services.find(x=>x.id===s.id).kind);
 }
 assert.equal(doc.overall_status,priority[max],`${label}: overall reducer mismatch`);
 overall.add(doc.overall_status);
}
checkOutput(output,'base output');
for(const c of cases) {
 assert(!seen.has(c.id),c.id);seen.add(c.id);
 const raw=applyPatch(input,c.input_patch),expected=applyPatch(output,c.expected_patch);
 validate('evaluationInput',raw,c.id+' input');checkOutput(expected,c.id+' expected');
 const host=raw.host_read.snapshot;
 if(host) {
   validate('hostSnapshot',host,c.id+' host');
   const start=Date.parse(host.collection_started_at),end=Date.parse(host.generated_at);
   assert(end>=start && end-start<=20000,c.id+' collection duration');
   for(const o of host.observations) if(o.observed_at) assert(Date.parse(o.observed_at)>=start && Date.parse(o.observed_at)<=end,c.id+' host envelope ordering');
 }
}
for(const c of collectors) { validate('hostObservation',c.expected_observation,c.id); assert(!seen.has(c.id),c.id);seen.add(c.id); }
for(const c of wire) {
 assert(!seen.has(c.id),c.id);seen.add(c.id);assert(c.layer && c.input && c.expected,c.id);
 if(c.input.statuses) {
   const derived=priority[Math.max(...c.input.statuses.map(s=>priority.indexOf(s)))];
   assert.equal(c.expected.overall_status,derived,c.id);overall.add(derived);
 }
}
for(const status of schema.$defs.status.enum) assert(states.has(status),`missing row status ${status}`);
for(const required of ['idle','active','closed','no_demand','failed','stale','unknown','disabled','pending','scheduled','current']) assert(workStates.has(required),`missing work state ${required}`);
for(const key of ['host_ttl_90','host_ttl_90_000001','calendar_age_129600','producer_heartbeat_10','producer_heartbeat_10_000001','producer_lease_margin_5','producer_lease_margin_5_000001','eod_cutoff_equal','eod_grace_equal','close_equality','open_equality','kst_midnight','same_job_recovered','calendar_gap_not_holiday']) assert(seen.has(key),key);
for(const entry of catalog.services) {
 const [f,l]=entry.source.split(':');const lines=fs.readFileSync(path.join(root,f),'utf8').split('\n');assert(Number(l)>0 && Number(l)<=lines.length && lines[Number(l)-1].trim(),entry.source);
}
// Validate the whole filename inventory, including down files. Never ignore a
// reservation collision just because an up file is missing or has another name.
export function validateMigrationInventory(names, { baselineReservations = false } = {}) {
 const approved = new Map([[58,'service_status_reads'],[59,'research_execution_status']]);
 const revisions = new Map();
 for (const name of names.filter(n => n.endsWith('.sql'))) {
   const match = /^(\d{4})_([a-z0-9_]+)\.(up|down)\.sql$/.exec(name);
   assert(match, `malformed migration filename: ${name}`);
   const [, revision, slug, direction] = match;
   const number = Number(revision);
   assert(number >= 1 && number <= (baselineReservations ? 57 : 59),
     `unapproved migration revision: ${name}`);
   if (approved.has(number)) assert.equal(slug, approved.get(number), `reservation collision: ${name}`);
   const pair = revisions.get(number) ?? { slug, directions: new Set() };
   assert.equal(pair.slug, slug, `multiple names for migration ${revision}`);
   assert(!pair.directions.has(direction), `duplicate migration ${revision} ${direction}`);
   pair.directions.add(direction);
   revisions.set(number, pair);
 }
 const ordered = [...revisions.keys()].sort((a,b) => a-b);
 assert(ordered.length >= 57, 'missing baseline migrations 0001–0057');
 ordered.forEach((number, index) => {
   assert.equal(number, index + 1, 'migration revision gap');
   assert.equal(revisions.get(number).directions.size, 2, `unpaired migration ${String(number).padStart(4,'0')}`);
 });
 return {
   mode: baselineReservations ? 'baseline-reservations' : 'implementation-waves',
   highest_revision: String(ordered.at(-1)).padStart(4,'0'),
   approved_pairs_present: [...approved].filter(([n]) => revisions.has(n)).map(([n,s]) => `${String(n).padStart(4,'0')}_${s}`),
   free_reservations: [...approved].filter(([n]) => !revisions.has(n)).map(([n,s]) => `${String(n).padStart(4,'0')}_${s}`)
 };
}
const migrationEntries = fs.readdirSync(path.join(root,'migrations'), { withFileTypes: true });
for (const entry of migrationEntries) if (entry.name.endsWith('.sql')) assert(entry.isFile(), `migration must be a regular file: ${entry.name}`);
const migrationReservations = validateMigrationInventory(migrationEntries.map(e => e.name), { baselineReservations });
// Closed schemas reject arbitrary secret-bearing fields and duplicate IDs need semantic validation.
const invalid=structuredClone(output);invalid.services[0].secret_sentinel='SYNTHETIC_FORBIDDEN_VALUE';
assert(!validators.apiResponse(invalid),'additionalProperties must fail closed');
const badHost=structuredClone(input.host_read.snapshot);badHost.observations[0].process.health='unrecognized';
assert(!validators.hostSnapshot(badHost),'unknown enum must fail closed');
console.log(JSON.stringify({schema:'Draft 2020-12 + formats valid',services:ids.length,projection_cases:cases.length,collector_vectors:collectors.length,wire_domain_ui_vectors:wire.length,row_states:[...states].sort(),overall_states:[...overall].sort(),source_citations:catalog.services.length,migration_reservations:migrationReservations,product_tests_run:0}));
