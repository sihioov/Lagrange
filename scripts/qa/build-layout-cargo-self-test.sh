#!/usr/bin/env bash
set -euo pipefail
umask 077
[[ $# == 0 ]] || { echo 'usage: build-layout-cargo-self-test.sh' >&2; exit 2; }
export RUSTUP_TOOLCHAIN=1.97.1 RUSTUP_AUTO_INSTALL=0 CARGO_BUILD_JOBS=2 CARGO_NET_OFFLINE=true
unset RUSTFLAGS LAGRANGE_CODE_COMMIT
script_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
run_dir=$(mktemp -d /tmp/lagrange-build-layout-cargo-self-test.XXXXXX)
printf 'evidence: %s\n' "$run_dir"
python3 - "$repo_root" "$run_dir" <<'PY'
import hashlib,json,os,pathlib,re,shlex,subprocess,sys
repo,out=map(pathlib.Path,sys.argv[1:]);fixture=out/'fixture';fixture.mkdir();(fixture/'src').mkdir()
helper=repo/'scripts/ops/lib/release-build-layout.sh'
source=helper.read_text();body=source.split('rbl_builder_compile() {',1)[1].split('\nrbl_guard_build() {',1)[0]
# Exercise the actual producer's Cargo options, including its verbosity, so a
# future mixed-stdout regression fails with a genuine cold build.
commands=re.findall(r'^  (cargo build .*?)(?= >"\$rbl_producer/cargo.jsonl")',body,re.M|re.S)
assert len(commands)==1,'producer Cargo command must be unambiguous'
args=shlex.split(commands[0].replace('\\\n',' '))
assert args[:4]==['cargo','build','--locked','--release']
args=[{'$CARGO_PACKAGE':'cargo-stream-probe','$CARGO_BIN':'cargo-stream-probe'}.get(a,a) for a in args]
assert not any('$' in a for a in args),'unresolved producer Cargo argument'
assert args.count('--package')==args.count('--bin')==1 and '--message-format=json-render-diagnostics' in args
(fixture/'Cargo.toml').write_text('[package]\nname="cargo-stream-probe"\nversion="0.1.0"\nedition="2021"\nbuild="build.rs"\n[workspace]\n')
(fixture/'build.rs').write_text('fn main() { println!("cargo:rerun-if-changed=build.rs"); println!("plain-build-script-stdout"); }\n')
(fixture/'src/lib.rs').write_text('pub fn value() -> u32 { 42 }\n')
(fixture/'src/main.rs').write_text('fn main() { println!("{}", cargo_stream_probe::value()); }\n')
env=os.environ.copy();env['CARGO_TARGET_DIR']=str(out/'target');env['CARGO_INCREMENTAL']='0'
def run(argv,name):
 r=subprocess.run(argv,cwd=fixture,env=env,capture_output=True,timeout=120)
 (out/(name+'.stdout')).write_bytes(r.stdout);(out/(name+'.stderr')).write_bytes(r.stderr)
 assert r.returncode==0,name+' failed (see preserved stderr)'
 return r
version=run(['cargo','--version'],'version').stdout.decode().strip()
assert version.startswith('cargo 1.97.1 '),'unexpected installed toolchain'
run(['cargo','generate-lockfile','--offline'],'lockfile')
def pairs(items):
 result={}
 for k,v in items:
  assert k not in result,'duplicate JSON key';result[k]=v
 return result
def reject_constant(value):raise AssertionError('nonfinite JSON constant')
records=[]
for phase,fresh,marker in [('cold',False,'Compiling'),('warm',True,'Fresh')]:
 r=run(args,phase);lines=r.stdout.splitlines();assert lines,'Cargo stdout empty'
 events=[json.loads(line,object_pairs_hook=pairs,parse_constant=reject_constant) for line in lines if line.strip()]
 assert all(isinstance(e,dict) and isinstance(e.get('reason'),str) for e in events),'not JSON event objects'
 artifacts=[e for e in events if e['reason']=='compiler-artifact']
 assert len(artifacts)==3 and all(e.get('fresh') is fresh for e in artifacts),'unexpected compilation/reuse set'
 assert any(e['target']['kind']==['lib'] and e['executable'] is None for e in artifacts),'library null event missing'
 assert any(e['target']['kind']==['custom-build'] and e['executable'] is None for e in artifacts),'build-script null event missing'
 target=[e for e in artifacts if e['target']['name']=='cargo-stream-probe' and e['target']['kind']==['bin']]
 assert len(target)==1 and target[0]['executable']==str(out/'target/release/cargo-stream-probe'),'selected executable mismatch'
 finished=[e for e in events if e['reason']=='build-finished']
 assert finished==[{'reason':'build-finished','success':True}] and events[-1]==finished[0],'terminal success event missing'
 assert sum(e['reason']=='build-script-executed' for e in events)==1,'build-script event missing'
 assert re.search(r'(?:^|\s)'+marker+r'\s',r.stderr.decode()),'verbose stderr evidence missing'
 assert b'plain-build-script-stdout' not in r.stdout,'verbose build-script text polluted JSON stream'
 outputs=list((out/'target/release/build').glob('cargo-stream-probe-*/output'))
 assert len(outputs)==1 and b'plain-build-script-stdout' in outputs[0].read_bytes(),'Cargo did not retain build-script output'
 binary=out/'target/release/cargo-stream-probe';assert binary.is_file() and os.access(binary,os.X_OK)
 records.append({'phase':phase,'artifact_count':len(artifacts),'fresh':fresh,'event_count':len(events),'stdout_sha256':hashlib.sha256(r.stdout).hexdigest(),'stderr_sha256':hashlib.sha256(r.stderr).hexdigest(),'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest()})
assert records[0]['binary_sha256']==records[1]['binary_sha256'],'unchanged warm binary differs'
result={'status':'PASS','cargo_version':version,'producer_argv':args,'helper_sha256':hashlib.sha256(helper.read_bytes()).hexdigest(),'records':records,'scope':'Two tiny offline host builds; fixture binary not executed; no Docker, product Cargo build, or provider call'}
(out/'result.json').write_text(json.dumps(result,indent=2)+'\n')
print('BUILD_LAYOUT_CARGO_SELF_TEST: PASS (cold/warm genuine Cargo JSON and nullable artifact events)')
PY
