#!/usr/bin/env python3
"""Disposable-only Linux source witness rehearsal entry point.

Default prints NOT RUN. Running this file in the development/production host is
not authorization. The VM marker must be provisioned out of band, never here.
Provider-free kernel/FD checks are separate from PG/Docker/systemd/clock-step
proof; no mocked test upgrades those lanes to PASS.
"""
from __future__ import annotations
import argparse
import errno
import hashlib
import stat
import contextlib
import re
import importlib.util
import json
import os
from pathlib import Path
import select
import socket
import struct
import subprocess
import sys
import time
import uuid

sys.dont_write_bytecode=True
ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('paper_witness_integration',ROOT/'scripts/ops/lib/owner-beta-paper-evidence.py')
w=importlib.util.module_from_spec(spec);spec.loader.exec_module(w)
LANES=('kernel_timerfd','inherited_private_fd','clock_step_cancel','suspend','systemd_restart_crash','docker_lifetime','pg_roles_concurrency','synthetic_producer_db')

def kernel_fd(root=None):
    # Actual 60-second future arm, consumed exactly once. Never changes time.
    before=w.sample();timer=w.Timer(before['realtime_ns']+60*10**9)
    try:
        if not select.select([timer.fd],[],[],65)[0]:raise w.Rejected('CLOCK_INVALID')
        timer.consume();after=w.sample();w.clock_step(before,after)
        w.require(not select.select([timer.fd],[],[],0)[0])
        if root:publish(root/'timerfd-observation.json',dict(synthetic=True,before=before,after=after,expirations=1))
    finally:timer.close()
    return 'PASS'

def inherited_fd(root=None):
    parent,child=socket.socketpair(socket.AF_UNIX,socket.SOCK_SEQPACKET)
    parent.setsockopt(socket.SOL_SOCKET,socket.SO_PASSCRED,1)
    parent.settimeout(10)
    nonce=str(uuid.uuid4())
    code="import os,socket,sys; s=socket.socket(fileno=int(sys.argv[1]));s.send(sys.argv[2].encode())"
    proc=subprocess.Popen([sys.executable,'-I','-c',code,str(child.fileno()),nonce],pass_fds=(child.fileno(),),stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
    child.close()
    try:
        data,anc,flags,_=parent.recvmsg(256,128)
        creds=[struct.unpack('3i',v) for l,k,v in anc if l==socket.SOL_SOCKET and k==socket.SCM_CREDENTIALS]
        w.require(data==nonce.encode() and not flags and len(creds)==1 and creds[0][0]==proc.pid and creds[0][1]==0)
        w.require(proc.wait(timeout=10)==0)
        if root:publish(root/'fd-observation.json',dict(synthetic=True,pid=proc.pid,credential_pid=creds[0][0],credential_uid=creds[0][1],exit_code=proc.returncode))
    finally:
        parent.close()
        if proc.poll() is None:proc.terminate()
        proc.wait(timeout=10)
    return 'PASS'

def docker_lifetime(image_id,root=None):
    w.require(w.formatted(image_id,'sha256') and root is not None)
    runtime=w.load_runtime();docker=runtime.Docker()
    image=docker.object('image',image_id);w.require(image.get('Id')==image_id)
    nonce=str(uuid.uuid4());name='lagrange-paper-disposable-'+nonce
    # No provider/network/DB mount; this can never emit a production source chain.
    argv=['run','--name',name,'--network','none','--read-only','--cap-drop','ALL',
          '--cpus','1','--memory','128m','--pids-limit','32','--label','lagrange.paper.disposable='+nonce,
          '--security-opt','no-new-privileges:true','--restart','no','--log-driver','none',
          '--entrypoint','/bin/sh',image_id,'-c','exit 0']
    w.require(not docker.call(['ps','--all','--filter','name=^/'+name+'$','--format','{{.ID}}']).strip())
    publish(root/'docker-intent.json',dict(synthetic=True,name=name,image_id=image_id,argv=argv))
    def verify(value):
        config=value.get('Config',{});host=value.get('HostConfig',{})
        w.require((config.get('Labels') or {}).get('lagrange.paper.disposable')==nonce
            and config.get('Entrypoint')==['/bin/sh'] and config.get('Cmd')==['-c','exit 0']
            and host.get('NetworkMode')=='none' and host.get('ReadonlyRootfs') is True
            and host.get('CapDrop')==['ALL'] and not host.get('CapAdd') and not host.get('Privileged')
            and host.get('SecurityOpt')==['no-new-privileges:true'] and host.get('Memory')==128*1024**2
            and host.get('NanoCpus')==10**9 and host.get('PidsLimit')==32
            and host.get('AutoRemove') is False,'ORIGIN_UNPROVEN')
    lease=w.OwnedContainer(docker,name,image_id,time.time_ns(),verify);failure=None
    try:
        lease.launch_uncertain=True
        docker.call(argv,timeout=10)
        lease.launch_uncertain=False
        value=lease.observe(time.monotonic()+15);w.require(value is not None)
        observation=w.docker_observation(value,'capture',w.digest(w.canonical(argv)),w.process(os.getpid()),image_id)
        w.require(observation['exit_code']==0 and observation['oom_killed'] is False)
        publish(root/'docker-observation.json',dict(synthetic=True,observation=observation))
    except BaseException as error:
        failure=w.error_tag(error);raise
    finally:
        try:
            lease.cleanup(lambda value:publish(root/'docker-cleanup-terminal.json',dict(synthetic=True,
                id=value['Id'],created=value['Created'],image=value['Image'],state={k:value['State'].get(k) for k in ('Status','Running','ExitCode','OOMKilled')},wait_errors=lease.wait_errors)))
        except Exception as error:
            publish(root/'docker-cleanup.json',dict(synthetic=True,status='UNKNOWN',name=name,pin=lease.pin,
                observed_id=lease.observed_id,original_failure=failure,cleanup_error=w.error_tag(error),wait_errors=lease.wait_errors))
            raise w.Rejected('CLEANUP_UNKNOWN') from error
        publish(root/'docker-cleanup.json',dict(synthetic=True,status='CLEAN',name=name,pin=lease.pin,
            removed=lease.removed,original_failure=failure,wait_errors=lease.wait_errors))
    return 'PASS'

def clock_cancel(root=None):
    # This lane is separately opt-in inside a verified disposable VM. It never
    # runs as part of unit tests, plan mode or ordinary source verification.
    before=w.sample(); timer=w.Timer(before['realtime_ns']+60*10**9)
    try:
        time.clock_settime_ns(time.CLOCK_REALTIME,before['realtime_ns']+3*10**9)
        w.require(select.select([timer.fd],[],[],5)[0])
        try:os.read(timer.fd,8)
        except OSError as error:
            # A readiness event or a generic reader failure does not establish
            # CANCEL_ON_SET. Preserve the kernel's exact cancellation reason.
            w.require(error.errno==errno.ECANCELED,'CLOCK_CANCEL_UNPROVEN')
            if root:publish(root/'clock-observation.json',dict(synthetic=True,before=before,after_step=w.sample(),timer_cancelled=True,read_errno=error.errno))
            return 'PASS'
        raise w.Rejected('CLOCK_INVALID')
    finally:
        try:
            elapsed=time.clock_gettime_ns(time.CLOCK_BOOTTIME)-before['boottime_ns']
            time.clock_settime_ns(time.CLOCK_REALTIME,before['realtime_ns']+elapsed)
        finally:timer.close()

# Every receipt in this harness is permanently SYNTHETIC, outside the production
# evidence root. A PASS describes one exercised VM component, never a streak.
BASE=Path('/run/lagrange-paper-disposable')
BOUNDARIES=('STARTED','ARM_INTENT','ARMED','RESERVED','DB_RESERVED','LAUNCH_INTENT','RUNNING','RAW_SOURCE','SOURCE_FINISHED','ADMISSION_LINKED','RUN_LINKED','LINKED')

import signal
import math


def publish(path, value):
    raw=w.canonical(value) if not isinstance(value,bytes) else value
    temporary=path.with_name('.'+path.name+'.'+str(uuid.uuid4()))
    fd=os.open(temporary,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600)
    try:
        with os.fdopen(fd,'wb') as out:out.write(raw);out.flush();os.fsync(out.fileno())
        os.link(temporary,path,follow_symlinks=False);os.unlink(temporary)
        parent=os.open(path.parent,os.O_RDONLY|os.O_DIRECTORY)
        try:os.fsync(parent)
        finally:os.close(parent)
    finally:
        with contextlib.suppress(FileNotFoundError):temporary.unlink()


class CustodyCommandFailure(ValueError):
    def __init__(self, original, result, secondary):
        self.original, self.result, self.secondary = original, result, secondary
        super().__init__('CUSTODY_RECORD_FAILED')


def command(argv, *, env=None, timeout=60, log=None, custody_recorder=None):
    try:
        result=subprocess.run(argv,env=env or {'PATH':'/usr/sbin:/usr/bin:/sbin:/bin','LANG':'C.UTF-8','PYTHONDONTWRITEBYTECODE':'1'},
            stdin=subprocess.DEVNULL,capture_output=True,check=False,timeout=timeout)
    except (OSError,subprocess.SubprocessError) as original:
        if custody_recorder is not None:
            try:custody_recorder(custody_command_observation(None,original))
            except Exception as secondary:raise CustodyCommandFailure(original,None,secondary) from original
        raise
    if custody_recorder is not None:
        try:custody_recorder(custody_command_observation(result,None))
        except Exception as secondary:
            original=None
            try:w.require(result.returncode==0,'DISPOSABLE_COMMAND_FAILED')
            except Exception as error:original=error
            raise CustodyCommandFailure(original,result,secondary) from secondary
    if log is not None:publish(log,result.stdout+result.stderr)
    w.require(result.returncode==0,'DISPOSABLE_COMMAND_FAILED')
    return result.stdout


def context(args):
    if getattr(args,'systemd_custody_intent',None):return custody_context(args)
    w.require(sys.platform=='linux' and os.geteuid()==0 and w.formatted(args.vm_id or '','uuid') and args.lane)
    marker=w.decode(w.read_file('/run/lagrange-disposable-paper-vm.json',16384,exact=(0,0,0o600)))
    w.require(set(marker)<= {'schema','vm_id','production','provider_network','resources'}
        and marker.get('schema')=='lagrange-disposable-paper-vm-v1' and marker.get('vm_id')==args.vm_id
        and marker.get('production') is False and marker.get('provider_network') is False)
    resources=marker.get('resources',{});w.require(isinstance(resources,dict) and set(resources)<= {'postgres_config_sha256','docker_image_ids'})
    w.require(not any(k.startswith(('KIS_','OPENDART_')) for k in os.environ),'PROVIDER_ENV_FORBIDDEN')
    virtualization=command(['/usr/bin/systemd-detect-virt','--vm']).strip();w.require(virtualization not in (b'',b'none'))
    with w.opened(BASE,directory=True,exact=(0,0,0o700)):pass
    vm=BASE/args.vm_id
    if not vm.exists():vm.mkdir(mode=0o700)
    with w.opened(vm,directory=True,exact=(0,0,0o700)):pass
    if args.crash_worker:
        root=Path(args.run_root or '')
        w.require(root.is_relative_to(vm) and root.resolve()==root and root.name in BOUNDARIES)
        with w.opened(root,directory=True,exact=(0,0,0o700)):pass
        w.require(w.read_file(root/'SYNTHETIC',64,exact=(0,0,0o600))==b'SYNTHETIC-NOT-PRODUCTION\n')
    else:
        root=vm/str(uuid.uuid4());root.mkdir(mode=0o700)
        publish(root/'SYNTHETIC',b'SYNTHETIC-NOT-PRODUCTION\n')
        pins={str(p.relative_to(ROOT)):w.digest(p.read_bytes()) for p in (Path(__file__).resolve(),ROOT/'scripts/ops/test_owner_beta_paper_evidence.py',ROOT/'scripts/ops/lib/owner-beta-paper-evidence.py')}
        publish(root/'inputs.json',dict(schema='paper-disposable-inputs-v1',synthetic=True,vm_id=args.vm_id,lane=args.lane,source_pins=pins))
    return root,resources


def suspend_lane(root=None, *, _outcome=None):
    timer=None;phase='before'
    try:
        before=w.sample();_suspend_note(_outcome,'before',before)
        phase='timer';timer=w.Timer(before['realtime_ns']+60*10**9);_suspend_note(_outcome,'timer',timer)
        phase='command'
        returned=command(['/usr/sbin/rtcwake','--mode','mem','--seconds','3'],timeout=60)
        _suspend_note(_outcome,'command',returned)
        phase='after';after=w.sample();_suspend_note(_outcome,'after',after)
        phase='suspend_check'
        checked=w.require((after['boottime_ns']-before['boottime_ns'])-(after['monotonic_ns']-before['monotonic_ns'])>10**9,'SUSPEND_NOT_OBSERVED')
        _suspend_note(_outcome,'suspend_check',checked)
        phase='guard'
        try:
            checked=w.clock_step(before,after);_suspend_note(_outcome,'guard',checked)
        except w.Rejected as guard_error:
            _suspend_note(_outcome,'guard',error=guard_error)
            observation=dict(synthetic=True,before=before,after=after,guard_rejected=True)
            if _outcome is not None:_outcome.observation=observation
            phase='publication'
            if root:
                published=publish(root/'suspend-observation.json',observation)
                _suspend_note(_outcome,'publication',published)
            _suspend_note(_outcome,'operation','PASS')
            return 'PASS'
        phase='operation';raise w.Rejected('SUSPEND_NOT_REJECTED')
    except BaseException as error:
        _suspend_note(_outcome,phase,error=error)
        _suspend_note(_outcome,'operation',error=error)
        raise
    finally:
        if timer is not None:
            try:
                closed=timer.close();_suspend_note(_outcome,'close',closed)
            except BaseException as error:
                _suspend_note(_outcome,'close',error=error)
                raise


def load_synthetic():
    path=ROOT/'scripts/ops/test_owner_beta_paper_evidence.py'
    spec=importlib.util.spec_from_file_location('paper_disposable_synthetic',path)
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module


def wait_file(path, timeout=30):
    deadline=time.monotonic()+timeout
    while time.monotonic()<deadline:
        if path.is_file():return
        time.sleep(.05)
    raise w.Rejected('DISPOSABLE_WAIT_TIMEOUT')


def crash_worker(args, root):
    # Running only in an owned transient VM unit. No real provider/DB/Docker.
    module=load_synthetic();worker_pid=os.getpid()
    def checkpoint(kind,journal):
        if kind==args.boundary:
            publish(root/'checkpoint.json',dict(synthetic=True,boundary=kind,pid=worker_pid,events=len(journal.events)))
            while True:time.sleep(1)
    rig=module.SyntheticSource.restore(root,event_hook=checkpoint)
    try:rig.run()
    except (module.w.Rejected,OSError):
        publish(root/'restart-rejected.json',dict(synthetic=True,launches=rig.launches,provider_phases=rig.provider_phases))
        return 3
    return 4  # reaching LINKED is a failed crash-boundary scenario


def unit_show(unit, case, expected=None, timeout=5):
    raw=command(['/usr/bin/systemctl','show',unit,'--property=Id,LoadState,ExecStart,FragmentPath,MainPID,ControlPID,ActiveState,Result,ReadWritePaths,KillMode,Type,Restart'],timeout=timeout)
    values=dict(line.split('=',1) for line in raw.decode().splitlines() if '=' in line)
    return custody_unit_values(values,unit,case,expected)


def custody_unit_values(values,unit,case,expected=None):
    w.require(values.get('Id')==unit,'UNIT_OWNERSHIP_UNPROVEN')
    if values.get('LoadState')=='not-found':
        w.require(not values.get('ExecStart') and not values.get('FragmentPath') and values.get('MainPID')=='0'
            and values.get('ControlPID')=='0','UNIT_OWNERSHIP_UNPROVEN')
        return values
    w.require(expected and all(re.fullmatch(r'[A-Za-z0-9/._-]+',part) for part in expected),'UNIT_OWNERSHIP_UNPROVEN')
    # systemctl's ExecStart serialization: compare the complete argv, never a
    # path substring. Additional commands or escaped/ambiguous argv fail closed.
    prefix='{ path='+expected[0]+' ; argv[]='+' '.join(expected)+' ; ignore_errors=no ; '
    execution=values.get('ExecStart','')
    w.require(values.get('LoadState')=='loaded' and values.get('FragmentPath')=='/run/systemd/transient/'+unit
        and execution.startswith(prefix) and execution.endswith(' }') and execution.count('argv[]=')==1
        and execution.count('{')==execution.count('}')==1 and values.get('ReadWritePaths')==str(case)
        and values.get('KillMode')=='control-group' and values.get('Type')=='exec' and values.get('Restart')=='no',
        'UNIT_OWNERSHIP_UNPROVEN')
    return values


def cleanup_unit(unit,case,expected,launch_uncertain=False,*,deadline=None):
    deadline=time.monotonic()+20 if deadline is None else deadline
    observed=unit_show(unit,case,expected,w.remaining(deadline))
    if observed['LoadState']=='not-found':
        w.require(not launch_uncertain,'UNIT_CLEANUP_UNKNOWN');return observed
    command(['/usr/bin/systemctl','stop','--no-block',unit],timeout=w.remaining(deadline))
    while True:
        observed=unit_show(unit,case,expected,w.remaining(deadline))
        if observed['LoadState']=='not-found':return observed
        if observed.get('ActiveState') in ('inactive','failed') and observed.get('MainPID')=='0' and observed.get('ControlPID')=='0':break
        time.sleep(min(.1,w.remaining(deadline)))
    # Ownership was checked again above, after stop and before reset-failed.
    command(['/usr/bin/systemctl','reset-failed',unit],timeout=w.remaining(deadline))
    observed=unit_show(unit,case,expected,w.remaining(deadline))
    w.require(observed['LoadState']=='not-found' or (observed.get('ActiveState')=='inactive'
        and observed.get('MainPID')=='0' and observed.get('ControlPID')=='0'),'UNIT_CLEANUP_UNKNOWN')
    return observed


def systemd_crashes(args, root):
    module=load_synthetic();summary=[]
    intent=getattr(args,'_custody_intent',None)
    for boundary in ((intent['boundary'],) if intent else BOUNDARIES):
        case=root/boundary;case.mkdir(mode=0o700);publish(case/'SYNTHETIC',b'SYNTHETIC-NOT-PRODUCTION\n')
        module.SyntheticSource(case)  # fixture bytes only; no production syscall
        unit=intent['unit'] if intent else 'lagrange-paper-disposable-'+str(uuid.uuid4())+'.service'
        expected=['/usr/bin/python3',str(Path(__file__).resolve()),'--execute','--vm-id',args.vm_id,
            '--lane','systemd_restart_crash','--crash-worker','--run-root',str(case),'--boundary',boundary]
        argv=['/usr/bin/systemd-run','--unit='+unit,'--property=Type=exec','--property=Restart=no',
            '--property=PrivateNetwork=yes','--property=NoNewPrivileges=yes','--property=ProtectSystem=strict',
            '--property=ReadWritePaths='+str(case),'--property=UMask=0077','--property=KillMode=control-group',
            '--property=TimeoutStopSec=5s','--property=SendSIGKILL=yes',*expected]
        publish(case/'owned-unit.json',dict(unit=unit,argv=argv,synthetic=True));failure=None;launch_uncertain=True
        try:
            if intent:
                w.require(argv==intent['launch_argv'] and expected==intent['worker_argv'])
                publish(case/'launch-pending.json',custody_transition(intent,'LAUNCH_PENDING'))
                command(argv,timeout=10,custody_recorder=lambda actual:publish(case/'launch-returned.json',
                    custody_transition(intent,'LAUNCH_RETURNED',command=actual)))
            else:command(argv,timeout=10)
            launch_uncertain=False
            wait_file(case/'checkpoint.json');observed=unit_show(unit,case,expected);w.require(int(observed['MainPID'])>0)
            if intent:
                checkpoint=w.decode(w.read_file(case/'checkpoint.json',8192,exact=(0,0,0o600)))
                custody_observed_checkpoint(intent,observed,checkpoint)
                publish(case/'api-identity-observed.json',custody_transition(intent,'API_IDENTITY_OBSERVED',observation=observed,checkpoint=checkpoint))
                custody_self_fault(intent)
            before={str(p.relative_to(case)):w.digest(p.read_bytes()) for p in (case/'evidence').rglob('*.json')}
            unit_show(unit,case,expected)
            command(['/usr/bin/systemctl','kill','--signal=KILL','--kill-whom=all',unit],timeout=5)
            unit_show(unit,case,expected)
            command(['/usr/bin/systemctl','restart','--no-block',unit],timeout=5);wait_file(case/'restart-rejected.json')
            rejected=w.decode(w.read_file(case/'restart-rejected.json',4096,exact=(0,0,0o600)))
            w.require(rejected['launches']==0 and rejected['provider_phases']==[],'CRASH_REDISPATCH')
            after={str(p.relative_to(case)):w.digest(p.read_bytes()) for p in (case/'evidence').rglob('*.json')}
            w.require(before==after,'CRASH_EVIDENCE_CHANGED');summary.append(dict(boundary=boundary,relaunches=0,originals_preserved=True))
        except BaseException as error:
            failure=w.error_tag(error);raise
        finally:
            if intent is None:
                try:final=cleanup_unit(unit,case,expected,launch_uncertain)
                except Exception as error:
                    publish(case/'unit-cleanup.json',dict(synthetic=True,status='UNKNOWN',unit=unit,
                        original_failure=failure,cleanup_error=w.error_tag(error)))
                    raise w.Rejected('UNIT_CLEANUP_UNKNOWN') from error
                publish(case/'unit-cleanup.json',dict(synthetic=True,status='CLEAN',unit=unit,final=final,original_failure=failure))
    publish(root/'systemd-boundaries.json',dict(synthetic=True,scenarios=summary,source_io='SYNTHETIC',systemd='ACTUAL'))
    return 'SYNTHETIC_PASS'


def pg_environment(args, resources, root):
    from urllib.parse import quote
    w.require(args.pg_config and resources.get('postgres_config_sha256'),'PG_NOT_BOUND')
    raw=w.read_file(args.pg_config,16384,exact=(0,0,0o600));w.require(w.digest(raw)==resources['postgres_config_sha256'])
    config=w.decode(raw)
    w.require(set(config)=={'schema','socket_dir','port','database','superuser','system_identifier','test_binary','test_binary_sha256'}
        and config['schema']=='paper-disposable-postgres-v1')
    socket_dir=Path(config['socket_dir']);w.require(socket_dir.is_relative_to(BASE/args.vm_id) and socket_dir.resolve()==socket_dir)
    for name in ('database','superuser'):w.require(re.fullmatch('[a-z][a-z0-9_]{0,62}',config[name]) is not None)
    w.require(type(config['port']) is int and 1024<=config['port']<=65535 and re.fullmatch('[0-9]{10,24}',config['system_identifier']))
    binary=Path(config['test_binary']);w.require(binary.is_absolute() and binary.resolve()==binary)
    with w.opened(binary) as fd:
        info=os.fstat(fd);w.require(not info.st_mode&0o022 and info.st_mode&0o111)
        h=hashlib.sha256()
        while part:=os.read(fd,1024*1024):h.update(part)
    w.require('sha256:'+h.hexdigest()==config['test_binary_sha256'],'TEST_BINARY_CHANGED')
    # Out-of-band binding plus actual cluster identifier prevents accidentally
    # using a different server at the same Unix socket after VM reprovisioning.
    env={'PATH':'/usr/sbin:/usr/bin:/sbin:/bin','LANG':'C.UTF-8','PGPASSFILE':'/dev/null','PGSERVICEFILE':'/dev/null'}
    probe=command(['/usr/bin/psql','--no-psqlrc','--tuples-only','--no-align','-h',str(socket_dir),'-p',str(config['port']),'-U',config['superuser'],'-d',config['database'],'-c','SELECT system_identifier::text FROM pg_control_system()'],env=env)
    w.require(probe.decode().strip()==config['system_identifier'],'PG_CLUSTER_CHANGED')
    # Unix socket only, no password or service-file inheritance.
    env['DATABASE_URL']=f"postgresql://{config['superuser']}@{quote(str(socket_dir),safe='')}:{config['port']}/{config['database']}"
    env['PAPER_SYNTHETIC_DB_RECEIPT_DIR']=str(root)
    publish(root/'postgres-binding.json',dict(synthetic=True,config_sha256=w.digest(raw),test_binary_sha256=config['test_binary_sha256'],system_identifier=config['system_identifier']))
    return config,env


def pg_roles(args, resources, root):
    config,env=pg_environment(args,resources,root)
    cases=('paper_0066_catalog_roles_and_preserving_down','paper_0066_replay_current_authority_and_concurrency','paper_0066_positive_terminal_and_negative_joins','paper_0066_wrong_persisted_attempt_is_rejected')
    for name in cases:
        output=command([config['test_binary'],'--exact',name,'--nocapture','--test-threads=1'],env=env,timeout=1800,log=root/(name+'.log'))
        w.require(b'1 passed; 0 failed; 0 ignored' in output and b'SKIP:' not in output,'PG_TEST_NOT_EXECUTED')
    return 'SYNTHETIC_PASS'


class PgBridge:
    def __init__(self,args,resources,root):
        self.root=root/'bridge';self.root.mkdir(mode=0o700);publish(self.root/'SYNTHETIC',b'SYNTHETIC-NOT-PRODUCTION\n');self.sequence=0;self.closed=False
        config,env=pg_environment(args,resources,root);env['PAPER_SYNTHETIC_BRIDGE_DIR']=str(self.root)
        self.log=open(root/'bridge.log','xb',buffering=0)
        self.process=subprocess.Popen([config['test_binary'],'--exact','paper_0066_synthetic_producer_bridge','--ignored','--nocapture','--test-threads=1'],env=env,stdin=subprocess.DEVNULL,stdout=self.log,stderr=self.log)
        publish(root/'owned-bridge-process.json',dict(pid=self.process.pid,identity=w.process(self.process.pid),synthetic=True))
        try:
            wait_file(self.root/'ready.json',120);self.ready=w.decode(w.read_file(self.root/'ready.json',16384,exact=(0,0,0o600)),16384,False)
            w.require(self.ready.get('synthetic') is True and self.ready.get('schema')=='paper-synthetic-bridge-v1')
        except Exception:
            self.close(False);raise
    def request(self,operation,events):
        n=self.sequence;self.sequence+=1
        publish(self.root/f'request-{n:02}.json',dict(schema='paper-synthetic-request-v1',operation=operation,events=events))
        path=self.root/f'response-{n:02}.json';wait_file(path,120)
        return w.decode(w.read_file(path,w.SESSION_LIMIT,exact=(0,0,0o600)),w.SESSION_LIMIT,False)
    def close(self,success):
        try:
            if self.process.poll() is None:
                result=self.request('stop' if success else 'abort',[])
                w.require(result.get('synthetic') is True)
                code=self.process.wait(timeout=30)
                if success:w.require(code==0,'PG_BRIDGE_FAILED')
            elif success:w.require(self.process.returncode==0)
            self.closed=True
        finally:
            if not self.closed:
                # Preserve DB name, exact PID, requests and all ambiguous state.
                # Never discover/drop arbitrary databases or kill another PID.
                publish(self.root/'cleanup-unknown.json',dict(synthetic=True,pid=self.process.pid,ready=getattr(self,'ready',None)))
                if self.process.poll() is None:self.process.terminate();self.process.wait(timeout=30)
            self.log.close()


def producer_db(args,resources,root):
    # Real future-slot timer and PostgreSQL APIs. Provider/container transport
    # is the same explicitly SYNTHETIC I/O rig as offline tests. This lane does
    # not attest the production Rust helper's sealed Raw filesystem read.
    bridge=PgBridge(args,resources,root);success=False
    try:
        module=load_synthetic();session=bridge.ready['session'];due=module.w.due_ns(session)
        w.require(due-time.time_ns()>=60*10**9,'MISSED_UNARMED')
        rig=module.SyntheticSource(root/'source',session=session,cohort=bridge.ready['cohort'],real_time=True)
        rig.database=lambda op:bridge.request(op,rig.scheduler.j.events)
        chain=rig.run();module.w.validate(chain);module.w.verify_chain(chain['events'],'LINKED')
        w.require(rig.launches==1 and rig.provider_phases==['precheck','capture','action'])
        publish(root/'chain.json',dict(schema='paper-disposable-synthetic-chain-v1',synthetic=True,production_eligible=False,chain=chain))
        before=rig.launches
        try:rig.run()
        except (module.w.Rejected,OSError):pass
        else:raise w.Rejected('DUPLICATE_DISPATCH')
        w.require(rig.launches==before);success=True
    finally:bridge.close(success)
    return 'SYNTHETIC_PASS'



CUSTODY_SCOPE='R5_G2A_SYSTEMD_CUSTODY_DISPOSABLE_ONLY'
CUSTODY_FAULT='CONTROLLER_SELF_SIGKILL_AFTER_CHECKPOINT'
CUSTODY_GUEST_ROOT='/opt/lagrange-g2a-source'
CUSTODY_SOURCES=(
 'scripts/ops/owner-beta-paper-evidence-local-integration.py',
 'scripts/ops/lib/owner-beta-paper-evidence.py',
 'scripts/ops/test_owner_beta_paper_evidence.py',
 'docs/contracts/owner-beta-paper-source-evidence-v1.schema.json',
 'docs/contracts/owner-beta-paper-current-binding-v1.schema.json',
 'scripts/ops/lib/owner-beta-daily-runtime.py',
 'crates/job-queue/tests/fixtures/owner_beta_paper_source_evidence/valid_chain.json',
 'guest_probe.py',
)

def custody_canonical(value):
    return json.dumps(value,sort_keys=True,separators=(',',':'),ensure_ascii=False,allow_nan=False).encode()

def custody_uuid(value):
    try:return type(value) is str and str(uuid.UUID(value))==value and uuid.UUID(value).int!=0
    except (ValueError,AttributeError):return False

def custody_profile(profile,vm_id):
    if (type(profile) is not dict or set(profile)!={'run_id','nonce','boundary','fault'}
        or not custody_uuid(vm_id) or not custody_uuid(profile.get('run_id')) or not custody_uuid(profile.get('nonce'))
        or len({vm_id,profile['run_id'],profile['nonce']})!=3
        or profile['boundary']!='STARTED' or profile['fault']!=CUSTODY_FAULT):
        raise ValueError('CUSTODY_PROFILE_INVALID')
    return profile

def custody_intent(vm_id,boot_id,sources,profile):
    custody_profile(profile,vm_id)
    if not custody_uuid(boot_id) or type(sources) is not dict or set(sources)!=set(CUSTODY_SOURCES):
        raise ValueError('CUSTODY_COHORT_INVALID')
    if any(type(pin) is not str or re.fullmatch('[0-9a-f]{64}',pin) is None for pin in sources.values()):
        raise ValueError('CUSTODY_SOURCE_PIN_INVALID')
    root='/run/lagrange-paper-disposable/'+vm_id+'/'+profile['run_id']
    case=root+'/STARTED';unit='lagrange-paper-disposable-'+profile['run_id']+'.service'
    harness=CUSTODY_GUEST_ROOT+'/scripts/ops/owner-beta-paper-evidence-local-integration.py'
    worker=['/usr/bin/python3',harness,'--execute','--vm-id',vm_id,'--lane','systemd_restart_crash',
        '--crash-worker','--run-root',case,'--boundary','STARTED']
    launch=['/usr/bin/systemd-run','--unit='+unit,'--property=Type=exec','--property=Restart=no',
        '--property=PrivateNetwork=yes','--property=NoNewPrivileges=yes','--property=ProtectSystem=strict',
        '--property=ReadWritePaths='+case,'--property=UMask=0077','--property=KillMode=control-group',
        '--property=TimeoutStopSec=5s','--property=SendSIGKILL=yes',*worker]
    controller=['/usr/bin/python3','-B',harness,'--execute','--vm-id',vm_id,'--lane','systemd_restart_crash',
        '--systemd-custody-intent',root+'/custody-intent.json','--systemd-custody-fault',CUSTODY_FAULT]
    return dict(schema='r5-systemd-custody-intent-v1',synthetic=True,production_eligible=False,scope=CUSTODY_SCOPE,
        vm_id=vm_id,run_id=profile['run_id'],nonce=profile['nonce'],boot_id=boot_id,source_pins=dict(sources),
        artifact_root=root,case_path=case,boundary='STARTED',unit=unit,worker_argv=worker,launch_argv=launch,
        controller_argv=controller,fault=CUSTODY_FAULT)

def custody_validate_intent(value,vm_id,boot_id,sources,profile):
    expected=custody_intent(vm_id,boot_id,sources,profile)
    if type(value) is not dict or set(value)!=set(expected) or custody_canonical(value)!=custody_canonical(expected):
        raise ValueError('CUSTODY_INTENT_MISMATCH')
    return value

def custody_transition(intent,phase,**extra):
    fields={'LAUNCH_PENDING':set(),'LAUNCH_RETURNED':{'command'},'API_IDENTITY_OBSERVED':{'observation','checkpoint'}}
    if phase not in fields or set(extra)!=fields[phase]:raise ValueError('CUSTODY_TRANSITION_FIELDS')
    return dict(schema='r5-systemd-custody-result-v1',phase=phase,unit=intent['unit'],nonce=intent['nonce'],
        intent_sha256=hashlib.sha256(custody_canonical(intent)).hexdigest(),**extra)

def custody_command_observation(result,error):
    stdout=result.stdout if result is not None else getattr(error,'stdout',None)
    stderr=result.stderr if result is not None else getattr(error,'stderr',None)
    def fingerprint(raw):
        raw=raw if isinstance(raw,bytes) else b''
        return dict(sha256=hashlib.sha256(raw).hexdigest(),length=len(raw))
    return dict(state='RETURNED' if error is None else 'TIMEOUT' if isinstance(error,subprocess.TimeoutExpired) else 'ERROR',
        returncode=result.returncode if result is not None else None,stdout=fingerprint(stdout),stderr=fingerprint(stderr),
        output_complete=error is None and isinstance(stdout,bytes) and isinstance(stderr,bytes),
        error=None if error is None else type(error).__name__)

def custody_checkpoint(value):
    if (type(value) is not dict or set(value)!={'synthetic','boundary','pid','events'}
        or value['synthetic'] is not True or value['boundary']!='STARTED'
        or type(value['pid']) is not int or value['pid']<=0 or type(value['events']) is not int or value['events']!=1):
        raise ValueError('CUSTODY_CHECKPOINT_INVALID')
    return value

def custody_observed_checkpoint(intent,observed,checkpoint):
    custody_checkpoint(checkpoint)
    values=custody_unit_values(observed,intent['unit'],Path(intent['case_path']),intent['worker_argv'])
    if values.get('LoadState')!='loaded' or values.get('MainPID')!=str(checkpoint['pid']):
        raise ValueError('CUSTODY_CHECKPOINT_PID_JOIN')
    return checkpoint


def custody_validate_transitions(intent,transitions):
    phases=['LAUNCH_PENDING','LAUNCH_RETURNED','API_IDENTITY_OBSERVED']
    if type(transitions) is not list or len(transitions)!=3:raise ValueError('CUSTODY_TRANSITIONS_MISSING')
    for value,phase in zip(transitions,phases):
        if type(value) is not dict:raise ValueError('CUSTODY_TRANSITION_INVALID')
        extra={} if phase=='LAUNCH_PENDING' else {'command':value.get('command')} if phase=='LAUNCH_RETURNED' else {'observation':value.get('observation'),'checkpoint':value.get('checkpoint')}
        if custody_canonical(value)!=custody_canonical(custody_transition(intent,phase,**extra)):
            raise ValueError('CUSTODY_TRANSITION_INVALID')
    result=transitions[1]['command']
    if (type(result) is not dict or set(result)!={'state','returncode','stdout','stderr','output_complete','error'}
        or result['state']!='RETURNED' or type(result['returncode']) is not int or result['returncode']!=0
        or result['output_complete'] is not True or result['error'] is not None):
        raise ValueError('CUSTODY_LAUNCH_UNKNOWN')
    for name in ('stdout','stderr'):
        fp=result[name]
        if (type(fp) is not dict or set(fp)!={'sha256','length'} or type(fp['length']) is not int or fp['length']<0
            or type(fp['sha256']) is not str or re.fullmatch('[0-9a-f]{64}',fp['sha256']) is None):
            raise ValueError('CUSTODY_COMMAND_FINGERPRINT')
    custody_observed_checkpoint(intent,transitions[2]['observation'],transitions[2]['checkpoint'])
    return transitions

def custody_validate_result(value,intent):
    keys={'schema','synthetic','production_eligible','intent_sha256','unit','launch_status','status','final','errors'}
    if (type(value) is not dict or set(value)!=keys or value['schema']!='r5-systemd-custody-result-v1'
        or value['synthetic'] is not True or value['production_eligible'] is not False
        or value['intent_sha256']!=hashlib.sha256(custody_canonical(intent)).hexdigest() or value['unit']!=intent['unit']
        or value['launch_status']!='RETURNED_ZERO_AND_CHECKPOINT_OBSERVED'
        or value['status']!='API_STATE_TERMINAL_OBSERVED' or value['errors']!=[]):
        raise ValueError('CUSTODY_CLEANUP_UNKNOWN')
    final=custody_unit_values(value['final'],intent['unit'],Path(intent['case_path']),intent['worker_argv'])
    if final['LoadState']!='not-found' and not (final.get('ActiveState')=='inactive' and final.get('MainPID')=='0' and final.get('ControlPID')=='0'):
        raise ValueError('CUSTODY_API_NOT_TERMINAL')
    return value

def custody_self_fault(intent):
    if intent.get('scope')!=CUSTODY_SCOPE or intent.get('fault')!=CUSTODY_FAULT:raise ValueError('CUSTODY_FAULT_REFUSED')
    w.require(sys.platform=='linux' and os.geteuid()==0 and str(Path(__file__))==CUSTODY_GUEST_ROOT+'/scripts/ops/owner-beta-paper-evidence-local-integration.py')
    os.kill(os.getpid(),signal.SIGKILL)
    raise ValueError('CUSTODY_FAULT_DID_NOT_TERMINATE')

def custody_cli(args):
    active=bool(args.systemd_custody_intent is not None or args.systemd_custody_fault is not None or args.systemd_custody_cleanup_only or args.systemd_custody_cleanup_deadline is not None)
    if not active:return False
    if (not args.execute or args.lane!='systemd_restart_crash' or not args.systemd_custody_intent
        or args.crash_worker or args.run_root or args.boundary or args.image_id or args.pg_config or args.allow_clock_mutation or args.allow_suspend):
        raise ValueError('CUSTODY_CLI_CONFLICT')
    if args.systemd_custody_cleanup_only:
        deadline=args.systemd_custody_cleanup_deadline
        if args.systemd_custody_fault is not None or type(deadline) is not float or not math.isfinite(deadline):
            raise ValueError('CUSTODY_CLEANUP_DEADLINE_INVALID')
    elif args.systemd_custody_fault!=CUSTODY_FAULT or args.systemd_custody_cleanup_deadline is not None:
        raise ValueError('CUSTODY_FAULT_REFUSED')
    return True

def custody_boot_id():
    fd=os.open('/proc/sys/kernel/random/boot_id',os.O_RDONLY|os.O_CLOEXEC|os.O_NOFOLLOW)
    try:value=os.read(fd,129).decode('ascii').strip()
    finally:os.close(fd)
    if not custody_uuid(value):raise ValueError('CUSTODY_BOOT_INVALID')
    return value

def custody_context(args):
    w.require(sys.platform=='linux' and os.geteuid()==0 and str(Path(__file__))==CUSTODY_GUEST_ROOT+'/scripts/ops/owner-beta-paper-evidence-local-integration.py')
    path=Path(args.systemd_custody_intent)
    w.require(custody_uuid(args.vm_id) and path.is_absolute() and len(path.parts)==6 and path.name=='custody-intent.json'
        and custody_uuid(path.parent.name) and path==BASE/args.vm_id/path.parent.name/'custody-intent.json')
    for directory in (BASE,BASE/args.vm_id,path.parent):
        with w.opened(directory,directory=True,exact=(0,0,0o700)):pass
    raw=w.read_file(path,32768,exact=(0,0,0o600));intent=w.decode(raw,32768,False)
    marker=w.decode(w.read_file('/run/lagrange-disposable-paper-vm.json',16384,exact=(0,0,0o600)))
    w.require(custody_canonical(marker)==custody_canonical(dict(schema='lagrange-disposable-paper-vm-v1',vm_id=args.vm_id,production=False,provider_network=False,resources={})))
    sources={name:hashlib.sha256(w.read_file(ROOT/name,1024*1024,exact=(0,0,0o444))).hexdigest() for name in CUSTODY_SOURCES}
    profile={name:intent[name] for name in ('run_id','nonce','boundary','fault')}
    custody_validate_intent(intent,args.vm_id,custody_boot_id(),sources,profile);w.require(raw==custody_canonical(intent))
    args._custody_intent=intent
    if not args.systemd_custody_cleanup_only:
        publish(path.parent/'inputs.json',dict(schema='paper-disposable-inputs-v1',synthetic=True,vm_id=args.vm_id,lane=args.lane,
            source_pins={name:'sha256:'+sources[name] for name in CUSTODY_SOURCES[:3]}))
    return path.parent,{}

def custody_read_transitions(intent):
    case=Path(intent['case_path'])
    return [w.decode(w.read_file(case/name,16384,exact=(0,0,0o600)),16384,False)
        for name in ('launch-pending.json','launch-returned.json','api-identity-observed.json')]

def custody_cleanup_argv(intent,deadline):
    if type(deadline) not in (float,int) or not math.isfinite(deadline) or deadline<=0:
        raise ValueError('CUSTODY_CLEANUP_DEADLINE_INVALID')
    return [*intent['controller_argv'][:-2],'--systemd-custody-cleanup-only',
        '--systemd-custody-cleanup-deadline',str(deadline)]


def custody_cleanup_main(args):
    root,_=custody_context(args);intent=args._custody_intent;deadline=args.systemd_custody_cleanup_deadline
    final=None;errors=[];known=False
    try:custody_validate_transitions(intent,custody_read_transitions(intent));known=True
    except Exception as error:errors.append(w.error_tag(error))
    try:
        w.require(0<deadline-time.monotonic()<=20,'UNIT_CLEANUP_UNKNOWN')
        final=cleanup_unit(intent['unit'],Path(intent['case_path']),intent['worker_argv'],not known,deadline=deadline)
    except Exception as error:errors.append(w.error_tag(error))
    status='API_STATE_TERMINAL_OBSERVED' if known and not errors else 'UNKNOWN'
    result=dict(schema='r5-systemd-custody-result-v1',synthetic=True,production_eligible=False,
        intent_sha256=hashlib.sha256(custody_canonical(intent)).hexdigest(),unit=intent['unit'],
        launch_status='RETURNED_ZERO_AND_CHECKPOINT_OBSERVED' if known else 'UNKNOWN',status=status,final=final,errors=errors)
    publish(root/'custody-result.json',result);print(custody_canonical(result).decode('ascii'))
    return 0 if status=='API_STATE_TERMINAL_OBSERVED' else 2



SUSPEND_SCOPE='R5_G2A_SUSPEND_DISPOSABLE_ONLY'
SUSPEND_OUTCOME_SCHEMA='r5-disposable-suspend-outcome-v1'
SUSPEND_KERNEL='6.8.0-147-generic'
SUSPEND_VERSION='rtcwake from util-linux 2.39.3'
SUSPEND_STAGES=('before','timer','command','after','suspend_check','guard','publication','operation','close')
SUSPEND_SUPPORT_FIELDS={'status','reason','kernel_release','tool','version','rtc','rtc_sysfs','wakeup','states','mem_sleep','error'}
SUSPEND_ENVELOPE_FIELDS={'schema','vm_id','lane','synthetic','production_eligible','artifact_root','status','support','outcome'}
SUSPEND_SUMMARY_FIELDS={'schema','envelope','envelope_publication','receipt_publication'}

class _SuspendOutcome:
    """Private whole Python results; this observation holder grants no authority."""
    def __init__(self):
        self.stages={name:dict(state='NOT_RUN',value=None,error=None) for name in SUSPEND_STAGES}
        self.observation=None

def _suspend_note(outcome,stage,value=None,*,error=None):
    if outcome is not None:
        outcome.stages[stage]=dict(state='ERROR' if error is not None else 'RETURNED',value=value,error=error)

def suspend_error(error):
    if error is None:return None
    if isinstance(error,subprocess.TimeoutExpired):code='COMMAND_TIMEOUT'
    elif isinstance(error,w.Rejected) and str(error) in ('CLOCK_INVALID','SUSPEND_NOT_OBSERVED','SUSPEND_NOT_REJECTED'):
        code=str(error)
    elif isinstance(error,OSError):code='OS_ERROR'
    elif isinstance(error,w.Rejected):code='REJECTED'
    else:code='CALL_FAILED'
    number=getattr(error,'errno',None)
    return dict(code=code,errno=number if type(number) is int else None)

def suspend_wire(outcome):
    stages={}
    for name,item in outcome.stages.items():
        value=item['value']
        if item['state']=='RETURNED':
            if name=='timer':value='ACQUIRED'
            elif name=='command':value=dict(sha256=hashlib.sha256(value).hexdigest(),length=len(value)) if type(value) is bytes else 'UNEXPECTED_RETURN'
            elif name not in ('before','after','operation'):value=None if value is None else 'UNEXPECTED_RETURN'
        else:value=None
        stages[name]=dict(state=item['state'],value=value,error=suspend_error(item['error']))
    return dict(stages=stages,observation=outcome.observation)

class _SuspendUnsupported(Exception):
    pass

def _suspend_read(path):
    if path not in ('/sys/class/rtc/rtc0/dev','/sys/class/rtc/rtc0/device/power/wakeup',
                    '/sys/power/state','/sys/power/mem_sleep'):
        raise ValueError('SUSPEND_READ_PATH')
    fd=os.open(path,os.O_RDONLY|os.O_CLOEXEC|os.O_NOFOLLOW)
    try:
        raw=os.read(fd,257)
        if not 0<len(raw)<=256:raise ValueError('SUSPEND_READ_SIZE')
        return raw.decode('ascii','strict').strip()
    finally:os.close(fd)

def _suspend_tool():
    path='/usr/sbin/rtcwake';before=os.lstat(path)
    if (not stat.S_ISREG(before.st_mode) or before.st_uid!=0 or before.st_mode&0o022
        or not before.st_mode&0o111 or not 0<before.st_size<=16*1024*1024):
        raise _SuspendUnsupported('TOOL_METADATA_UNSUPPORTED')
    raw=w.read_file(path,16*1024*1024)
    after=os.lstat(path)
    fields=('st_dev','st_ino','st_mode','st_uid','st_gid','st_size','st_mtime_ns','st_ctime_ns')
    if any(getattr(before,k)!=getattr(after,k) for k in fields) or len(raw)!=before.st_size:
        raise ValueError('SUSPEND_TOOL_CHANGED')
    return dict(sha256=hashlib.sha256(raw).hexdigest(),length=len(raw),uid=before.st_uid,
                gid=before.st_gid,mode=stat.S_IMODE(before.st_mode))

def suspend_prerequisites():
    value=dict(status='UNKNOWN',reason=None,kernel_release=None,tool=None,version=None,
               rtc=None,rtc_sysfs=None,wakeup=None,states=None,mem_sleep=None,error=None)
    try:
        release=os.uname().release
        if type(release) is not str or len(release)>128:raise ValueError('SUSPEND_KERNEL_INVALID')
        value['kernel_release']=release
        if release!=SUSPEND_KERNEL:raise _SuspendUnsupported('KERNEL_UNSUPPORTED')
        value['tool']=_suspend_tool()
        raw=command(['/usr/sbin/rtcwake','--version'],timeout=5)
        value['version']=dict(sha256=hashlib.sha256(raw).hexdigest(),length=len(raw))
        if raw!= (SUSPEND_VERSION+'\n').encode('ascii'):raise _SuspendUnsupported('TOOL_VERSION_UNSUPPORTED')
        meta=os.stat('/dev/rtc0',follow_symlinks=False)
        if not stat.S_ISCHR(meta.st_mode):raise _SuspendUnsupported('RTC_NODE_UNSUPPORTED')
        value['rtc']=dict(major=os.major(meta.st_rdev),minor=os.minor(meta.st_rdev))
        text=_suspend_read('/sys/class/rtc/rtc0/dev')
        if re.fullmatch(r'[0-9]{1,10}:[0-9]{1,10}',text) is None:raise ValueError('SUSPEND_RTC_DEV_INVALID')
        major,minor=(int(part) for part in text.split(':'));value['rtc_sysfs']=dict(major=major,minor=minor)
        if value['rtc']!=value['rtc_sysfs']:raise _SuspendUnsupported('RTC_DEVICE_MISMATCH')
        value['wakeup']=_suspend_read('/sys/class/rtc/rtc0/device/power/wakeup')
        if value['wakeup'] not in ('enabled','disabled'):raise ValueError('SUSPEND_WAKEUP_INVALID')
        if value['wakeup']!='enabled':raise _SuspendUnsupported('WAKEUP_DISABLED')
        states=_suspend_read('/sys/power/state').split()
        if not states or len(states)>32 or any(re.fullmatch('[a-z_]+',x) is None for x in states):
            raise ValueError('SUSPEND_STATES_INVALID')
        value['states']=states
        if 'mem' not in states:raise _SuspendUnsupported('MEM_UNSUPPORTED')
        modes=_suspend_read('/sys/power/mem_sleep').split()
        if (not modes or len(modes)>16 or any(re.fullmatch(r'(?:[a-z][a-z0-9_]*|\[[a-z][a-z0-9_]*\])',x) is None for x in modes)
            or sum(x.startswith('[') for x in modes)!=1):raise ValueError('SUSPEND_MODES_INVALID')
        value['mem_sleep']=modes
        if '[deep]' not in modes:raise _SuspendUnsupported('DEEP_NOT_SELECTED')
        value['status']='CHECKED_PREREQUISITES'
    except FileNotFoundError as error:
        value.update(status='NOT_SUPPORTED',reason='PREREQUISITE_ABSENT',error=suspend_error(error))
    except _SuspendUnsupported as error:
        value.update(status='NOT_SUPPORTED',reason=str(error))
    except Exception as error:
        value.update(status='UNKNOWN',reason='PREREQUISITE_READ_OR_PARSE',error=suspend_error(error))
    return value

def suspend_cli(args):
    if not getattr(args,'suspend_component',False):return False
    if (not args.execute or args.lane!='suspend' or args.allow_suspend is not True
        or args.allow_clock_mutation or args.crash_worker or args.run_root is not None
        or args.boundary is not None or args.image_id is not None or args.pg_config is not None
        or args.systemd_custody_intent is not None or args.systemd_custody_fault is not None
        or args.systemd_custody_cleanup_only or args.systemd_custody_cleanup_deadline is not None):
        raise ValueError('SUSPEND_CLI_CONFLICT')
    return True

def suspend_envelope_clean(value):
    support=value['support'];stages=value['outcome']['stages']
    return (support['status']=='CHECKED_PREREQUISITES'
        and all(stages[k]['state']=='RETURNED' and stages[k]['error'] is None
                for k in ('before','timer','command','after','suspend_check','publication','operation','close'))
        and stages['operation']['value']=='PASS' and stages['close']['value'] is None
        and stages['guard']['state']=='ERROR' and stages['guard']['error']==dict(code='CLOCK_INVALID',errno=None)
        and value['outcome']['observation'] is not None)

def suspend_validate_summary(value,vm_id):
    if (type(value) is not dict or set(value)!=SUSPEND_SUMMARY_FIELDS or value['schema']!=SUSPEND_OUTCOME_SCHEMA):
        raise ValueError('SUSPEND_SUMMARY_FIELDS')
    envelope=value['envelope']
    if (type(envelope) is not dict or set(envelope)!=SUSPEND_ENVELOPE_FIELDS
        or envelope['schema']!=SUSPEND_OUTCOME_SCHEMA or envelope['vm_id']!=vm_id or envelope['lane']!='suspend'
        or envelope['synthetic'] is not True or envelope['production_eligible'] is not False
        or envelope['status'] not in ('PASS','INCOMPLETE')):
        raise ValueError('SUSPEND_ENVELOPE_FIELDS')
    support=envelope['support'];outcome=envelope['outcome']
    if type(support) is not dict or set(support)!=SUSPEND_SUPPORT_FIELDS or support['status'] not in ('CHECKED_PREREQUISITES','NOT_SUPPORTED','UNKNOWN'):
        raise ValueError('SUSPEND_SUPPORT_FIELDS')
    if type(outcome) is not dict or set(outcome)!={'stages','observation'} or type(outcome['stages']) is not dict or set(outcome['stages'])!=set(SUSPEND_STAGES):
        raise ValueError('SUSPEND_STAGES')
    for item in list(outcome['stages'].values())+[value['envelope_publication'],value['receipt_publication']]:
        if (type(item) is not dict or set(item)!={'state','value','error'}
            or item['state'] not in ('NOT_RUN','RETURNED','ERROR')):
            raise ValueError('SUSPEND_STAGE_FIELDS')
        error=item['error']
        if error is not None and (type(error) is not dict or set(error)!={'code','errno'}
            or error['code'] not in ('COMMAND_TIMEOUT','CLOCK_INVALID','SUSPEND_NOT_OBSERVED','SUSPEND_NOT_REJECTED','OS_ERROR','REJECTED','CALL_FAILED')
            or (error['errno'] is not None and type(error['errno']) is not int)):
            raise ValueError('SUSPEND_ERROR_FIELDS')
        if ((item['state']=='ERROR')!=(error is not None)
            or (item['state']=='NOT_RUN' and item['value'] is not None)):
            raise ValueError('SUSPEND_STAGE_STATE')
    if (envelope['status']=='PASS') != suspend_envelope_clean(envelope):raise ValueError('SUSPEND_STATUS')
    return value

def suspend_component(args,root):
    outcome=_SuspendOutcome();support=suspend_prerequisites()
    if support['status']=='CHECKED_PREREQUISITES':
        try:suspend_lane(root,_outcome=outcome)
        except Exception:pass  # whole original and close objects already retained by SAME lane
    envelope=dict(schema=SUSPEND_OUTCOME_SCHEMA,vm_id=args.vm_id,lane='suspend',synthetic=True,
                  production_eligible=False,artifact_root=str(root),status='INCOMPLETE',support=support,
                  outcome=suspend_wire(outcome))
    if suspend_envelope_clean(envelope):envelope['status']='PASS'
    companion=_SuspendOutcome()
    for name,path,body in (
        ('publication',root/'suspend-outcome.json',envelope),
        ('close',root/'receipt.json',dict(schema='paper-disposable-result-v1',synthetic=True,production_eligible=False,
                                        lane='suspend',status='PASS' if envelope['status']=='PASS' else 'FAIL_OR_UNKNOWN',overall='INCOMPLETE'))):
        try:returned=publish(path,body);_suspend_note(companion,name,returned)
        except Exception as error:_suspend_note(companion,name,error=error)
    wire=suspend_wire(companion)['stages']
    summary=dict(schema=SUSPEND_OUTCOME_SCHEMA,envelope=envelope,envelope_publication=wire['publication'],
                 receipt_publication=wire['close'])
    print(w.canonical(summary).decode('ascii'))
    return 0 if envelope['status']=='PASS' and all(summary[k]['state']=='RETURNED' for k in ('envelope_publication','receipt_publication')) else 2


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--execute',action='store_true');p.add_argument('--vm-id');p.add_argument('--lane',choices=LANES)
    p.add_argument('--image-id');p.add_argument('--pg-config');p.add_argument('--allow-clock-mutation',action='store_true');p.add_argument('--allow-suspend',action='store_true')
    p.add_argument('--crash-worker',action='store_true',help=argparse.SUPPRESS);p.add_argument('--run-root',help=argparse.SUPPRESS);p.add_argument('--boundary',choices=BOUNDARIES,help=argparse.SUPPRESS)
    p.add_argument('--systemd-custody-intent',help=argparse.SUPPRESS)
    p.add_argument('--systemd-custody-fault',help=argparse.SUPPRESS)
    p.add_argument('--systemd-custody-cleanup-only',action='store_true',help=argparse.SUPPRESS)
    p.add_argument('--systemd-custody-cleanup-deadline',type=float,help=argparse.SUPPRESS)
    p.add_argument('--suspend-component',action='store_true',help=argparse.SUPPRESS)
    args=p.parse_args()
    suspend_cli(args)
    if custody_cli(args) and args.systemd_custody_cleanup_only:return custody_cleanup_main(args)
    results={k:'NOT RUN' for k in LANES};root=None
    if args.execute:
        if args.suspend_component:
            w.require(str(Path(__file__))=='/opt/lagrange-g2a-source/scripts/ops/owner-beta-paper-evidence-local-integration.py','ORIGIN_UNPROVEN')
            root,_=context(args);return suspend_component(args,root)
        root,resources=context(args)
        if args.crash_worker:
            w.require(args.lane=='systemd_restart_crash' and args.boundary);return crash_worker(args,root)
        try:
            if args.lane=='clock_step_cancel':w.require(args.allow_clock_mutation);status=clock_cancel(root)
            elif args.lane=='suspend':w.require(args.allow_suspend);status=suspend_lane(root)
            elif args.lane=='docker_lifetime':
                w.require(args.image_id in resources.get('docker_image_ids',[]));status=docker_lifetime(args.image_id,root)
            elif args.lane=='systemd_restart_crash':status=systemd_crashes(args,root)
            elif args.lane=='pg_roles_concurrency':status=pg_roles(args,resources,root)
            elif args.lane=='synthetic_producer_db':status=producer_db(args,resources,root)
            else:status=kernel_fd(root) if args.lane=='kernel_timerfd' else inherited_fd(root)
            results[args.lane]=status
            publish(root/'receipt.json',dict(schema='paper-disposable-result-v1',synthetic=True,production_eligible=False,lane=args.lane,status=status,overall='INCOMPLETE'))
        except Exception:
            publish(root/'failed.json',dict(schema='paper-disposable-result-v1',synthetic=True,production_eligible=False,lane=args.lane,status='FAIL_OR_UNKNOWN',overall='INCOMPLETE'))
            raise
    print(json.dumps({'schema':'owner-beta-paper-isolated-proof-v1','synthetic':True,'production_eligible':False,'lanes':results,'overall':'INCOMPLETE','artifacts':str(root) if root else None},sort_keys=True))
    return 0
if __name__=='__main__':
    try:raise SystemExit(main())
    except (w.Rejected,OSError,ValueError,KeyError,subprocess.SubprocessError):
        print('PAPER_DISPOSABLE_INTEGRATION_REJECTED',file=sys.stderr);raise SystemExit(1) from None
