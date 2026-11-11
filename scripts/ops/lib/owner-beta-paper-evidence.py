#!/usr/bin/env python3
"""Protected future-slot source witness. Root/kernel/Docker are the trust base.

No --run-now or supplied-success path. A decoder/fixture is never authority.
Runtime entry requires root, an installed protected binding and the inherited
socket held by the still-alive timer owner. Error messages are finite codes.
"""
from __future__ import annotations

import argparse
import contextlib
import ctypes
import datetime as dt
import fcntl
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import select
import signal
import socket
import stat
import struct
import subprocess
import sys
import time
import uuid

sys.dont_write_bytecode = True
SOURCE_LIMIT, SESSION_LIMIT, DEPTH = 65536, 262144, 16
SCHEMA_PATH = Path(__file__).resolve().parents[3] / 'docs/contracts/owner-beta-paper-source-evidence-v1.schema.json'
SCHEMA = json.loads(SCHEMA_PATH.read_bytes())
CURRENT_SCHEMA = json.loads(SCHEMA_PATH.with_name('owner-beta-paper-current-binding-v1.schema.json').read_bytes())
CURRENT_LEAF = 'current-binding.json'
REFRESH_SECONDS = 15  # leave headroom inside the contract's 30-second maximum
# Linux UAPI asm-generic/fcntl.h: Python may omit the named OFD constants even
# though fcntl(2) exposes them. The syscall still fails closed on unsupported FS.
OFD_SETLK = getattr(fcntl, 'F_OFD_SETLK', 37) if sys.platform == 'linux' else None
ENV = {'PATH':'/usr/sbin:/usr/bin:/sbin:/bin','HOME':'/root','LANG':'C.UTF-8',
       'PYTHONDONTWRITEBYTECODE':'1','DOCKER_HOST':'unix:///var/run/docker.sock'}
KST = dt.timezone(dt.timedelta(hours=9))
TERM_GRACE, KILL_GRACE, CONTAINER_CLEANUP = 5, 5, 20
RELAY_HORIZON = 43200  # existing maximum source horizon, never a cleanup timeout

class Rejected(Exception):
    pass

def require(ok, code='SOURCE_INVALID'):
    if not ok:
        raise Rejected(code)

def error_tag(error):
    codes=set(SCHEMA['$defs']['failure']['properties']['code']['enum'])|{'UNIT_OWNERSHIP_UNPROVEN','UNIT_CLEANUP_UNKNOWN','CLEANUP_UNKNOWN','REPLACED'}
    return str(error) if isinstance(error,Rejected) and str(error) in codes else type(error).__name__

def digest(raw):
    return 'sha256:' + hashlib.sha256(raw).hexdigest()

def canonical(value):
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(',', ':'), allow_nan=False).encode()

def pairs(items):
    out = {}
    for k, v in items:
        require(k not in out)
        out[k] = v
    return out

def depth(value, level=1):
    require(level <= DEPTH)
    if isinstance(value, dict):
        for child in value.values(): depth(child, level + 1)
    elif isinstance(value, list):
        for child in value: depth(child, level + 1)
    elif isinstance(value, float):
        raise Rejected('SOURCE_INVALID')

def decode(raw, limit=SOURCE_LIMIT, canonical_required=True):
    require(0 < len(raw) <= limit)
    try:
        value = json.loads(raw, object_pairs_hook=pairs,
                           parse_constant=lambda _: (_ for _ in ()).throw(Rejected('SOURCE_INVALID')))
        depth(value)
        require(not canonical_required or canonical(value) == raw)
        return value
    except (ValueError, UnicodeError, RecursionError):
        raise Rejected('SOURCE_INVALID') from None

def formatted(value, kind):
    if kind == 'sha256': return re.fullmatch(r'sha256:[0-9a-f]{64}', value) is not None
    if kind in ('hex64', 'commit'): return re.fullmatch('[0-9a-f]{%d}' % (64 if kind == 'hex64' else 40), value) is not None
    if kind == 'uuid':
        try: return str(uuid.UUID(value)) == value and uuid.UUID(value).int != 0
        except ValueError: return False
    if kind == 'date':
        try: return dt.date.fromisoformat(value).isoformat() == value
        except ValueError: return False
    if kind == 'db-timestamp':
        try:
            parsed=dt.datetime.fromisoformat(value.replace('Z','+00:00'))
            return re.fullmatch(r'\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d{1,9})?(?:Z|[+-]\d\d:\d\d)',value) is not None and parsed.tzinfo is not None and parsed.year>1970 and len(value)<=35
        except ValueError:return False
    if kind == 'date-time':
        try:
            parsed = dt.datetime.fromisoformat(value.replace('Z', '+00:00'))
            return re.fullmatch(r'\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d(?:\.\d{1,9})?Z',value) is not None and parsed.year > 1970
        except ValueError: return False
    if kind == 'leaf': return re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._-]{0,191}', value) is not None and value != 'batch.json'
    return False

def validate(value, rule=None):
    """Enforce the deliberately small shared JSON-schema vocabulary."""
    rule = SCHEMA if rule is None else rule
    if '$ref' in rule:
        return validate(value, SCHEMA['$defs'][rule['$ref'].split('/')[-1]])
    for key in ('oneOf','anyOf'):
        if key in rule:
            count = 0
            for branch in rule[key]:
                try: validate(value, branch); count += 1
                except Rejected: pass
            require(count == 1 if key == 'oneOf' else count >= 1)
            return
    if 'const' in rule: require(type(value) is type(rule['const']) and value == rule['const'])
    if 'enum' in rule: require(value in rule['enum'])
    kind = rule.get('type')
    if kind == 'object':
        require(isinstance(value, dict) and set(value) == set(rule['required']))
        for key, child in value.items(): validate(child, rule['properties'][key])
    elif kind == 'array':
        require(isinstance(value, list) and len(value) <= rule['maxItems'])
        for child in value: validate(child, rule['items'])
    elif kind == 'integer': require(type(value) is int and rule['minimum'] <= value <= rule['maximum'])
    elif kind == 'boolean': require(type(value) is bool)
    elif kind == 'null': require(value is None)
    elif kind == 'string':
        require(isinstance(value, str) and rule['minLength'] <= len(value) <= rule['maxLength'] and value.isascii() and all(ord(c) >= 32 for c in value))
        if 'format' in rule: require(formatted(value, rule['format']))

def due_ns(session):
    return int(dt.datetime.combine(dt.date.fromisoformat(session), dt.time(16,30), KST).timestamp()) * 10**9

def slot_id(cohort, session):
    return digest(canonical({'schema':'owner-beta-paper-slot-v1','owner':cohort['owner_user_id'],
        'policy':cohort['policy_sha256'],'cohort':cohort['cohort_sha256'],
        'session':session,'due_realtime_ns':due_ns(session)}))

def dispatch_key(cohort, session):
    # Excludes cohort intentionally: a config/release change cannot re-dispatch.
    return digest(canonical([cohort['owner_user_id'],cohort['policy_sha256'],session]))[7:]

def sample():
    return dict(realtime_ns=time.clock_gettime_ns(time.CLOCK_REALTIME),
                boottime_ns=time.clock_gettime_ns(time.CLOCK_BOOTTIME),
                monotonic_ns=time.clock_gettime_ns(time.CLOCK_MONOTONIC))

def clock_step(before, after):
    require(all(after[k] >= before[k] for k in before), 'CLOCK_INVALID')
    delta = {k:after[k]-before[k] for k in before}
    require(abs(delta['realtime_ns']-delta['boottime_ns']) <= 10**9 and
            abs(delta['boottime_ns']-delta['monotonic_ns']) <= 10**9, 'CLOCK_INVALID')

def original_hash(originals):
    keys = ['|'.join(o[k] for k in ('provider','market','batch_id','file_name','wire_sha256')) for o in originals]
    identities=[tuple(o[k] for k in ('provider','market','batch_id','file_name')) for o in originals]
    require(len(keys) == 23 and identities == sorted(set(identities)))
    return digest(('\n'.join(keys)+'\n').encode('ascii'))

def verify_chain(events, terminal=None):
    require(1 <= len(events) <= 64 and len(canonical({'schema':'owner-beta-paper-source-chain-v1','events':events})) <= SESSION_LIMIT)
    previous = None
    stages = ['STARTED','ARM_INTENT','ARMED','RESERVED','DB_RESERVED','LAUNCH_INTENT','RUNNING','SOURCE_FINISHED','ADMISSION_LINKED','RUN_LINKED','LINKED']
    stage = -1; phases=[]; raw=None; source=None; containers=[]
    first = events[0]
    for index, event in enumerate(events):
        validate(event, SCHEMA['$defs']['event'])
        require(len(canonical(event)) <= SOURCE_LIMIT and event['sequence'] == index and event['previous_sha256'] == previous)
        for k in ('slot_id','session','due_realtime_ns','cohort','arm','reservation_id','invocation_id'):
            require(event[k] == first[k])
        require(event['slot_id'] == slot_id(event['cohort'],event['session']) and event['due_realtime_ns'] == due_ns(event['session']))
        if index:
            clock_step(events[index-1]['observed'],event['observed'])
            clock_step(events[0]['observed'],event['observed'])
        kind, data, now, due = event['kind'],event['data'],event['observed']['realtime_ns'],event['due_realtime_ns']
        if kind == 'FAILED':
            require(index == len(events)-1)
            stage = -2
        elif kind == 'CONTAINER':
            require(stage >= 3 and stage < 10)
            phase = data['phase']
            require(data['exit_code'] == 0 and data['oom_killed'] is False)
            require(data['image_id'] == event['cohort']['research_image_id' if phase in ('precheck','capture','action') else 'helper_image_id'])
            require(data['id'] not in containers); containers.append(data['id'])
            if phase in ('precheck','capture','action'):
                require(stage == 6 and phase == ['precheck','capture','action'][len(phases)])
                phases.append(phase)
        elif kind == 'RAW_SOURCE':
            require(stage == 6 and raw is None and phases == ['precheck','capture'])
            require(data['target_date'] == event['session'] and data['reused_existing_source'] is False and data['calendar_claim_preexisting'] is False)
            require(data['batch_id'] != data['calendar_batch_id']); raw=data
        else:
            require(stage >= -1 and stage+1 < len(stages) and kind == stages[stage+1]); stage += 1
            if kind in ('ARM_INTENT','ARMED'): require(due-now >= 60*10**9,'MISSED_UNARMED')
            if kind in ('RESERVED','DB_RESERVED','LAUNCH_INTENT','RUNNING'): require(due <= now <= due+60*10**9,'MISSED_LATE')
            if kind == 'RUNNING':
                require(data['child']['pid']!=event['arm']['parent']['pid'] and data['child']['start_ticks']>=event['arm']['parent']['start_ticks'])
            if kind == 'SOURCE_FINISHED':
                require(raw is not None and phases == ['precheck','capture','action'])
                require(original_hash(data['originals']) == data['original_set_sha256'])
                require(all(o['batch_date'] == event['session'] for o in data['originals']))
                require(sum(o['batch_id'] == raw['batch_id'] for o in data['originals']) == 22)
                require(sum(o['batch_id'] == raw['calendar_batch_id'] for o in data['originals']) == 1)
                launched=next(e['observed']['realtime_ns'] for e in events if e['kind']=='RUNNING')
                for original in data['originals']:
                    retrieved=int(dt.datetime.fromisoformat(original['retrieved_at'].replace('Z','+00:00')).timestamp()*10**9)
                    require(launched-10**9 <= retrieved <= now+10**9)
                source=(previous, digest(canonical(event)),data['original_set_sha256'])
            if kind == 'ADMISSION_LINKED': require(source and data['source_event_sha256'] == source[1] and data['original_set_sha256'] == source[2])
            if kind == 'LINKED': require(now < due+int(7.5*3600*10**9))
        previous = digest(canonical(event))
    if terminal is not None: require(events[-1]['kind'] == terminal, 'ORIGIN_UNPROVEN')
    return previous

# Open every component relative to a held directory descriptor. Never follow a
# symlink; reject writable ancestors and POSIX ACLs (including inherited ACLs).
def no_acl(fd):
    require(not any(k.startswith('system.posix_acl_') for k in os.listxattr(fd)), 'ORIGIN_UNPROVEN')

@contextlib.contextmanager
def opened(path, *, directory=False, owners=(0,), exact=None, write=False):
    path = str(path)
    require(path.startswith('/') and path != '/' and all(p not in ('','.','..') for p in path[1:].split('/')), 'ORIGIN_UNPROVEN')
    handles=[]
    try:
        current=os.open('/',os.O_RDONLY|os.O_DIRECTORY|os.O_CLOEXEC); handles.append(current)
        parts=path[1:].split('/')
        for index, part in enumerate(parts):
            is_dir=index < len(parts)-1 or directory
            flags=(os.O_RDWR if write and not is_dir else os.O_RDONLY)|os.O_NOFOLLOW|os.O_CLOEXEC
            if is_dir: flags |= os.O_DIRECTORY
            current=os.open(part,flags,dir_fd=current); handles.append(current)
            m=os.fstat(current); no_acl(current)
            require(m.st_uid in owners and m.st_mode & 0o022 == 0, 'ORIGIN_UNPROVEN')
            require(stat.S_ISDIR(m.st_mode) if is_dir else stat.S_ISREG(m.st_mode) and m.st_nlink == 1,'ORIGIN_UNPROVEN')
        if exact:
            require((m.st_uid,m.st_gid,stat.S_IMODE(m.st_mode)) == exact,'ORIGIN_UNPROVEN')
        yield current
        if not directory and not write:
            after=os.fstat(current)
            require((after.st_dev,after.st_ino,after.st_uid,after.st_gid,after.st_mode,after.st_nlink,after.st_size,after.st_mtime_ns,after.st_ctime_ns)==(m.st_dev,m.st_ino,m.st_uid,m.st_gid,m.st_mode,m.st_nlink,m.st_size,m.st_mtime_ns,m.st_ctime_ns),'ORIGIN_UNPROVEN')
    except OSError:
        raise Rejected('ORIGIN_UNPROVEN') from None
    finally:
        for fd in reversed(handles): os.close(fd)

def read_file(path, limit, **kw):
    with opened(path,**kw) as fd:
        size=os.fstat(fd).st_size; require(0 < size <= limit)
        data=os.read(fd,limit+1); require(len(data) == size)
        return data

def publish(parent, name, raw, control=False):
    require('/' not in name and len(raw) <= SOURCE_LIMIT)
    exact=(0,0,0o700) if control else (0,10001,0o750)
    with opened(parent,directory=True,exact=exact) as fd:
        temporary='.'+str(uuid.uuid4())
        leaf=os.open(temporary,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o600,dir_fd=fd)
        try:
            os.fchown(leaf,0,0 if control else 10001); os.fchmod(leaf,0o600 if control else 0o440)
            with os.fdopen(os.dup(leaf),'wb') as out: out.write(raw); out.flush(); os.fsync(out.fileno())
            # Atomic no-replace publication. Readers reject the transient second
            # hardlink; the directory is fsynced only after it is removed.
            os.link(temporary,name,src_dir_fd=fd,dst_dir_fd=fd,follow_symlinks=False)
            os.unlink(temporary,dir_fd=fd); os.fsync(fd)
        finally:
            os.close(leaf)
            with contextlib.suppress(FileNotFoundError): os.unlink(temporary,dir_fd=fd)

def process(pid):
    text=Path(f'/proc/{pid}/stat').read_text(); rest=text[text.rindex(')')+2:].split()
    return {'pid':pid,'start_ticks':int(rest[19])}

def descendant(pid, parent):
    for _ in range(12):
        if pid == parent['pid']: return process(pid) == parent
        if pid <= 1: return False
        text=Path(f'/proc/{pid}/stat').read_text(); pid=int(text[text.rindex(')')+2:].split()[1])
    return False

def remaining(deadline, cap=5):
    value=min(cap,deadline-time.monotonic())
    require(value>0,'UNKNOWN')
    return value

class SigchldAction(ctypes.Structure):
    # glibc Linux x86-64 bits/sigaction.h; queried only, never changed.
    _fields_=[('handler',ctypes.c_void_p),('mask',ctypes.c_ulong*16),('flags',ctypes.c_int),('restorer',ctypes.c_void_p)]

def unreaped_child_policy():
    # The cleanup-only numeric-PID fallback needs zombies to remain allocated
    # and no competing reaper. Normal source ownership still requires pidfd.
    require(sys.platform=='linux' and os.uname().machine=='x86_64'
        and ctypes.sizeof(SigchldAction)==152 and SigchldAction.flags.offset==136
        and (os.confstr('CS_GNU_LIBC_VERSION') or '').startswith('glibc '),'ORIGIN_UNPROVEN')
    require(len(list(Path('/proc/self/task').iterdir()))==1
        and signal.getsignal(signal.SIGCHLD)==signal.SIG_DFL,'ORIGIN_UNPROVEN')
    libc=ctypes.CDLL(None,use_errno=True);action=SigchldAction()
    libc.sigaction.argtypes=[ctypes.c_int,ctypes.c_void_p,ctypes.POINTER(SigchldAction)]
    libc.sigaction.restype=ctypes.c_int
    require(libc.sigaction(signal.SIGCHLD,None,ctypes.byref(action))==0
        and action.handler is None and not action.flags&2,'ORIGIN_UNPROVEN')  # SA_NOCLDWAIT

# Strong custody for UNKNOWN rollback, including callers without a Journal.
# Operational diagnostics only: never read by eligibility/source validators.
UNRESOLVED_CHILDREN=[]

class OwnedChild:
    """Direct Popen child; never reap the session leader before group cleanup.

    WNOWAIT keeps its PID/session ID allocated, so killpg cannot target a later
    reused group. A separate session is used only for our wrapper; relay's
    Docker client stays in that session and is signalled via its exact pidfd.
    Root/kernel and the pinned wrapper (which does not daemonize) are trusted.
    """
    def __init__(self, child, group=False, *, observe=True):
        # This non-observing prefix cannot fail on proc/pidfd I/O. Launchers
        # assign it before acquire(), retaining the exact returned Popen.
        self.child=child;self.group=group;self.fd=-1;self.reaped=False
        self.identity=None;self.lineage=None;self.parent=os.getpid();self.acquired=False
        if observe:self.acquire()
    def acquire(self):
        try:
            self.identity=process(self.child.pid)
            self.fd=os.pidfd_open(self.child.pid,0)
            self.check();self.acquired=True
        except BaseException as error:
            error.child_custody=self
            try:self.close()
            except Exception as failure:error.custody_close_error=error_tag(failure)
            raise
    def diagnostic(self):
        return dict(pid=self.child.pid,parent=self.parent,start_ticks=None if self.identity is None else self.identity['start_ticks'],
            group=self.group,lineage=self.lineage,acquired=self.acquired,reaped=self.reaped,
            returncode=self.child.returncode,status='CLEAN' if self.reaped else 'UNKNOWN')
    def rollback_state(self):
        require(self.child.returncode is None and self.child.pid>1,'ORIGIN_UNPROVEN')
        # ECHILD is a refusal, not absence/success. WNOWAIT never frees the PID.
        status=os.waitid(os.P_PID,self.child.pid,os.WEXITED|os.WNOHANG|os.WNOWAIT)
        require(status is None or status.si_pid==self.child.pid,'ORIGIN_UNPROVEN')
        raw=Path(f'/proc/{self.child.pid}/stat').read_text();parts=raw[raw.rindex(')')+2:].split()
        identity=dict(pid=self.child.pid,start_ticks=int(parts[19]))
        lineage=dict(parent=int(parts[1]),pgrp=int(parts[2]),session=int(parts[3]))
        require(lineage['parent']==self.parent==os.getpid(),'ORIGIN_UNPROVEN')
        require(self.identity is None or identity==self.identity,'ORIGIN_UNPROVEN')
        require(self.lineage is None or lineage==self.lineage,'ORIGIN_UNPROVEN')
        if self.group:require(lineage['pgrp']==lineage['session']==self.child.pid,'ORIGIN_UNPROVEN')
        self.identity=identity;self.lineage=lineage
        return status is not None
    def rollback(self):
        # Hold CPython Popen's reap mutex, reject other threads/auto-reaping,
        # and retain the direct child's PID (including a zombie leader) until
        # its group is quiescent. No poll()/send_signal() may reap behind us.
        deadline=time.monotonic()+TERM_GRACE+KILL_GRACE;settled=False
        mutex=self.child._waitpid_lock
        require(mutex.acquire(timeout=remaining(deadline)),'UNKNOWN')
        try:
            unreaped_child_policy()
            for sig,grace in ((signal.SIGTERM,TERM_GRACE),(signal.SIGKILL,KILL_GRACE)):
                exited=self.rollback_state()
                settled=exited and (not self.group or not self.live_group())
                if not settled:
                    unreaped_child_policy();self.rollback_state()
                    if self.group:os.killpg(self.child.pid,sig)
                    else:os.kill(self.child.pid,sig)
                until=min(deadline,time.monotonic()+grace)
                while not settled:
                    exited=self.rollback_state()
                    settled=exited and (not self.group or not self.live_group())
                    if settled:break
                    delay=until-time.monotonic()
                    if delay<=0:break
                    time.sleep(min(.05,delay))
                if settled:break
        finally:mutex.release()
        require(settled,'UNKNOWN')
        # No further PID/group signal is possible after releasing the mutex.
        self.child.wait(timeout=remaining(deadline));self.reaped=True
    def check(self):
        require(process(self.child.pid)==self.identity,'ORIGIN_UNPROVEN')
        raw=Path(f'/proc/{self.child.pid}/stat').read_text();parts=raw[raw.rindex(')')+2:].split()
        require(int(parts[1])==os.getpid(),'ORIGIN_UNPROVEN')
        if self.group:require(int(parts[2])==int(parts[3])==self.child.pid,'ORIGIN_UNPROVEN')
    def exited(self):
        return os.waitid(os.P_PIDFD,self.fd,os.WEXITED|os.WNOHANG|os.WNOWAIT) is not None
    def live_group(self):
        self.check()
        for entry in Path('/proc').iterdir():
            if not entry.name.isdecimal():continue
            try:
                raw=(entry/'stat').read_text();parts=raw[raw.rindex(')')+2:].split()
            except FileNotFoundError:continue
            if int(parts[2])==self.child.pid and int(parts[3])==self.child.pid and parts[0] not in ('Z','X'):return True
        return False
    def send(self, sig):
        self.check()
        if self.group:os.killpg(self.child.pid,sig)
        else:signal.pidfd_send_signal(self.fd,sig)
    def settled(self):
        return self.exited() and (not self.group or not self.live_group())
    def wait(self, timeout):
        deadline=time.monotonic()+timeout
        while not self.settled():time.sleep(min(.05,remaining(deadline)))
        result=self.child.wait(timeout=remaining(deadline))
        self.reaped=True;return result
    def cleanup(self):
        if self.reaped:return
        if self.fd<0:return self.rollback()
        for sig,grace in ((signal.SIGTERM,TERM_GRACE),(signal.SIGKILL,KILL_GRACE)):
            if not self.settled():self.send(sig)
            try:self.wait(grace);return
            except Rejected as error:
                if str(error)!='UNKNOWN':raise
            except subprocess.TimeoutExpired:pass
        raise Rejected('UNKNOWN')  # retained identity, never an unbounded reap
    def close(self):
        if self.fd>=0:
            fd=self.fd;self.fd=-1;os.close(fd)  # Linux close must never be retried

def child_failure(original,owned,errors):
    """Preserve original cause and exact custody; never stringify command data."""
    errors=list(errors)
    if hasattr(original,'custody_close_error'):errors.append('pidfd:'+original.custody_close_error)
    result=Rejected('UNKNOWN') if errors else original
    result.original_failure=error_tag(original);result.cleanup_errors=errors
    if isinstance(original,OSError):result.original_errno=original.errno
    if owned is not None:
        result.child_custody=owned
        if errors and not owned.reaped and owned not in UNRESOLVED_CHILDREN:UNRESOLVED_CHILDREN.append(owned)
    return result

def close_child(owned,child,errors):
    for label,resource in [('pidfd',owned),('stdout',None if child is None else child.stdout),('stderr',None if child is None else child.stderr)]:
        if resource is not None:
            try:resource.close()
            except Exception as failure:errors.append(label+':'+error_tag(failure))

def read_child_output(owned, deadline, limit=32768):
    stream=owned.child.stdout;fd=stream.fileno();os.set_blocking(fd,False);output=bytearray()
    while True:
        ready=select.select([fd],[],[],remaining(deadline,.25))[0]
        if not ready:continue
        try:part=os.read(fd,4096)
        except BlockingIOError:continue
        if not part:break
        output.extend(part);require(len(output)<=limit)
    return bytes(output),owned.wait(remaining(deadline,RELAY_HORIZON))

class OwnedContainer:
    """An absent-before-launch UUID intent, then a frozen full CID/Created pair."""
    def __init__(self,docker,name,image,launched,verify):
        self.docker=docker;self.name=name;self.image=image;self.launched=launched
        self.verify=verify;self.pin=None;self.removed=False;self.removal_requested=False;self.wait_errors=[]
        self.launch_uncertain=False;self.observed_id=None
    def observe(self,deadline):
        ids=self.docker.call(['ps','--all','--no-trunc','--filter','name=^/'+self.name+'$','--format','{{.ID}}'],timeout=remaining(deadline)).splitlines()
        if not ids:
            require(self.pin is None or self.removal_requested,'ORIGIN_UNPROVEN');return None
        require(len(ids)==1 and formatted(ids[0],'hex64'),'ORIGIN_UNPROVEN')
        self.observed_id=ids[0]
        if self.pin:require(ids[0]==self.pin[0],'ORIGIN_UNPROVEN')
        value=self.docker.object('container',ids[0],timeout=remaining(deadline))
        require(value.get('Id')==ids[0] and value.get('Name')=='/'+self.name and value.get('Image')==self.image,'ORIGIN_UNPROVEN')
        created=value.get('Created');require(isinstance(created,str),'ORIGIN_UNPROVEN')
        stamp=int(dt.datetime.fromisoformat(created.replace('Z','+00:00')).timestamp()*10**9)
        require(self.launched-10**9<=stamp<=time.time_ns()+10**9,'ORIGIN_UNPROVEN')
        self.verify(value)
        pin=(ids[0],created);require(self.pin is None or self.pin==pin,'ORIGIN_UNPROVEN');self.pin=pin
        return value
    def remove(self,deadline):
        final=self.observe(deadline);require(final is not None and final.get('State',{}).get('Running') is False,'UNKNOWN')
        self.removal_requested=True
        self.docker.call(['rm',self.pin[0]],timeout=remaining(deadline))
        require(self.observe(deadline) is None,'UNKNOWN')
        self.removed=True
    def cleanup(self,record):
        if self.removed:return
        deadline=time.monotonic()+CONTAINER_CLEANUP;value=self.observe(deadline)
        if value is None:
            require(not self.launch_uncertain or self.removal_requested,'UNKNOWN')
            self.removed=self.removal_requested;return
        for sig,grace in (('TERM',TERM_GRACE),('KILL',KILL_GRACE)):
            if not value.get('State',{}).get('Running'):break
            self.docker.call(['kill','--signal='+sig,self.pin[0]],timeout=remaining(deadline))
            try:self.docker.call(['wait',self.pin[0]],timeout=remaining(deadline,grace))
            except Exception as error:
                # A timed-out wait is not terminal evidence. Retain its finite
                # type and require a fresh exact-object observation below.
                self.wait_errors.append(type(error).__name__)
            value=self.observe(deadline);require(value is not None,'UNKNOWN')
        require(value.get('State',{}).get('Running') is False,'UNKNOWN')
        record(value)  # evidence must be durable before removal, even on failure
        self.remove(deadline)

class Timer:
    def __init__(self, due):
        libc=ctypes.CDLL(None,use_errno=True)
        self.fd=libc.timerfd_create(time.CLOCK_REALTIME,os.O_CLOEXEC|os.O_NONBLOCK)
        require(self.fd >= 0,'CLOCK_INVALID')
        class Spec(ctypes.Structure): _fields_=[('interval_sec',ctypes.c_long),('interval_nsec',ctypes.c_long),('sec',ctypes.c_long),('nsec',ctypes.c_long)]
        spec=Spec(0,0,due//10**9,due%10**9)
        if libc.timerfd_settime(self.fd,3,ctypes.byref(spec),None) != 0:
            self.close(); raise Rejected('CLOCK_INVALID')
    def consume(self):
        try: require(struct.unpack('Q',os.read(self.fd,8))[0] == 1,'CLOCK_INVALID')
        except OSError: raise Rejected('CLOCK_INVALID') from None
    def close(self):
        if self.fd >= 0:
            fd=self.fd;self.fd=-1;os.close(fd)

class Journal:
    def __init__(self, binding, session):
        self.binding=binding; self.session=session; self.events=[]
        c=binding['cohort']; self.slot=slot_id(c,session)
        self.directory=Path(binding['evidence_root'])/'events'/self.slot[7:]
        with opened(self.directory.parent,directory=True,exact=(0,10001,0o750)) as parent:
            os.mkdir(self.directory.name,0o750,dir_fd=parent)
            os.chown(self.directory.name,0,10001,dir_fd=parent,follow_symlinks=False); os.chmod(self.directory.name,0o750,dir_fd=parent,follow_symlinks=False); os.fsync(parent)
        self.common=dict(schema='owner-beta-paper-source-event-v1',slot_id=self.slot,session=session,
            due_realtime_ns=due_ns(session),cohort=c,arm={'boot_id':Path('/proc/sys/kernel/random/boot_id').read_text().strip(),
            'generation':str(uuid.uuid4()),'nonce':str(uuid.uuid4()),'parent':process(os.getpid())},
            reservation_id=str(uuid.uuid4()),invocation_id=str(uuid.uuid4()))
    def append(self, kind, data=None):
        event=dict(self.common,sequence=len(self.events),previous_sha256=digest(canonical(self.events[-1])) if self.events else None,
                   observed=sample(),kind=kind,data={} if data is None else data)
        validate(event,SCHEMA['$defs']['event']); require(len(canonical(event)) <= SOURCE_LIMIT)
        publish(self.directory,f'{len(self.events):06}.json',canonical(event)); self.events.append(event)
        return digest(canonical(event))
    def claim(self):
        publish(Path(self.binding['evidence_root'])/'.control',dispatch_key(self.binding['cohort'],self.session)+'.json',canonical(self.common),True)

# The binding is installed OFF and contains only nonsecret identities/paths.
# Paper account/consent and activation are deliberately not manufactured here.
BINDING_KEYS={'schema','cohort','evidence_root','release_root','production_env_file','production_env_sha256','calendar_dir','calendar_file','release_manifest_file','data_root','raw_owner_uid','daily_config_file','daily_config_sha256','secret_root','database','project','network','max_source_seconds','state_dir','lock_file'}
def public_mount_paths(value):
    # Private release/calendar originals stay with root. Only these existing
    # public mounts and the new current directory are visible to the helper.
    return [value['evidence_root']+'/current',value['evidence_root']+'/events',
        value['data_root']+'/raw',value['data_root']+'/owner-beta-daily/admitted',value['data_root']+'/owner-beta-daily/historical-base/admitter',
        value['daily_config_file'],value['release_root']+'/.owner-beta-daily/policy.json']

def binding_structure(value):
    require(set(value)==BINDING_KEYS and value['schema']=='owner-beta-paper-witness-binding-v1')
    validate(value,SCHEMA['$defs']['binding'])
    validate(value['cohort'],SCHEMA['$defs']['cohort'])
    require(type(value['raw_owner_uid']) is int and value['raw_owner_uid'] in (10001,10002))
    require(type(value['max_source_seconds']) is int and 1 <= value['max_source_seconds'] <= 43200)
    require(formatted(value['daily_config_sha256'],'sha256') and formatted(value['production_env_sha256'],'sha256'))
    for key in ('database','project','network'): require(re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_-]{0,62}',value[key]) is not None)
    for key in ('evidence_root','release_root','production_env_file','calendar_dir','calendar_file','release_manifest_file','data_root','daily_config_file','secret_root','state_dir','lock_file'):
        require(isinstance(value[key],str) and value[key].startswith('/') and not any(x in ('','.','..') for x in value[key][1:].split('/')))
    require(value['release_root'].endswith('/'+value['cohort']['verifier_revision']))
    require(value['evidence_root']==value['data_root']+'/owner-beta-paper-evidence')
    require(value['daily_config_file']==value['release_root']+'/.owner-beta-daily/config.json')
    require(value['release_manifest_file']==value['release_root']+'/.lagrange-release-manifest')
    for public in public_mount_paths(value):
        for private in (value['production_env_file'],value['secret_root']):
            require(private!=public and not private.startswith(public+'/'),'CONFIG_CHANGED')

def structural_binding(path):
    """Discover the protected control path; never mint authority here."""
    raw=read_file(path,SOURCE_LIMIT,exact=(0,0,0o600));value=decode(raw);binding_structure(value)
    require(all(str(path)!=p and not str(path).startswith(p+'/') for p in public_mount_paths(value)),'CONFIG_CHANGED')
    return value,raw

def semantic_hash(value):
    material=json.loads(canonical(value));del material['cohort']['semantic_config_sha256'];del material['cohort']['cohort_sha256']
    return digest(canonical(material))

def validate_current(value):
    # The shared small validator does not implement allOf/if. Enforce the
    # frozen envelope's conditional rules explicitly, using identical defs.
    require(all(CURRENT_SCHEMA['$defs'][k]==SCHEMA['$defs'][k] for k in ('binding','cohort','clock','process')))
    validate(value,CURRENT_SCHEMA);depth(value);require(len(canonical(value))<=SOURCE_LIMIT)
    if value['status']=='CURRENT':require(value['reason']=='NONE' and value['binding'] is not None)
    else:require(value['reason']!='NONE')
    require((value['binding'] is None)==(value['binding_sha256'] is None))
    if value['binding'] is not None:
        binding_structure(value['binding'])
        require(value['binding_sha256']==digest(canonical(value['binding'])))
        require(semantic_hash(value['binding'])==value['binding']['cohort']['semantic_config_sha256'])

def _binding_assets(path):
    value,raw=structural_binding(path)
    require(digest(read_file(value['production_env_file'],131072,exact=(0,0,0o600)))==value['production_env_sha256'])
    require(digest(read_file(value['calendar_file'],16*1024*1024)) == value['cohort']['calendar_sha256'])
    require(digest(read_file(value['release_manifest_file'],1024*1024)) == value['cohort']['release_manifest_sha256'])
    require(digest(read_file(value['daily_config_file'],16384)) == value['daily_config_sha256'])
    spec=importlib.util.spec_from_file_location('paper_daily_operations',Path(__file__).with_name('owner-beta-daily-operations.py'))
    operations=importlib.util.module_from_spec(spec);sys.modules[spec.name]=operations;spec.loader.exec_module(operations)
    images=operations.read_manifest(value['release_root'],value['cohort']['verifier_revision'])
    require(images['research-worker']==value['cohort']['research_image_id'] and images['owner-beta-runner']==value['cohort']['helper_image_id'])
    config,config_hash=operations.install.verify_config(value['release_root'],value['cohort']['verifier_revision'])
    require(config_hash==value['daily_config_sha256'] and config['owner']==value['cohort']['owner_user_id'] and config['raw_owner_uid']==value['raw_owner_uid'])
    # Bind the nonsecret source configuration. Exclude both recursive digest
    # fields: the later complete Paper cohort includes this semantic digest.
    # Slot/DB equality independently binds that complete cohort; none is minted.
    require(semantic_hash(value) == value['cohort']['semantic_config_sha256'])
    return value,raw,operations

def binding(path):
    value,raw,_operations=_binding_assets(path)
    return value,raw

class _HelperProjection:
    """Frozen private metadata; neither a wire value nor native authority."""
    __slots__=('binding_sha256','production_env_sha256','cohort_sha256','worker_historical_root')
    def __init__(self,binding_sha256,production_env_sha256,cohort_sha256,worker_historical_root):
        for name,value in zip(self.__slots__,(binding_sha256,production_env_sha256,cohort_sha256,worker_historical_root)):
            object.__setattr__(self,name,value)
    def __setattr__(self,name,value):
        raise AttributeError('immutable helper projection')
    def __delattr__(self,name):
        raise AttributeError('immutable helper projection')
    def __eq__(self,other):
        return type(other) is _HelperProjection and all(getattr(self,k)==getattr(other,k) for k in self.__slots__)
    def __repr__(self):
        return '_HelperProjection(<redacted>)'

def _helper_projection_matches(projection,selected,raw):
    return (type(projection) is _HelperProjection and canonical(selected)==raw
        and projection.binding_sha256==digest(raw)
        and projection.production_env_sha256==selected['production_env_sha256']
        and projection.cohort_sha256==selected['cohort']['cohort_sha256'])

def _binding_with_helper_projection(path):
    value,raw,operations=_binding_assets(path)
    values,overlays=operations.read_context(value['release_root'],value['cohort']['verifier_revision'],
        'quiescent',expected_env_sha256=value['production_env_sha256'])
    require(values['LAGRANGE_CODE_COMMIT']==value['cohort']['verifier_revision']
        and values['LAGRANGE_DATA_DIR']==value['data_root']
        and values['LAGRANGE_RUNTIME_SECRET_DIR']==value['secret_root']
        and (values['POSTGRES_DB'] or 'lagrange')==value['database'] and overlays==[],'CONFIG_CHANGED')
    artifact_root=values['LAGRANGE_ARTIFACTS_DIR']
    require(operations.runtime.absolute(artifact_root) and ',' not in artifact_root,'CONFIG_CHANGED')
    worker_root=artifact_root+'/historical-price-beta-root'
    for private in (value['production_env_file'],value['secret_root'],str(path)):
        require(private!=worker_root and not private.startswith(worker_root+'/')
            and not worker_root.startswith(private+'/'),'CONFIG_CHANGED')
    projection=_HelperProjection(digest(raw),value['production_env_sha256'],value['cohort']['cohort_sha256'],worker_root)
    return value,raw,projection

class OfdFlock(ctypes.Structure):
    _fields_=[('type',ctypes.c_short),('whence',ctypes.c_short),('start',ctypes.c_longlong),('length',ctypes.c_longlong),('pid',ctypes.c_int)]

class CurrentPublication:
    """Mutable fixed leaf under the HELD global lock, never a journal hardlink."""
    def __init__(self,b,lock_fd):
        self.directory=Path(b['evidence_root'])/'current';self.lock_path=Path(b['evidence_root'])/'.control/lock'
        self.lock_fd=lock_fd;self.lock_identity=self.inode(os.fstat(lock_fd));self.fd=-1;self.sequence=-1
        self.generation=str(uuid.uuid4());self.boot=Path('/proc/sys/kernel/random/boot_id').read_text().strip()
        self.publisher=process(os.getpid());self.check_lock()
        with opened(self.directory,directory=True,exact=(0,10001,0o750)) as directory:self.expected=self.snapshot(directory)
    @staticmethod
    def inode(info):return info.st_dev,info.st_ino
    def check_lock(self):
        require(self.inode(os.fstat(self.lock_fd))==self.lock_identity,'REPLACED')
        with opened(self.lock_path.parent,directory=True,exact=(0,0,0o700)):pass
        with opened(self.lock_path,write=True,exact=(0,0,0o600)) as named:
            require(self.inode(os.fstat(named))==self.lock_identity,'REPLACED')
        fcntl.flock(self.lock_fd,fcntl.LOCK_EX|fcntl.LOCK_NB)
    def snapshot(self,directory):
        try:fd=os.open(CURRENT_LEAF,os.O_RDONLY|os.O_NOFOLLOW|os.O_CLOEXEC,dir_fd=directory)
        except FileNotFoundError:return None
        try:
            info=os.fstat(fd);self.metadata(fd,info)
            raw=os.read(fd,SOURCE_LIMIT+1);require(len(raw)==info.st_size)
            value=decode(raw);validate_current(value)
            after=os.fstat(fd);named=os.stat(CURRENT_LEAF,dir_fd=directory,follow_symlinks=False)
            require((info.st_dev,info.st_ino,info.st_size,info.st_mtime_ns,info.st_ctime_ns)==(after.st_dev,after.st_ino,after.st_size,after.st_mtime_ns,after.st_ctime_ns)
                and self.inode(named)==self.inode(info),'REPLACED')
            return (info.st_dev,info.st_ino,value['generation'],value['sequence'],digest(raw))
        finally:os.close(fd)
    @staticmethod
    def metadata(fd,info):
        no_acl(fd)
        require(stat.S_ISREG(info.st_mode) and (info.st_uid,info.st_gid,stat.S_IMODE(info.st_mode),info.st_nlink)==(0,10001,0o440,1),'ORIGIN_UNPROVEN')
    def check_current(self):
        self.check_lock();require(self.fd>=0 and not os.get_inheritable(self.fd),'ORIGIN_UNPROVEN')
        with opened(self.directory,directory=True,exact=(0,10001,0o750)) as directory:
            require(self.snapshot(directory)==self.expected and self.expected[2]==self.generation
                and self.inode(os.fstat(self.fd))==self.expected[:2],'REPLACED')
    def close(self):
        if self.fd>=0:
            fd=self.fd;self.fd=-1;os.close(fd)
    def emit(self,status,reason,selected,observed=None):
        fd=-1;temporary=None
        try:
            self.check_lock();sequence=self.sequence+1
            # Reserve the last signed-i64 sequence for invalidation.
            require(sequence<=2**63-1 and (status!='CURRENT' or sequence<2**63-1),'UNKNOWN')
            value=dict(schema='owner-beta-paper-current-binding-v1',status=status,reason=reason,generation=self.generation,
                boot_id=self.boot,publisher=self.publisher,sequence=sequence,observed=sample() if observed is None else observed,
                binding=selected,binding_sha256=None if selected is None else digest(canonical(selected)))
            validate_current(value);raw=canonical(value)
            require(OFD_SETLK is not None and ctypes.sizeof(ctypes.c_void_p)==8 and ctypes.sizeof(OfdFlock)==32,'ORIGIN_UNPROVEN')
            with opened(self.directory,directory=True,exact=(0,10001,0o750)) as directory:
                require(self.snapshot(directory)==self.expected,'REPLACED')
                temporary='.'+str(uuid.uuid4())
                try:
                    fd=os.open(temporary,os.O_RDWR|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW|os.O_CLOEXEC,0o600,dir_fd=directory)
                    os.set_inheritable(fd,False);require(not os.get_inheritable(fd),'ORIGIN_UNPROVEN')
                    offset=0
                    while offset<len(raw):
                        written=os.write(fd,raw[offset:]);require(written>0);offset+=written
                    os.fsync(fd)
                    fcntl.fcntl(fd,OFD_SETLK,bytes(OfdFlock(fcntl.F_WRLCK,os.SEEK_SET,0,0,0)))
                    os.fchown(fd,0,10001);os.fchmod(fd,0o440);os.fsync(fd)
                    info=os.fstat(fd);self.metadata(fd,info)
                    self.check_lock();require(self.snapshot(directory)==self.expected,'REPLACED')
                    os.replace(temporary,CURRENT_LEAF,src_dir_fd=directory,dst_dir_fd=directory);temporary=None
                    os.fsync(directory)
                    self.expected=(info.st_dev,info.st_ino,self.generation,sequence,digest(raw));self.sequence=sequence
                    self.close()
                    if status=='CURRENT':self.fd=fd;fd=-1
                finally:
                    if temporary is not None:
                        try:os.unlink(temporary,dir_fd=directory)
                        except FileNotFoundError:pass
            return value
        except BaseException:
            self.close();raise
        finally:
            if fd>=0:os.close(fd)

class CurrentAuthority:
    """Cooperative guarded refresh: no background thread can outlive the lock."""
    def __init__(self,scheduler,lock_fd):
        self.scheduler=scheduler;self.publication=CurrentPublication(scheduler.b,lock_fd)
        self.selected=None;self.fatal=None;self.stopped=False;self.previous=sample();self.initial=self.previous;self.refreshed=None
        self._helper_projection=None
    def start(self):
        try:
            self.publication.emit('INVALID','STARTING',None)
            self.check_assets();self.poll(check_assets=True,starting=True)
        except BaseException as error:
            self.failed(error);raise
    def check_assets(self):
        selected,raw,projection=_binding_with_helper_projection(self.scheduler.path)
        require(raw==self.scheduler.pin and selected==self.scheduler.b,'CONFIG_CHANGED')
        require(_helper_projection_matches(projection,selected,raw),'CONFIG_CHANGED')
        require(self._helper_projection is None or self._helper_projection==projection,'CONFIG_CHANGED')
        self.selected=selected
        self._helper_projection=projection
    def helper_projection(self):
        require(not self.stopped and self.fatal is None and self.selected is not None
            and self.selected==self.scheduler.b
            and _helper_projection_matches(self._helper_projection,self.selected,self.scheduler.pin),'CONFIG_CHANGED')
        return self._helper_projection
    def poll(self,check_assets=False,starting=False):
        require(not self.stopped and self.fatal is None,'CONFIG_CHANGED')
        try:
            now=sample();clock_step(self.previous,now);clock_step(self.initial,now);self.previous=now
            require(Path('/proc/sys/kernel/random/boot_id').read_text().strip()==self.publication.boot,'CLOCK_INVALID')
            due=self.refreshed is None or now['monotonic_ns']-self.refreshed>=REFRESH_SECONDS*10**9
            if self.refreshed is not None:require(now['monotonic_ns']-self.refreshed<=30*10**9,'UNKNOWN')
            if check_assets or due:self.check_assets()
            after=sample();clock_step(now,after);require(after['monotonic_ns']-now['monotonic_ns']<=15*10**9,'UNKNOWN')
            require(Path('/proc/sys/kernel/random/boot_id').read_text().strip()==self.publication.boot,'CLOCK_INVALID')
            if not starting:self.publication.check_current()
            if due:
                if self.refreshed is not None:require(after['monotonic_ns']-self.refreshed<=30*10**9,'UNKNOWN')
                self.publication.emit('CURRENT','NONE',self.selected,after);self.refreshed=after['monotonic_ns']
            self.previous=after
        except BaseException as error:
            self.failed(error);raise
    def failed(self,error):
        # Preserve the triggering guard error even if INVALID cannot be written.
        # revoke's finally closes the fence before any owned-resource cleanup.
        try:self.revoke(error_tag(error))
        except Exception as failure:self.scheduler.cleanup_errors.append('current:'+error_tag(failure))
    def revoke(self,reason='STOPPED'):
        self._helper_projection=None
        if self.stopped:return
        self.stopped=True;self.fatal=reason
        reason=reason if reason in ('STARTING','STOPPED','CONFIG_CHANGED','CLOCK_INVALID','REPLACED') else 'UNKNOWN'
        try:self.publication.emit('INVALID',reason,self.selected)
        finally:self.publication.close()


def guarded_command(argv,timeout,tick,on_failure,*,stdin=None):
    """Serial local command. Refresh while polling both bounded output pipes."""
    child=None;owned=None;failure=None;errors=[];result=None
    try:
        tick()
        child=subprocess.Popen(argv,env=ENV,stdin=subprocess.DEVNULL if stdin is None else stdin,stdout=subprocess.PIPE,stderr=subprocess.PIPE,close_fds=True,pass_fds=())
        owned=OwnedChild(child,observe=False);owned.acquire();deadline=time.monotonic()+timeout
        streams={child.stdout.fileno():bytearray(),child.stderr.fileno():bytearray()};active=list(streams)
        for fd in active:os.set_blocking(fd,False)
        while active or not owned.settled():
            tick();wait=remaining(deadline,.25)
            for fd in select.select(active,[],[],wait)[0]:
                try:part=os.read(fd,4096)
                except BlockingIOError:continue
                if part:streams[fd].extend(part);require(len(streams[fd])<=2*1024*1024)
                else:active.remove(fd)
        result=subprocess.CompletedProcess(argv,owned.wait(remaining(deadline)),bytes(streams[child.stdout.fileno()]),bytes(streams[child.stderr.fileno()]))
        tick()
    except BaseException as error:
        failure=error
        try:on_failure(error)
        except Exception as cleanup:errors.append('invalidate:'+error_tag(cleanup))
        if owned is not None:
            try:owned.cleanup()
            except Exception as cleanup:errors.append('child:'+error_tag(cleanup))
    finally:
        close_child(owned,child,errors)
    if errors and failure is None:
        failure=Rejected('UNKNOWN')
        try:on_failure(failure)
        except Exception as cleanup:errors.append('invalidate:'+error_tag(cleanup))
    if failure is not None:
        result=child_failure(failure,owned,errors)
        if result is failure:raise failure
        raise result from failure
    return result


def load_runtime():
    spec=importlib.util.spec_from_file_location('paper_daily_runtime',Path(__file__).with_name('owner-beta-daily-runtime.py'))
    module=importlib.util.module_from_spec(spec); sys.modules[spec.name]=module; spec.loader.exec_module(module)
    return module

def source_environment(b, fd):
    # Use the installed, hashed paths. Ambient service/terminal overrides must
    # not silently select a different state file or collector lock.
    return dict(ENV,LAGRANGE_ENV_FILE=b['production_env_file'],LAGRANGE_XKRX_CALENDAR_DIR=b['calendar_dir'],
                KIS_DAILY_STATE_DIR=b['state_dir'],KIS_DAILY_LOCK_FILE=b['lock_file'],
                BACKFILL_CONFIRM_EXTERNAL='I_UNDERSTAND_READ_ONLY_KIS_CALLS',LAGRANGE_CODE_COMMIT=b['cohort']['verifier_revision'],
                LAGRANGE_PAPER_WITNESS_FD=str(fd))

def docker_observation(value, phase, argv_sha, client, expected_image):
    state=value.get('State',{}); config=value.get('Config',{}); host=value.get('HostConfig',{})
    require(value.get('Image') == expected_image and state.get('Status') == 'exited' and state.get('Running') is False
            and type(state.get('OOMKilled')) is bool and type(state.get('ExitCode')) is int and state.get('ExitCode') >= 0 and not state.get('Error')
            and host.get('AutoRemove') is False,'ORIGIN_UNPROVEN')
    # Hash only the allowlisted nonsecret configuration. Never hash/record Env.
    semantic={k:config.get(k) for k in ('User','Entrypoint','Cmd','Image')}
    semantic.update({k:host.get(k) for k in ('ReadonlyRootfs','Privileged','CapAdd','CapDrop','SecurityOpt','GroupAdd','NetworkMode')})
    result=dict(phase=phase,id=value['Id'],image_id=value['Image'],config_sha256=digest(canonical(semantic)),
        created_at=value['Created'],started_at=state['StartedAt'],finished_at=state['FinishedAt'],
        exit_code=state['ExitCode'],oom_killed=state['OOMKilled'],argv_sha256=argv_sha,client=client)
    validate(result,SCHEMA['$defs']['container'])
    times=[dt.datetime.fromisoformat(result[k].replace('Z','+00:00')) for k in ('created_at','started_at','finished_at')]
    require(times==sorted(times),'ORIGIN_UNPROVEN')
    return result

def raw_originals(b, raw):
    """Exact IDs from child IPC, never directory diff/date proximity discovery."""
    result=[]; root=Path(b['data_root'])/'raw'; uid=b['raw_owner_uid']
    for bid, kinds in ((raw['batch_id'],{'bars','reference'}),(raw['calendar_batch_id'],{'calendar'})):
        found=[]
        for provider in ('kis','kis-calendar'):
            manifest=root/'manifests'/f'provider={provider}'/'market=kr'/'manifest.jsonl'
            if not manifest.exists(): continue
            with opened(manifest.with_name('commit.lock'),owners=(0,uid),write=True) as lock:
                fcntl.flock(lock,fcntl.LOCK_SH|fcntl.LOCK_NB)
                wire=read_file(manifest,8388608,owners=(0,uid))
                for line in wire.splitlines():
                    record=decode(line,262144,False)
                    if record.get('batch_id') != bid: continue
                    require(record.get('provider') == provider and record.get('market')=='kr' and record.get('date')==raw['target_date'])
                    found.append(bid)
                    folder=root/f'provider={provider}'/'market=kr'/f"date={raw['target_date']}"/f'batch={bid}'
                    meta=read_file(folder/'batch.json',262144,owners=(0,uid)); require(decode(meta,262144,False)==record)
                    for f in record['files']:
                        if f['kind'] not in kinds: continue
                        require(not f.get('copied_from') and formatted(f['file_name'],'leaf'))
                        contents=read_file(folder/f['file_name'],131072,owners=(0,uid))
                        require(digest(contents)==f['content_hash'] and len(contents)==f['size_bytes'])
                        result.append(dict(provider=provider,market='kr',batch_id=bid,batch_date=record['date'],
                            file_name=f['file_name'],wire_sha256=f['content_hash'],size_bytes=f['size_bytes'],
                            batch_json_sha256=digest(meta),manifest_line_sha256=digest(line),retrieved_at=record['retrieved_at']))
                fcntl.flock(lock,fcntl.LOCK_UN)
        require(found == [bid])
    result.sort(key=lambda o:tuple(o[k] for k in ('provider','market','batch_id','file_name','wire_sha256')))
    original_hash(result)
    for value in result: validate(value,SCHEMA['$defs']['original'])
    return result

def ipc_request(message):
    # An environment number is navigation only. A real socket, credentialled
    # response and live root parent holding the other endpoint are required.
    channel=None;duplicate=None
    try:
        fd=int(os.environ['LAGRANGE_PAPER_WITNESS_FD'])
        require(fd >= 3 and stat.S_ISSOCK(os.fstat(fd).st_mode),'ORIGIN_UNPROVEN')
        duplicate=os.dup(fd);channel=socket.socket(fileno=duplicate);duplicate=None
        channel.setsockopt(socket.SOL_SOCKET,socket.SO_PASSCRED,1)
        channel.settimeout(60)
        channel.send(canonical(message))
        raw, ancillary, flags, _=channel.recvmsg(SOURCE_LIMIT,128)
        credentials=[struct.unpack('3i',v) for lev,kind,v in ancillary if lev==socket.SOL_SOCKET and kind==socket.SCM_CREDENTIALS]
        require(not flags and len(credentials)==1 and credentials[0][1]==0,'ORIGIN_UNPROVEN')
        reply=decode(raw); require(reply.get('ok') is True,'ORIGIN_UNPROVEN')
        require(reply.get('parent') == process(credentials[0][0]),'ORIGIN_UNPROVEN')
        return reply
    except (KeyError,OSError,ValueError): raise Rejected('ORIGIN_UNPROVEN') from None
    finally:
        if channel is not None:channel.close()
        elif duplicate is not None:os.close(duplicate)

def relay_compose(phase, argv):
    require(phase in ('precheck','capture','action'))
    client=process(os.getpid())
    answer=ipc_request({'op':'begin','phase':phase,'argv':argv,'client':client})
    require(argv.count('--rm')==1)
    argv=list(argv); argv[argv.index('--rm'):argv.index('--rm')+1]=['--name',answer['name']]
    child=None;owned=None;failure=None;errors=[]
    try:
        child=subprocess.Popen(argv,stdin=subprocess.DEVNULL,stdout=subprocess.PIPE,stderr=subprocess.DEVNULL,env=os.environ.copy(),close_fds=True,pass_fds=())
        owned=OwnedChild(child,observe=False);owned.acquire()
        output,rc=read_child_output(owned,time.monotonic()+RELAY_HORIZON)
    except BaseException as error:
        failure=error
        if owned is not None:
            try:owned.cleanup()
            except Exception as cleanup:errors.append('child:'+error_tag(cleanup))
    finally:
        close_child(owned,child,errors)
    if failure is not None or errors:
        failure=failure or Rejected('UNKNOWN');result=child_failure(failure,owned,errors)
        if result is failure:raise failure
        raise result from failure
    ipc_request({'op':'end','phase':phase,'client':client,'exit_code':rc,'output':output.decode('utf-8')})
    sys.stdout.buffer.write(output); sys.stdout.buffer.flush()
    return rc

def relay_raw(record):
    raw={k:record[k] for k in SCHEMA['$defs']['raw']['required']}
    validate(raw,SCHEMA['$defs']['raw'])
    ipc_request({'op':'raw','data':raw})

def action_result(b, output, session):
    lines=output.splitlines(); require(len(lines)==1)
    value=decode(lines[0].encode(),8192,False)
    require(set(value)=={'outcome','start','end','batch_id','reserved_wire_gets','raw_only','admitted'})
    claim=None
    if value['batch_id'] is not None:
        wire=read_file(Path(b['data_root'])/'raw/.r2-action-supply'/f'{session}.json',4096,owners=(0,b['raw_owner_uid']),exact=(b['raw_owner_uid'],b['raw_owner_uid'],0o600))
        claim=decode(wire,4096,False)
        require(claim.get('schema')=='r2-action-supply-claim-v1' and claim.get('day')==session and claim.get('batch_id')==value['batch_id'] and claim.get('reserved_wire_gets')==value['reserved_wire_gets'])
        if value['outcome']=='RAW_CAPTURE_INDEPENDENTLY_VERIFIED':
            require((claim['start'],claim['end'])==(value['start'],value['end']))
        else:
            require(value['outcome']=='VERIFIED_PRIOR_CAPTURE' and value['start'] is None and value['end'] is None)
            value['start'],value['end']=claim['start'],claim['end']
        claim=digest(wire)
    value['claim_sha256']=claim; validate(value,SCHEMA['$defs']['action']); return value

def validate_compose(argv,b,session,phase):
    prefix=['docker','compose','--env-file',b['production_env_file'],'-f',b['release_root']+'/deploy/compose/compose.yml']
    require(argv[:len(prefix)]==prefix,'ORIGIN_UNPROVEN'); remaining=argv[len(prefix):]
    overlays=['compose.intraday.yml','compose.intraday-operational.yml','compose.market-stream.yml']; seen=[]
    while remaining[:1]==['-f']:
        require(len(remaining)>1)
        value=remaining[1]; require(value.startswith(b['release_root']+'/deploy/compose/'))
        leaf=value.rsplit('/',1)[1]; require(leaf in overlays and leaf not in seen)
        seen.append(leaf); remaining=remaining[2:]
    require(seen==sorted(seen,key=overlays.index) and 'compose.intraday.yml' in seen)
    tail={'precheck':['--eod-raw-check-session-dates',session],
          'capture':['--eod-raw-session-dates',session], 'action':['--r2-action-supply']}[phase]
    expected=['run','--rm','--no-deps']+(['-e','RESEARCH_R2_ACTION_SUPPLY_CONFIG_HEX'] if phase=='action' else [])+['research-worker']+tail
    require(remaining==expected,'ORIGIN_UNPROVEN')

def load_lifecycle():
    name='owner_beta_paper_lifecycle'
    if name not in sys.modules:
        spec=importlib.util.spec_from_file_location(name,Path(__file__).with_name('owner-beta-paper-lifecycle.py'))
        module=importlib.util.module_from_spec(spec);sys.modules[name]=module;spec.loader.exec_module(module)
    return sys.modules[name]

def source_lifecycle_barrier(path,authority):
    require(authority is not None,'ORIGIN_UNPROVEN')
    authority.poll(check_assets=True)
    result=load_lifecycle().source_barrier(path,refresh=lambda:authority.poll(check_assets=True))
    authority.poll(check_assets=True)
    return result

class Scheduler:
    def __init__(self,path):
        require(sys.platform=='linux' and os.geteuid()==0,'ORIGIN_UNPROVEN')
        self.path=path; self.b,self.pin=structural_binding(path); self.runtime=load_runtime(); self.docker=self.runtime.Docker()
        self.j=None; self.child=None; self.current=None; self.raw=None; self.action=None; self.phase_index=0
        self.current_container=None;self.owned_child=None;self.cleanup_errors=[];self.authority=None;self.in_cleanup=False
        self.command_custodies=[]
        self.previous_clock=sample(); self.initial_clock=self.previous_clock
    def guard(self):
        require(self.authority is not None,'ORIGIN_UNPROVEN');self.authority.poll(check_assets=True)
        now=sample(); clock_step(self.previous_clock,now); clock_step(self.initial_clock,now); self.previous_clock=now
        require(Path('/proc/sys/kernel/random/boot_id').read_text().strip()==self.j.common['arm']['boot_id'],'CLOCK_INVALID')
        require(read_file(self.path,SOURCE_LIMIT,exact=(0,0,0o600))==self.pin,'CONFIG_CHANGED')
        require(digest(read_file(self.b['production_env_file'],131072,exact=(0,0,0o600)))==self.b['production_env_sha256'],'CONFIG_CHANGED')
        for path,key,limit in ((self.b['calendar_file'],'calendar_sha256',16*1024*1024),(self.b['release_manifest_file'],'release_manifest_sha256',1024*1024)):
            require(digest(read_file(path,limit))==self.b['cohort'][key],'CONFIG_CHANGED')
        if select.select([self.cancel.fd],[],[],0)[0]:
            self.cancel.consume(); raise Rejected('CLOCK_INVALID')
    def invalidate(self,error):
        if self.authority is not None:
            try:self.authority.revoke('STOPPED' if error_tag(error)=='INTERRUPTED' else error_tag(error))
            except Exception as failure:self.cleanup_errors.append('current:'+error_tag(failure))
    @contextlib.contextmanager
    def cleanup_scope(self):
        previous=self.in_cleanup;self.in_cleanup=True
        try:yield
        finally:self.in_cleanup=previous
    def command(self,argv,timeout):
        def tick():
            if not self.in_cleanup:
                require(self.authority is not None,'ORIGIN_UNPROVEN');self.authority.poll()
        try:return guarded_command(argv,timeout,tick,self.invalidate)
        except BaseException as error:
            custody=getattr(error,'child_custody',None)
            if custody is not None and not custody.reaped:
                self.command_custodies.append(custody)
                self.failure_code=getattr(error,'original_failure',error_tag(error))
                try:self.cleanup_note('command','UNKNOWN',errors=getattr(error,'cleanup_errors',()))
                except Exception as failure:
                    # Keep the actual Popen on both Scheduler and exception if
                    # even the private handoff write fails during startup.
                    error.handoff_error=error_tag(failure)
                    self.cleanup_errors.append('handoff:'+error_tag(failure))
            raise
    def docker_call(self,args,timeout=15):
        result=self.command(['/usr/bin/docker','--host','unix:///var/run/docker.sock',*args],timeout)
        self.runtime.require(result.returncode==0,'DAILY_RUNTIME_DOCKER')
        try:return result.stdout.decode('utf-8')
        except UnicodeError:raise self.runtime.RuntimeErrorCode('DAILY_RUNTIME_DOCKER') from None
    def absent(self,name):
        # A failed inspect is ambiguous. Listing succeeds and must contain none.
        ids=self.docker.call(['ps','--all','--no-trunc','--filter','name=^/'+name+'$','--format','{{.ID}}']).splitlines()
        require(not ids,'ORIGIN_UNPROVEN')
    def cleanup_note(self,scope,status,lease=None,observation=None,errors=()):
        # Operational diagnostics only, never part of the source/DB wire and
        # never consumed as eligibility. No Env, provider output or free text.
        invocation=self.j.common['invocation_id'] if self.j is not None else None
        value=dict(invocation_id=invocation,scope=scope,status=status,
            errors=list(errors),original_failure=getattr(self,'failure_code',None))
        if lease is not None:
            value.update(name=lease.name,image=lease.image,pin=lease.pin,observed_id=lease.observed_id,
                removed=lease.removed,launch_uncertain=lease.launch_uncertain,wait_errors=lease.wait_errors)
        if observation is not None:
            value['terminal']=dict(id=observation['Id'],created=observation['Created'],running=observation['State']['Running'])
        if self.child is not None:value['candidate_child_pid']=self.child.pid
        if self.owned_child is not None:
            value['child']=self.owned_child.identity
            value['child_custody']=self.owned_child.diagnostic()
        if getattr(self,'command_custodies',None):value['command_children']=[owned.diagnostic() for owned in self.command_custodies]
        publish(Path(self.b['evidence_root'])/'.control',(invocation or 'startup')+'-cleanup-'+str(uuid.uuid4())+'.json',canonical(value),True)
    def cleanup_container(self,lease,scope):
        try:
            lease.cleanup(lambda value:self.cleanup_note(scope,'OBSERVED_TERMINAL',lease,value))
            self.cleanup_note(scope,'CLEAN',lease)
        except Exception as error:
            self.cleanup_errors.append(scope+':'+error_tag(error))
            self.cleanup_note(scope,'UNKNOWN',lease,errors=(error_tag(error),))
            raise Rejected('UNKNOWN') from error
    def research_container(self):
        phase,name,argv,client,launched=self.current
        expected={'precheck':['--eod-raw-check-session-dates',self.j.session],
            'capture':['--eod-raw-session-dates',self.j.session],'action':['--r2-action-supply']}[phase]
        def verify(value):
            config=value.get('Config',{});labels=config.get('Labels') or {}
            require(config.get('Cmd')==expected and labels.get('com.docker.compose.service')=='research-worker'
                and labels.get('org.opencontainers.image.revision')==self.b['cohort']['verifier_revision']
                and config.get('User') in (str(self.b['raw_owner_uid']),f"{self.b['raw_owner_uid']}:{self.b['raw_owner_uid']}"),'ORIGIN_UNPROVEN')
        return OwnedContainer(self.docker,name,self.b['cohort']['research_image_id'],launched,verify)
    def helper(self,op,extra=()):
        self.guard()
        self.authority.helper_projection()
        network=self.docker.object('network',self.b['network'])
        require(network.get('Internal') is True and network.get('Labels',{}).get('com.docker.compose.project')==self.b['project'] and network.get('Labels',{}).get('com.docker.compose.network')=='owner-beta-db')
        image=self.docker.object('image',self.b['cohort']['helper_image_id'])
        require(image.get('Id')==self.b['cohort']['helper_image_id'] and image.get('Config',{}).get('Labels',{}).get('org.opencontainers.image.revision')==self.b['cohort']['verifier_revision'])
        name='lagrange-paper-'+self.j.common['invocation_id']+'-'+op+'-'+str(len(self.j.events))
        self.absent(name)
        self.guard()
        projection=self.authority.helper_projection()
        selected=decode(self.pin)
        require(selected==self.b and _helper_projection_matches(projection,selected,self.pin),'CONFIG_CHANGED')
        runtime=self.runtime;worker_root=projection.worker_historical_root;extra=tuple(extra)
        args=runtime.paper_evidence_helper_args(selected,op,self.j.slot,extra,name,
            worker_historical_root=worker_root)
        client=process(os.getpid())
        lease=OwnedContainer(self.docker,name,selected['cohort']['helper_image_id'],time.time_ns(),
            lambda value,runtime=runtime,selected=selected,op=op,slot=self.j.slot,extra=extra,name=name,worker_root=worker_root:
                runtime.verify_paper_evidence_helper(value,selected,op,slot,extra,name,
                    worker_historical_root=worker_root))
        try:
            lease.launch_uncertain=True
            raw=self.docker.call(args,timeout=180)
            lease.launch_uncertain=False
            value=lease.observe(time.monotonic()+15);require(value is not None,'DB_UNAVAILABLE')
            observation=docker_observation(value,op,digest(canonical(args)),client,self.b['cohort']['helper_image_id'])
            self.j.append('CONTAINER',observation)
            lease.remove(time.monotonic()+15)
            require(observation['exit_code']==0 and observation['oom_killed'] is False,'DB_UNAVAILABLE')
            result=decode(raw.encode().strip(),SESSION_LIMIT,False)
            validate(result,SCHEMA['$defs']['database'])
            self.guard(); return result
        except BaseException as error:
            self.failure_code=error_tag(error);self.invalidate(error)
            with self.cleanup_scope():self.cleanup_container(lease,'helper-'+op)
            raise
    def respond(self,channel,peer,request):
        require(descendant(peer,self.child_identity),'ORIGIN_UNPROVEN')
        op=request.get('op'); reply={}
        if op=='begin':
            require(set(request)=={'op','phase','argv','client'} and self.current is None and self.phase_index<3)
            phase=request['phase']; require(phase==['precheck','capture','action'][self.phase_index])
            require(request['client']==process(peer)); validate_compose(request['argv'],self.b,self.j.session,phase)
            name='lagrange-paper-'+self.j.common['invocation_id']+'-'+phase; self.absent(name)
            self.current=(phase,name,request['argv'],request['client'],time.time_ns())
            self.current_container=self.research_container();self.current_container.launch_uncertain=True;reply['name']=name
        elif op=='end':
            require(set(request)=={'op','phase','client','exit_code','output'} and self.current is not None)
            phase,name,argv,client,launched=self.current
            require(request['phase']==phase and request['client']==client and client==process(peer) and type(request['exit_code']) is int)
            self.current_container.launch_uncertain=False
            obj=self.current_container.observe(time.monotonic()+15);require(obj is not None,'ORIGIN_UNPROVEN')
            actual=list(argv); actual[actual.index('--rm'):actual.index('--rm')+1]=['--name',name]
            observation=docker_observation(obj,phase,digest(canonical(actual)),client,self.b['cohort']['research_image_id'])
            self.j.append('CONTAINER',observation)
            self.current_container.remove(time.monotonic()+15)
            require(int(dt.datetime.fromisoformat(observation['created_at'].replace('Z','+00:00')).timestamp()*10**9)>=launched-10**9 and int(dt.datetime.fromisoformat(observation['finished_at'].replace('Z','+00:00')).timestamp()*10**9)<=time.time_ns()+10**9,'ORIGIN_UNPROVEN')
            require(request['exit_code']==0 and observation['exit_code']==0 and observation['oom_killed'] is False)
            if phase=='precheck':
                lines=request['output'].splitlines(); require(len(lines)==2)
                checked=decode(lines[0].encode(),8192,False); terminal=decode(lines[1].encode(),8192,False)
                require(checked.get('source_present') is False and checked.get('target_date')==self.j.session
                        and checked.get('event')=='eod_source_checked' and terminal.get('outcome')=='raw_checked')
            if phase=='action': self.action=action_result(self.b,request['output'],self.j.session)
            self.current=None;self.current_container=None;self.phase_index+=1
        elif op=='raw':
            require(set(request)=={'op','data'} and self.raw is None and self.phase_index==2 and self.current is None)
            validate(request['data'],SCHEMA['$defs']['raw']); self.raw=request['data']
            require(self.raw['target_date']==self.j.session and self.raw['reused_existing_source'] is False and self.raw['calendar_claim_preexisting'] is False)
            self.j.append('RAW_SOURCE',self.raw)
        else: raise Rejected('ORIGIN_UNPROVEN')
        channel.send(canonical(dict(reply,ok=True,parent=process(os.getpid()))))
    def abort_owned(self):
        if self.current is None:return
        require(self.current_container is not None,'ORIGIN_UNPROVEN')
        with self.cleanup_scope():self.cleanup_container(self.current_container,'source-'+self.current[0])
        self.current=None;self.current_container=None
    def source_lifecycle_barrier(self):
        # The installed scheduler always uses the strict installed-process path.
        # A separately owned private fixture subclass can orchestrate original
        # constructors; no CLI/environment/checkpoint selects that subclass.
        return source_lifecycle_barrier(self.path,self.authority)
    def armed(self):
        pass
    def source_finished(self):
        pass
    def run_session(self,session):
        self.child=None;self.owned_child=None;self.failure_code=None;self.cleanup_errors=[]
        self.j=Journal(self.b,session); self.j.append('STARTED')
        due=due_ns(session); require(due-time.time_ns()>=60*10**9,'MISSED_UNARMED')
        # An intent is a durable one-use claim too. A crash before RESERVED may
        # lose a day, but a restarted parent can never inherit the old arm.
        self.source_lifecycle_barrier()
        require(due-time.time_ns()>=60*10**9,'MISSED_UNARMED')
        self.j.claim(); self.j.append('ARM_INTENT')
        timer=None;self.cancel=None;parent=None;child=None
        handles_closed=False
        def close_handles():
            nonlocal handles_closed
            if handles_closed:return []
            handles_closed=True;errors=[]
            for name,resource in (('timer',timer),('cancel',self.cancel),('parent',parent),('channel',child),('pidfd',self.owned_child)):
                if resource is not None:
                    try:resource.close()
                    except Exception as error:errors.append(name+':'+type(error).__name__)
            return errors
        try:
            timer=Timer(due);self.cancel=Timer(due+2*86400*10**9)
            self.j.append('ARMED'); require(due-time.time_ns()>=60*10**9,'MISSED_UNARMED')
            self.armed()
            while not select.select([timer.fd],[],[],0.25)[0]: self.guard()
            timer.consume(); self.guard(); require(due<=time.time_ns()<=due+60*10**9,'MISSED_LATE')
            reserved=self.j.append('RESERVED')
            db=self.helper('reserve'); require(db['slot']['root_event_sha256']==reserved and db['slot']['reservation_id']==self.j.common['reservation_id'],'DB_UNAVAILABLE')
            self.j.append('DB_RESERVED',{'readback_sha256':digest(canonical(db))})
            self.guard(); require(due<=time.time_ns()<=due+60*10**9,'MISSED_LATE')
            argv=['/bin/bash',self.b['release_root']+'/scripts/ops/kis-daily-production.sh','--raw-source','--action-supply','--execute']
            self.j.append('LAUNCH_INTENT',{'argv_sha256':digest(canonical(argv))})
            parent,child=socket.socketpair(socket.AF_UNIX,socket.SOCK_SEQPACKET)
            parent.setsockopt(socket.SOL_SOCKET,socket.SO_PASSCRED,1)
            parent.settimeout(1)  # a peer that stops reading cannot freeze guard/cleanup
            env=source_environment(self.b,child.fileno())
            self.child=subprocess.Popen(argv,pass_fds=(child.fileno(),),env=env,cwd=self.b['release_root'],stdin=subprocess.DEVNULL,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,start_new_session=True,close_fds=True)
            self.owned_child=OwnedChild(self.child,group=True,observe=False)
            self.owned_child.acquire()
            self.child_identity=self.owned_child.identity
            closing=child;child=None;closing.close()
            self.j.append('RUNNING',{'child':self.child_identity})
            deadline=time.monotonic()+self.b['max_source_seconds']
            while not self.owned_child.settled():
                self.guard(); require(time.monotonic()<deadline,'UNKNOWN')
                if select.select([parent],[],[],0.25)[0]:
                    data,anc,flags,_=parent.recvmsg(SOURCE_LIMIT,128)
                    require(data and not flags,'ORIGIN_UNPROVEN')
                    cred=[struct.unpack('3i',v) for l,k,v in anc if l==socket.SOL_SOCKET and k==socket.SCM_CREDENTIALS]
                    require(len(cred)==1 and cred[0][1]==0,'ORIGIN_UNPROVEN')
                    self.respond(parent,cred[0][0],decode(data))
            require(self.owned_child.wait(remaining(deadline,self.b['max_source_seconds']))==0
                and self.current is None and self.phase_index==3 and self.raw is not None and self.action is not None)
            closing=parent;parent=None;closing.close()
            self.guard(); originals=raw_originals(self.b,self.raw);self.guard()
            self.j.append('SOURCE_FINISHED',dict(wrapper_exit=0,original_set_sha256=original_hash(originals),originals=originals,action=self.action))
            self.source_finished()
            # Existing Daily producer/admitter/worker run independently. Only
            # identity linkage is retried; never repeat source/provider work.
            end=due+int(7.5*3600*10**9)
            for _ in range(12):
                require(time.time_ns()<end,'LINK_PENDING')
                self.guard(); db=self.helper('read')
                if db.get('candidate_admission_sha256'):
                    linked=self.helper('link-admission',(db['candidate_admission_sha256'],))
                    a=linked['admission']; self.j.append('ADMISSION_LINKED',dict(admission_sha256=a['admission_sha256'],source_event_sha256=a['source_event_sha256'],original_set_sha256=a['original_set_sha256'],readback_sha256=digest(canonical(linked))))
                    break
                for _ in range(15):
                    self.guard(); time.sleep(1)
            else: raise Rejected('LINK_PENDING')
            for _ in range(12):
                require(time.time_ns()<end,'LINK_PENDING')
                self.guard(); db=self.helper('read')
                if db.get('candidate_run_id') and db.get('terminal')=='SUCCEEDED':
                    linked=self.helper('link-run',(db['candidate_run_id'],))
                    r=linked['run']; self.j.append('RUN_LINKED',dict(run_id=r['run_id'],job_id=r['job_id'],attempt_id=r['attempt_id'],readback_sha256=digest(canonical(linked))))
                    break
                for _ in range(15):
                    self.guard(); time.sleep(1)
            else: raise Rejected('LINK_PENDING')
            self.guard(); verify_chain(self.j.events,'RUN_LINKED')
            # Final eligibility is published only after owned descriptors close.
            # A close error cannot leave a briefly eligible LINKED prefix.
            errors=close_handles();self.cleanup_errors.extend(errors)
            require(not errors,'UNKNOWN')
            self.j.append('LINKED');verify_chain(self.j.events,'LINKED')
        except BaseException as exc:
            self.invalidate(exc)
            codes=SCHEMA['$defs']['failure']['properties']['code']['enum']
            code=self.failure_code if self.failure_code in codes else str(exc) if isinstance(exc,Rejected) and str(exc) in codes else 'UNKNOWN'
            self.failure_code=code;errors=list(self.cleanup_errors)
            # Preserve failure and the durable claim BEFORE cleanup. No later
            # event can turn this failed invocation into a successful chain.
            try:self.j.append('FAILED',{'code':code})
            except Exception as error:errors.append('journal:'+type(error).__name__)
            if self.child is not None:
                try:
                    require(self.owned_child is not None,'UNKNOWN')
                    self.owned_child.cleanup()
                except Exception as error:errors.append('child:'+error_tag(error))
            # Quiesce the wrapper first: it must not launch a named object after
            # an absence observation. If child cleanup failed, absence is UNKNOWN.
            try:
                self.abort_owned()
            except Exception as error:errors.append('container:'+error_tag(error))
            try:self.cleanup_note('session','UNKNOWN' if errors else 'CLEAN',errors=errors)
            except Exception as error:errors.append('receipt:'+type(error).__name__)
            failure=child_failure(Rejected(code),self.owned_child,errors)
            failure.original_failure=error_tag(exc)
            if isinstance(exc,OSError):failure.original_errno=exc.errno
            raise failure from exc
        finally:
            # All descriptors close even if another descriptor's close fails.
            errors=close_handles()
            if errors:
                self.invalidate(Rejected('UNKNOWN'))
                if self.j.events[-1]['kind']!='FAILED':
                    self.failure_code='UNKNOWN'
                    try:self.j.append('FAILED',{'code':'UNKNOWN'})
                    except Exception as error:errors.append('journal:'+type(error).__name__)
                try:self.cleanup_note('descriptors','UNKNOWN',errors=errors)
                finally:raise Rejected('UNKNOWN')
    def run(self):
        root=Path(self.b['evidence_root'])
        with opened(root/'.control/lock',write=True,exact=(0,0,0o600)) as lock:
            fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
            self.authority=CurrentAuthority(self,lock)
            old_call=self.docker.call;self.docker.call=self.docker_call
            handlers={}
            def interrupted(sig,frame):raise Rejected('INTERRUPTED')
            try:
                for sig in (signal.SIGTERM,signal.SIGINT):handlers[sig]=signal.signal(sig,interrupted)
                self.authority.start()
                while True:
                    self.authority.poll(check_assets=True)
                    today=dt.datetime.now(KST).date()
                    for offset in range(370):
                        day=(today+dt.timedelta(days=offset)).isoformat()
                        if due_ns(day)-time.time_ns()<60*10**9: continue
                        command=['/usr/bin/python3',self.b['release_root']+'/scripts/ops/xkrx-calendar-bootstrap.py','--emit-sessions','--start',day,'--end',day,'--output-dir',self.b['calendar_dir']]
                        result=self.command(command,30)
                        require(result.returncode==0 and len(result.stdout)<32,'CONFIG_CHANGED')
                        meta=decode(result.stderr.strip(),8192,False)
                        require(meta.get('artifact_sha256')==self.b['cohort']['calendar_sha256'],'CONFIG_CHANGED')
                        if result.stdout.decode().splitlines()==[day]: break
                    else: raise Rejected('CONFIG_CHANGED')
                    self.run_session(day)
                    self.raw=None; self.action=None; self.phase_index=0; self.current=None
                    self.previous_clock=sample(); self.initial_clock=self.previous_clock
            except BaseException as error:
                self.invalidate(error);raise
            finally:
                try:self.authority.revoke('STOPPED')
                finally:
                    self.docker.call=old_call
                    for sig,handler in handlers.items():signal.signal(sig,handler)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    sub=parser.add_subparsers(dest='command',required=True)
    run=sub.add_parser('run'); run.add_argument('--binding',required=True)
    relay=sub.add_parser('compose'); relay.add_argument('phase',choices=['precheck','capture','action']); relay.add_argument('argv',nargs=argparse.REMAINDER)
    args=parser.parse_args()
    try:
        if args.command=='compose': return relay_compose(args.phase,args.argv[1:] if args.argv[:1]==['--'] else args.argv)
        Scheduler(args.binding).run(); return 0
    except (Rejected,OSError,ValueError,KeyError,subprocess.SubprocessError) as error:
        custody=getattr(error,'child_custody',None)
        if custody is not None and not custody.reaped:
            # A relay has no binding/Journal of its own. Preserve a bounded,
            # sanitized handoff for its caller as well as in-memory custody.
            note=dict(child=custody.diagnostic(),original_failure=getattr(error,'original_failure',error_tag(error)),
                cleanup_errors=getattr(error,'cleanup_errors',[]))
            print('PAPER_SOURCE_CHILD_UNKNOWN '+canonical(note).decode(),file=sys.stderr)
        print('PAPER_SOURCE_WITNESS_REJECTED',file=sys.stderr); return 1



def render_service(b, binding_path, state_dir, lock_file):
    for value in (binding_path,state_dir,lock_file,*[b[k] for k in ('release_root','production_env_file','calendar_dir','evidence_root')]):
        require(isinstance(value,str) and re.fullmatch(r'/[A-Za-z0-9/._-]+',value) is not None and '..' not in value.split('/'))
    install_root=str(Path(b['release_root']).parent.parent)
    require(b['release_root']==install_root+'/releases/'+b['cohort']['verifier_revision'])
    return f'''[Unit]
Description=Lagrange protected future KIS source witness
After=docker.service network-online.target
Requires=docker.service
Wants=network-online.target
[Service]
Type=simple
User=root
Group=root
WorkingDirectory={b['release_root']}
Environment=PYTHONDONTWRITEBYTECODE=1
Environment=KIS_DAILY_STATE_DIR={state_dir}
Environment=KIS_DAILY_LOCK_FILE={lock_file}
ExecStart=/usr/bin/python3 {b['release_root']}/scripts/ops/lib/owner-beta-paper-evidence.py run --binding {binding_path}
Restart=no
KillMode=mixed
TimeoutStopSec=12h
SendSIGKILL=no
UMask=0077
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictAddressFamilies=AF_UNIX
ReadOnlyPaths={b['release_root']} {binding_path} {b['production_env_file']} {b['calendar_dir']}
ReadWritePaths={b['evidence_root']} {state_dir} {lock_file} {install_root}/.lagrange-release.lock -{install_root}/.owner-beta-paper {install_root}/.owner-beta-paper-lifecycle
[Install]
WantedBy=multi-user.target
'''

def install_witness(args, *, _render_service=None, _systemctl=None):
    """Official installer branch. Merely defining this path never activates it."""
    b,_=binding(args.binding)
    require(b['release_root']==args.release_root and b['cohort']['verifier_revision']==args.commit)
    require(b['production_env_file']==args.env_file and b['calendar_dir']==args.calendar_dir)
    require(b['state_dir']==args.state_dir and b['lock_file']==args.lock_file)
    body=(render_service if _render_service is None else _render_service)(b,args.binding,args.state_dir,args.lock_file)
    name='lagrange-kis-witness-'+args.commit[:7]+'.service'; target=Path(args.systemd_dir)/name
    if args.mode in ('dry-run','preflight'):
        print(body,end=''); print('PAPER_WITNESS_INSTALL_PLAN: no runtime operation'); return 0
    if args.mode=='check':
        require(read_file(target,16384,exact=(0,0,0o644))==body.encode()); return 0
    require(args.mode in ('apply','install-disabled') and os.geteuid()==0)
    load_lifecycle().prepare_storage(str(Path(b['release_root']).parent.parent))
    def systemctl(*argv):
        result=subprocess.run(['/usr/bin/systemctl',*argv],env=ENV,stdin=subprocess.DEVNULL,capture_output=True,check=False,timeout=30)
        require(result.returncode==0 and len(result.stdout)<=65536,'ORIGIN_UNPROVEN');return result.stdout.decode().strip()
    if _systemctl is not None: systemctl=_systemctl
    if args.mode=='apply':
        listed=systemctl('list-unit-files','--no-legend','--no-pager','lagrange-kis-daily-*.timer')
        timers=[]
        for line in listed.splitlines():
            unit=line.split()[0];require(re.fullmatch(r'lagrange-kis-daily-[0-9a-f]{7}\.timer',unit) is not None)
            timers.append(unit)
        if timers: systemctl('disable','--now',*timers)
        for timer in timers:
            require(systemctl('show',timer,'--property=ActiveState','--value')=='inactive')
            require(systemctl('show',timer[:-6]+'.service','--property=ActiveState','--value')=='inactive')
        jobs=systemctl('list-jobs','--no-legend','--no-pager')
        require('lagrange-kis-daily-' not in jobs and 'lagrange-kis-witness-' not in jobs,'ORIGIN_UNPROVEN')
    require(not target.exists() and not target.is_symlink(),'ORIGIN_UNPROVEN')
    # Hold the existing collector lock through installation. Never remove it,
    # kill its owner or treat a timer stop as a completed source drain.
    with opened(args.lock_file,write=True,exact=(0,0,0o600)) as lock:
        fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
        root=Path(b['evidence_root'])
        for folder,gid,mode in ((root,10001,0o750),(root/'events',10001,0o750),(root/'.control',0,0o700)):
            with opened(folder.parent,directory=True) as parent:
                if not folder.exists():
                    os.mkdir(folder.name,mode,dir_fd=parent);os.chown(folder.name,0,gid,dir_fd=parent,follow_symlinks=False);os.chmod(folder.name,mode,dir_fd=parent,follow_symlinks=False);os.fsync(parent)
            with opened(folder,directory=True,exact=(0,gid,mode)): pass
        if not (root/'.control/lock').exists(): publish(root/'.control','lock',b'lock\n',True)
        with opened(root/'.control/lock',write=True,exact=(0,0,0o600)) as scheduler_lock:
            fcntl.flock(scheduler_lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
            with opened(root,directory=True,exact=(0,10001,0o750)) as parent:
                if not (root/'current').exists():
                    os.mkdir('current',0o750,dir_fd=parent);os.chown('current',0,10001,dir_fd=parent,follow_symlinks=False);os.chmod('current',0o750,dir_fd=parent,follow_symlinks=False);os.fsync(parent)
            current=CurrentPublication(b,scheduler_lock)
            try:current.emit('INVALID','STOPPED',b)
            finally:current.close()
            with opened(args.systemd_dir,directory=True) as parent:
                fd=os.open(name,os.O_WRONLY|os.O_CREAT|os.O_EXCL|os.O_NOFOLLOW,0o644,dir_fd=parent)
                with os.fdopen(fd,'wb') as out:
                    os.fchmod(out.fileno(),0o644);out.write(body.encode());out.flush();os.fsync(out.fileno())
                os.fsync(parent)
            systemctl('daemon-reload')
            require(systemctl('show',name,'--property=ActiveState','--value')=='inactive')
            if args.mode=='apply': systemctl('enable',name)
            else: require(systemctl('show',name,'--property=UnitFileState','--value')=='disabled')
    print('PAPER_WITNESS_INSTALLED_INACTIVE');return 0

if __name__=='__main__':
    if sys.argv[1:2]==['install']:
        p=argparse.ArgumentParser();p.add_argument('command')
        for key in ('binding','release-root','commit','env-file','calendar-dir','state-dir','lock-file','systemd-dir','mode'):p.add_argument('--'+key,required=True)
        try: raise SystemExit(install_witness(p.parse_args()))
        except (Rejected,OSError,ValueError,subprocess.SubprocessError):
            print('PAPER_WITNESS_INSTALL_REJECTED',file=sys.stderr);raise SystemExit(1) from None
    raise SystemExit(main())
