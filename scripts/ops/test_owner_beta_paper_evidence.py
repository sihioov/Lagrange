#!/usr/bin/env python3
"""Offline source/ABI regressions. No kernel timer, DB, Docker or provider calls."""
import copy
import ast
import errno
import json
import importlib.util
import os
from pathlib import Path
import socket
import struct
import tempfile
import threading
import types
import unittest
from unittest import mock

ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('paper_evidence',ROOT/'scripts/ops/lib/owner-beta-paper-evidence.py')
w=importlib.util.module_from_spec(spec);spec.loader.exec_module(w)
FIXTURES=ROOT/'crates/job-queue/tests/fixtures/owner_beta_paper_source_evidence'

def fixture(): return w.decode((FIXTURES/'valid_chain.json').read_bytes(),w.SESSION_LIMIT)
def rehash(events):
    for index,event in enumerate(events):
        event['sequence']=index
        event['previous_sha256']=w.digest(w.canonical(events[index-1])) if index else None

class WireTests(unittest.TestCase):
    def test_python_rust_shared_valid_chain(self):
        value=fixture(); w.validate(value); self.assertIsNotNone(w.verify_chain(value['events'],'LINKED'))
    def test_shared_semantic_rejections(self):
        cases=json.loads((FIXTURES/'rejections.json').read_bytes())
        self.assertEqual(len(cases),25)
        for case in cases:
            with self.subTest(case=case['name']):
                events=fixture()['events']; cursor=events[case['event']]; parts=case['path'].strip('/').split('/')
                for key in parts[:-1]: cursor=cursor[int(key)] if isinstance(cursor,list) else cursor[key]
                key=parts[-1]; cursor[int(key) if isinstance(cursor,list) else key]=case['value']
                rehash(events)
                with self.assertRaises(w.Rejected): w.verify_chain(events,'LINKED')
    def test_duplicate_unknown_and_noncanonical(self):
        for raw in (b'{"a":1,"a":1}',b'{ "a":1}',b'{"a":NaN}',b'1.5',b'['*17+b'0'+b']'*17):
            with self.subTest(raw=raw),self.assertRaises(w.Rejected): w.decode(raw)
        value=fixture(); value['events'][0]['ambient_success']=True
        with self.assertRaises(w.Rejected): w.validate(value)
    def test_bounds(self):
        with self.assertRaises(w.Rejected):w.decode(b' '*(w.SOURCE_LIMIT+1))
        events=fixture()['events']; events.extend(copy.deepcopy(events)*8)
        with self.assertRaises(w.Rejected):w.verify_chain(events)
    def test_crash_prefix_never_complete(self):
        events=fixture()['events']
        for count in range(1,len(events)):
            with self.subTest(count=count),self.assertRaises(w.Rejected):w.verify_chain(events[:count],'LINKED')
    def test_deleted_original_event_rejected_after_rehash(self):
        events=fixture()['events']; del events[9];rehash(events)
        with self.assertRaises(w.Rejected):w.verify_chain(events,'LINKED')
    def test_claim_survives_cohort_switch(self):
        c=fixture()['events'][0]['cohort']; changed=dict(c,cohort_sha256='sha256:'+'b'*64)
        self.assertEqual(w.dispatch_key(c,'2026-10-29'),w.dispatch_key(changed,'2026-10-29'))
        self.assertNotEqual(w.slot_id(c,'2026-10-29'),w.slot_id(changed,'2026-10-29'))
    def test_original_hash_is_identity_ordered(self):
        orig=fixture()['events'][11]['data']['originals']; self.assertEqual(w.original_hash(orig),fixture()['events'][11]['data']['original_set_sha256'])
        orig[1]['file_name']=orig[0]['file_name']; orig[1]['wire_sha256']='sha256:'+'a'*64
        with self.assertRaises(w.Rejected):w.original_hash(orig)
    def test_env_number_not_private_channel(self):
        with mock.patch.dict(os.environ,{'LAGRANGE_PAPER_WITNESS_FD':'999999'}),self.assertRaises(w.Rejected):w.ipc_request({'op':'raw'})
    def test_unprotected_file_rejected(self):
        with tempfile.TemporaryDirectory() as root:
            leaf=Path(root)/'event';leaf.write_text('{}')
            with self.assertRaises(w.Rejected):w.read_file(leaf,100,exact=(0,10001,0o440))
    def test_wrong_compose_surface_rejected(self):
        binding={'release_root':'/release','production_env_file':'/prod.env'}
        argv=['docker','compose','--env-file','/prod.env','-f','/release/deploy/compose/compose.yml','-f','/release/deploy/compose/compose.intraday.yml','run','--rm','--no-deps','research-worker','--eod-raw-session-dates','2026-10-29']
        w.validate_compose(argv,binding,'2026-10-29','capture')
        for tail in (['--backfill-session-dates','2026-10-29'],['--eod-raw-session-dates','2026-10-28'],['--eod-raw-session-dates','2026-10-29,2026-10-30']):
            with self.assertRaises(w.Rejected):w.validate_compose(argv[:-2]+tail,binding,'2026-10-29','capture')
    def test_helper_identity_roles_and_mounts(self):
        runtime=w.load_runtime(); first=fixture()['events'][0]
        b={'cohort':first['cohort'],'evidence_root':'/evidence','daily_config_file':'/release/.owner-beta-daily/config.json','release_manifest_file':'/release/.lagrange-release-manifest','calendar_dir':'/calendar','calendar_file':'/calendar/calendar.json','production_env_file':'/control/prod.env','release_root':'/release','data_root':'/data','secret_root':'/secrets','database':'lagrange','network':'owner-beta-db','daily_config_sha256':'sha256:'+'2'*64}
        for op,uid in (('reserve','10003:10003'),('link-admission','10003:10003'),('link-run','10001:10001'),('read','10001:10001')):
            args=runtime.paper_evidence_helper_args(b,op,first['slot_id'],('identity',) if op.startswith('link-') else (),'lagrange-paper-fixture',worker_historical_root='/synthetic/artifacts/historical-price-beta-root')
            self.assertIn(uid,args); self.assertNotIn('--rm',args); self.assertNotIn('docker.sock',' '.join(args)); self.assertNotIn('KIS_APP',' '.join(args))
            for i,arg in enumerate(args):
                if arg=='--mount':self.assertTrue(args[i+1].endswith(',readonly'))
    def test_installer_renders_future_service_without_catchup(self):
        b={'release_root':'/releases/'+'a'*40,'production_env_file':'/prod/env','calendar_dir':'/calendar','evidence_root':'/data/evidence'}
        body=w.render_service(b,'/control/binding.json','/state','/state/lock')
        self.assertIn('Type=simple',body);self.assertIn('run --binding /control/binding.json',body)
        self.assertNotIn('Persistent=true',body);self.assertNotIn('OnCalendar=',body)
        self.assertIn('Restart=no',body);self.assertIn('SendSIGKILL=no',body)
    def test_installed_custom_state_lock_survive_child_environment(self):
        b={'production_env_file':'/prod/env','calendar_dir':'/calendar','state_dir':'/custom/state',
           'lock_file':'/custom/locks/daily.lock','cohort':{'verifier_revision':'a'*40}}
        with mock.patch.dict(os.environ,{'KIS_DAILY_STATE_DIR':'/ambient/state','KIS_DAILY_LOCK_FILE':'/ambient/lock','KIS_APP_KEY':'sentinel'}):
            child=w.source_environment(b,9)
        self.assertEqual(child['KIS_DAILY_STATE_DIR'],'/custom/state')
        self.assertEqual(child['KIS_DAILY_LOCK_FILE'],'/custom/locks/daily.lock')
        self.assertNotIn('KIS_APP_KEY',child)
        self.assertEqual(child['LAGRANGE_PAPER_WITNESS_FD'],'9')
    def test_no_replace_claim_and_fs_security_are_not_fixture_bypasses(self):
        # Verify rejected filesystem metadata without requiring root/chown.
        fake=mock.Mock(st_uid=0,st_gid=10001,st_mode=0o100440,st_nlink=2,st_size=2)
        with mock.patch.object(w.os,'open',return_value=4),mock.patch.object(w.os,'close'),mock.patch.object(w.os,'fstat',return_value=fake),mock.patch.object(w.os,'listxattr',return_value=[]):
            with self.assertRaises(w.Rejected):w.read_file('/fixture',100,exact=(0,10001,0o440))
        with mock.patch.object(w.os,'listxattr',return_value=['system.posix_acl_access']),self.assertRaises(w.Rejected):w.no_acl(4)
    def test_foreign_container_is_never_signalled_or_removed(self):
        scheduler=object.__new__(w.Scheduler)
        scheduler.current=('capture','lagrange-paper-owned',[],{'pid':10,'start_ticks':20},1000)
        scheduler.b={'cohort':{'research_image_id':'sha256:'+'a'*64,'verifier_revision':'b'*40}}
        scheduler.docker=mock.Mock();scheduler.docker.object.return_value={'Name':'/lagrange-paper-foreign','Image':'sha256:'+'a'*64}
        scheduler.docker.call.return_value='a'*64
        scheduler.current_container=w.OwnedContainer(scheduler.docker,'lagrange-paper-owned','sha256:'+'a'*64,1000,lambda _:None)
        scheduler.cleanup_note=mock.Mock();scheduler.cleanup_errors=[];scheduler.in_cleanup=False
        with self.assertRaises(w.Rejected):scheduler.abort_owned()
        self.assertTrue(all(c.args[0][0]=='ps' for c in scheduler.docker.call.call_args_list))
    def test_docker_exit_oom_or_image_cannot_be_success(self):
        for state in ({'Status':'running','Running':True},{'Status':'exited','Running':False,'OOMKilled':True,'ExitCode':0}):
            with self.assertRaises(w.Rejected):w.docker_observation({'Image':'sha256:'+'a'*64,'State':state},'capture','sha256:'+'b'*64,{'pid':1,'start_ticks':1},'sha256:'+'a'*64)


# Test-only I/O rig. It runs production Scheduler/Journal/helper/respond/relay,
# not a second state machine. No patch or synthetic entry exists in production.
# This is never root-origin evidence, even when its canonical chain validates.
import contextlib
import datetime as dt
import io
import sys
import uuid

class SyntheticSource:
    def __init__(self, root, fault=None, event_hook=None, session=None, cohort=None, real_time=False):
        self.root=Path(root);self.fault=fault;self.event_hook=event_hook
        self.session=session or '2026-10-29';self.now=w.due_ns(self.session)-120*10**9;self.real_time=real_time
        self.launches=0;self.provider_phases=[];self.helper_ops=[];self.objects={};self.next_id=1
        self.real_open=os.open;self.real_chmod=os.chmod
        c=copy.deepcopy(cohort or fixture()['events'][0]['cohort'])
        self.b=dict(cohort=c,evidence_root=str(self.root/'evidence'),data_root=str(self.root/'data'),
            release_root=str(self.root/'release'),production_env_file=str(self.root/'prod.env'),calendar_dir=str(self.root/'calendar'),
            calendar_file=str(self.root/'calendar/calendar.json'),release_manifest_file=str(self.root/'release/.lagrange-release-manifest'),
            raw_owner_uid=10001,daily_config_file=str(self.root/'release/.owner-beta-daily/config.json'),daily_config_sha256='sha256:'+'2'*64,
            secret_root='/synthetic-secrets',database='synthetic',project='synthetic',network='synthetic-db',
            state_dir=str(self.root/'state'),lock_file=str(self.root/'state/lock'),max_source_seconds=60)
        for name in ('events','.control'):(self.root/'evidence'/name).mkdir(parents=True,exist_ok=True)
        for key in ('production_env_file','calendar_file','release_manifest_file'):
            Path(self.b[key]).parent.mkdir(parents=True,exist_ok=True);Path(self.b[key]).write_bytes(b'SYNTHETIC-NO-PROVIDER')
        self.b['production_env_sha256']=w.digest(Path(self.b['production_env_file']).read_bytes())
        for key in ('calendar','release_manifest'):
            if cohort is None:c[key+'_sha256']=w.digest(Path(self.b[key+'_file']).read_bytes())
        self.path=self.root/'binding';self.pin=w.canonical(self.b);self.path.write_bytes(self.pin)
        self.raw=dict(batch_id=str(uuid.uuid4()),calendar_batch_id=str(uuid.uuid4()),target_date=self.session,
                      reused_existing_source=False,calendar_claim_preexisting=False)
        self.db=None;self.runtime=w.load_runtime()
        self.raw_files()
        (self.root/'context.json').write_bytes(w.canonical(dict(b=self.b,raw=self.raw,session=self.session,real_time=self.real_time)))
    def timestamp(self):return dt.datetime.fromtimestamp((w.time.time_ns() if self.real_time else self.now)/10**9,dt.timezone.utc).isoformat().replace('+00:00','Z')
    @classmethod
    def restore(cls,root,event_hook=None):
        rig=object.__new__(cls);rig.root=Path(root);saved=w.decode((rig.root/'context.json').read_bytes())
        rig.b=saved['b'];rig.raw=saved['raw'];rig.session=saved['session'];rig.real_time=saved['real_time']
        rig.path=rig.root/'binding';rig.pin=rig.path.read_bytes();rig.now=w.due_ns(rig.session)-120*10**9
        rig.fault=None;rig.event_hook=event_hook;rig.launches=0;rig.provider_phases=[];rig.helper_ops=[];rig.objects={};rig.next_id=1
        rig.real_open=os.open;rig.real_chmod=os.chmod;rig.db=None;rig.runtime=w.load_runtime();return rig
    def clocks(self):return dict(realtime_ns=self.now+getattr(self,'wall_offset',0),boottime_ns=self.now-10**18,monotonic_ns=self.now-10**18)
    @contextlib.contextmanager
    def opened(self,path,*,directory=False,write=False,**_):
        fd=self.real_open(path,os.O_RDONLY|os.O_DIRECTORY if directory else os.O_RDWR if write else os.O_RDONLY)
        try:yield fd
        finally:os.close(fd)
    def raw_files(self):
        # Real temporary bytes/manifest/batch files; no provider response body.
        stamp=self.timestamp() if self.real_time else dt.datetime.fromtimestamp(w.due_ns(self.session)/10**9,dt.timezone.utc).isoformat().replace('+00:00','Z')
        for provider,bid,kinds in [('kis',self.raw['batch_id'],['bars']*11+['reference']*11),('kis-calendar',self.raw['calendar_batch_id'],['calendar'])]:
            folder=self.root/'data/raw'/f'provider={provider}'/'market=kr'/f'date={self.session}'/f'batch={bid}'
            folder.mkdir(parents=True,exist_ok=True)
            files=[]
            for i,kind in enumerate(kinds):
                raw=w.canonical({'SYNTHETIC':True,'kind':kind,'ordinal':i});name=f'{kind}-{i}.json';(folder/name).write_bytes(raw)
                files.append(dict(kind=kind,file_name=name,content_hash=w.digest(raw),size_bytes=len(raw)))
            meta=dict(provider=provider,market='kr',batch_id=bid,date=self.session,retrieved_at=stamp,files=files)
            (folder/'batch.json').write_bytes(w.canonical(meta))
            manifest=self.root/'data/raw/manifests'/f'provider={provider}'/'market=kr';manifest.mkdir(parents=True,exist_ok=True)
            (manifest/'manifest.jsonl').write_bytes(w.canonical(meta)+b'\n');(manifest/'commit.lock').touch()
    def observation(self,name,image,config,host,mounts=None):
        identity=f'{self.next_id:064x}';self.next_id+=1
        obj=dict(Id=identity,Name='/'+name,Image=image,Created=self.timestamp(),Config=config,HostConfig=host,
            State=dict(Status='exited',Running=False,Error='',OOMKilled=False,ExitCode=0,StartedAt=self.timestamp(),FinishedAt=self.timestamp()),Mounts=mounts or [])
        self.objects[name]=obj;return obj
    def object(self,kind,name,**_):
        if kind=='network':return dict(Internal=True,Labels={'com.docker.compose.project':self.b['project'],'com.docker.compose.network':'owner-beta-db'})
        if kind=='image':return dict(Id=name,Config={'Labels':{'org.opencontainers.image.revision':self.b['cohort']['verifier_revision']}})
        return copy.deepcopy(self.objects.get(name) or next(v for v in self.objects.values() if v['Id']==name))
    def call(self,args,**_):
        if args[0]=='ps':
            name=next(v[len('name=^/'):-1] for v in args if v.startswith('name=^/'))
            return self.objects.get(name,{}).get('Id','')
        if args[0]=='rm':
            key=next(k for k,v in self.objects.items() if v['Id']==args[1]);del self.objects[key];return ''
        if args[0]=='run':
            op=args[args.index('paper-evidence')+1];self.helper_ops.append(op)
            name=args[args.index('--name')+1];env=[args[i+1] for i,v in enumerate(args) if v=='--env'];mounts=[]
            for i,v in enumerate(args):
                if v=='--mount':
                    parts=dict(x.split('=',1) for x in args[i+1].split(',') if '=' in x)
                    mounts.append(dict(Type='bind',RW=False,Destination=parts['dst'],Source=parts['src']))
            host=dict(ReadonlyRootfs=True,Init=True,Privileged=False,AutoRemove=False,CapDrop=['ALL'],CapAdd=None,
                SecurityOpt=['no-new-privileges:true'],GroupAdd=['10001'] if op in ('reserve','link-admission') else [],
                NetworkMode=self.b['network'],Memory=1024**3,NanoCpus=10**9,PidsLimit=64,LogConfig={'Type':'none'},RestartPolicy={'Name':'no'})
            config=dict(User=args[args.index('--user')+1],Entrypoint=[self.runtime.BINARY],Cmd=args[args.index('paper-evidence'):],Env=env)
            self.observation(name,self.b['cohort']['helper_image_id'],config,host,mounts)
            return w.canonical(self.database(op)).decode()
        raise AssertionError('unexpected synthetic Docker operation')
    def database(self,op):
        events=self.scheduler.j.events;first=events[0];c=self.b['cohort']
        if op=='reserve':
            self.db={k:None for k in w.SCHEMA['$defs']['database']['required']}
            self.db.update(schema='owner-beta-paper-db-v1',slot=dict(slot_id=first['slot_id'],reservation_id=first['reservation_id'],
                root_event_sha256=w.digest(w.canonical(next(e for e in events if e['kind']=='RESERVED'))),
                owner_user_id=c['owner_user_id'],policy_sha256=c['policy_sha256'],activation_epoch=c['activation_epoch'],
                session_date=self.session,cohort_sha256=c['cohort_sha256'],cohort=c,reserved_at=self.timestamp()))
        source=next((e for e in events if e['kind']=='SOURCE_FINISHED'),None)
        if source:
            pins=dict(admission_sha256='sha256:'+'a'*64,policy_sha256=c['policy_sha256'],content_sha256='sha256:'+'b'*64,manifest_sha256='sha256:'+'c'*64)
            self.db.update(candidate_admission_sha256=pins['admission_sha256'],candidate_pins=pins,admitted_at=self.timestamp())
            if op=='link-admission':
                self.db['admission']=dict(slot_id=first['slot_id'],source_event_sha256=w.digest(w.canonical(source)),original_set_sha256=source['data']['original_set_sha256'],admission_sha256=pins['admission_sha256'],linked_at=self.timestamp())
            if self.db['admission']:
                run,job,attempt=[str(uuid.UUID(int=i)) for i in (10,11,12)]
                self.db.update(candidate_run_id=run,terminal='SUCCEEDED',job_id=job,attempt_id=attempt,pins=pins,
                    factor_snapshot_sha256='sha256:'+'d'*64,target_snapshot_sha256='sha256:'+'e'*64,finished_at=self.timestamp(),
                    attempts=[dict(id=attempt,attempt_no=1,outcome='SUCCEEDED',started_at=self.timestamp(),finished_at=self.timestamp())])
                if op=='link-run':self.db['run']=dict(slot_id=first['slot_id'],run_id=run,job_id=job,attempt_id=attempt,linked_at=self.timestamp())
        return copy.deepcopy(self.db)
    def ipc(self,message):
        channel=mock.Mock();self.scheduler.respond(channel,200,message)
        return w.decode(channel.send.call_args.args[0])
    def compose(self,phase):
        argv=['docker','compose','--env-file',self.b['production_env_file'],'-f',self.b['release_root']+'/deploy/compose/compose.yml','-f',self.b['release_root']+'/deploy/compose/compose.intraday.yml','run','--rm','--no-deps']
        if phase=='action':argv+=['-e','RESEARCH_R2_ACTION_SUPPLY_CONFIG_HEX']
        argv+=['research-worker']+({'precheck':['--eod-raw-check-session-dates',self.session],'capture':['--eod-raw-session-dates',self.session],'action':['--r2-action-supply']}[phase])
        w.relay_compose(phase,argv)
    def popen(self,argv,**kwargs):
        if argv[0]=='/bin/bash':
            assert kwargs.get('start_new_session') is True
            self.launches+=1;rig=self
            class Wrapper:
                pid=200
                done=False
                def poll(self):
                    if self.done:return 0
                    self.done=True
                    rig.compose('precheck')
                    rig.compose('action' if rig.fault=='phase_order' else 'capture')
                    record=dict(rig.raw)
                    if rig.fault=='missing_calendar':del record['calendar_batch_id']
                    if rig.fault=='reused_calendar':record['calendar_claim_preexisting']=True
                    w.relay_raw(record)
                    if rig.fault=='duplicate_raw':w.relay_raw(record)
                    rig.compose('action')
                    return 0 if rig.real_time else None
                def wait(self,timeout):assert 0<timeout<=43200;self.done=True;return 0
            self.child=Wrapper();return self.child
        phase=self.scheduler.current[0];name=argv[argv.index('--name')+1];self.provider_phases.append(phase)
        if phase=='capture' and self.real_time:self.raw_files()
        cmd=argv[argv.index('research-worker')+1:]
        config=dict(User='10001:10001',Cmd=cmd,Labels={'com.docker.compose.service':'research-worker','org.opencontainers.image.revision':self.b['cohort']['verifier_revision']})
        obj=self.observation(name,self.b['cohort']['research_image_id'],config,dict(AutoRemove=False))
        if self.fault=='container_oom' and phase=='capture':obj['State']['OOMKilled']=True
        if phase=='precheck':
            output=w.canonical(dict(event='eod_source_checked',target_date=self.session,source_present=False))+b'\n'+w.canonical(dict(outcome='raw_checked'))+b'\n'
        elif phase=='capture':output=w.canonical(self.raw)+b'\n'
        else:output=w.canonical(dict(outcome='ALREADY_COVERED',start=None,end=None,batch_id=None,reserved_wire_gets=0,raw_only=True,admitted=False))+b'\n'
        child=mock.Mock();child.stdout=io.BytesIO(output);child.wait.return_value=0;child.poll.return_value=0;return child
    @contextlib.contextmanager
    def patches(self):
        rig=self
        class OwnedChild:
            # Synthetic kernel/process I/O only. Real ownership and timeout
            # algorithms are exercised below with distinct syscall mocks.
            def __init__(self,child,group=False,observe=True):
                self.child=child;self.group=group;self.identity=dict(pid=200,start_ticks=200);self.reaped=False
            def acquire(self):pass
            def diagnostic(self):return dict(pid=200,parent=100,start_ticks=200,group=self.group,reaped=self.reaped,status='CLEAN' if self.reaped else 'UNKNOWN')
            def exited(self):return self.child.poll() is not None
            def settled(self):return self.exited()
            def wait(self,timeout):self.reaped=True;return self.child.wait(timeout=timeout)
            def cleanup(self):
                if self.group:self.child.done=True
                self.wait(5)
            def close(self):pass
        def output(owned,deadline,limit=32768):
            data=owned.child.stdout.read();w.require(len(data)<=limit)
            return data,owned.wait(5)
        class Timer:
            def __init__(self,due):self.due=due;self.fd=self
            def consume(self):rig.now=self.due
            def close(self):pass
        def selected(read,*_):
            timer=read[0]
            if isinstance(timer,Timer) and timer.due<=w.due_ns(rig.session):return (read,[],[])
            return ([],[],[])
        original_append=w.Journal.append
        def append(j,kind,data=None):
            result=original_append(j,kind,data)
            if rig.event_hook:rig.event_hook(kind,j)
            return result
        with contextlib.ExitStack() as stack:
            for obj,name,value in [(w,'opened',self.opened),(w,'binding',lambda _: (self.b,self.pin)),(w,'structural_binding',lambda _: (self.b,self.pin)),
                (w,'sample',self.clocks),(w.time,'time_ns',lambda:self.now),(w.time,'monotonic',lambda:self.now/10**9),
                (w,'Timer',Timer),(w.select,'select',selected),(w.os,'geteuid',lambda:0),
                (w.os,'chown',lambda *a,**k:None),(w.os,'fchown',lambda *a,**k:None),
                (w,'process',lambda pid:dict(pid=pid,start_ticks=100 if pid!=200 else 200)),
                (w,'descendant',lambda pid,parent:pid==200 and parent['pid']==200),
                (w,'OwnedChild',OwnedChild),(w,'read_child_output',output),
                (w.subprocess,'Popen',self.popen),(w.socket,'socketpair',lambda *a:(mock.Mock(),mock.Mock())),
                (w,'ipc_request',self.ipc),(w,'load_runtime',lambda:self.runtime),
                (self.runtime,'Docker',lambda:self),(w.Journal,'append',append)]:
                if self.real_time and ((obj is w and name in ('sample','Timer')) or (obj is w.time and name in ('time_ns','monotonic')) or (obj is w.select and name=='select')):continue
                stack.enter_context(mock.patch.object(obj,name,value))
            # relay identity is a synthetic child; root/child process identity is
            # separately tested by the real-FD opt-in lane, not inferred here.
            stack.enter_context(mock.patch.object(w.os,'getpid',lambda:200))
            # Journal's root parent must differ from the synthetic child.
            old_init=w.Journal.__init__
            def init(j,*a):
                old_init(j,*a);j.common['arm']['parent']=dict(pid=100,start_ticks=100)
            stack.enter_context(mock.patch.object(w.Journal,'__init__',init))
            stack.enter_context(mock.patch.object(w.sys,'stdout',mock.Mock(buffer=io.BytesIO())))
            self.scheduler=w.Scheduler(self.path)
            self.scheduler.authority=mock.Mock()  # current publication kernel/FS is a separate mocked layer
            self.scheduler.authority.helper_projection.side_effect=lambda:helper_projection_fixture(self.b)
            yield
    def run(self):
        with self.patches():self.scheduler.run_session(self.session)
        return dict(schema='owner-beta-paper-source-chain-v1',events=self.scheduler.j.events)

class ProducerTests(unittest.TestCase):
    def test_guard_and_db_readback_stop_before_child_dispatch(self):
        for fault in ('clock_step','reservation_readback'):
            with self.subTest(fault=fault),tempfile.TemporaryDirectory() as folder:
                rig=SyntheticSource(folder)
                if fault=='clock_step':
                    def step(kind,journal):
                        if kind=='DB_RESERVED':rig.wall_offset=2*10**9
                    rig.event_hook=step
                else:
                    database=rig.database
                    def corrupt(op):
                        value=database(op)
                        if op=='reserve':value['slot']['root_event_sha256']='sha256:'+'f'*64
                        return value
                    rig.database=corrupt
                with self.assertRaises(w.Rejected):rig.run()
                self.assertEqual(rig.launches,0);self.assertEqual(rig.provider_phases,[])
                self.assertEqual(rig.scheduler.j.events[-1]['kind'],'FAILED')
                with self.assertRaises((w.Rejected,OSError)):rig.run()
                self.assertEqual(rig.launches,0)
    def test_crash_boundaries_and_changed_cohort_do_not_relaunch(self):
        for boundary in ('ARM_INTENT','ARMED','RESERVED','DB_RESERVED','LAUNCH_INTENT','RUNNING','RAW_SOURCE','SOURCE_FINISHED','ADMISSION_LINKED','RUN_LINKED'):
            with self.subTest(boundary=boundary),tempfile.TemporaryDirectory() as folder:
                def crash(kind,j):
                    if kind==boundary:raise w.Rejected('UNKNOWN')
                rig=SyntheticSource(folder,event_hook=crash)
                with self.assertRaises(w.Rejected):rig.run()
                launches=rig.launches;phases=list(rig.provider_phases)
                rig.event_hook=None;rig.b['cohort']['cohort_sha256']='sha256:'+'9'*64
                with self.assertRaises((w.Rejected,OSError)):rig.run()
                self.assertEqual(rig.launches,launches);self.assertEqual(rig.provider_phases,phases)
                claims=[p for p in (rig.root/'evidence/.control').glob('*.json') if w.re.fullmatch('[0-9a-f]{64}.json',p.name)]
                self.assertEqual(len(claims),1)
                self.assertEqual(claims[0].name,w.dispatch_key(rig.b['cohort'],rig.session)+'.json')
    def test_production_orchestration_emits_consumable_chain(self):
        with tempfile.TemporaryDirectory() as folder:
            rig=SyntheticSource(folder);chain=rig.run();w.validate(chain);w.verify_chain(chain['events'],'LINKED')
            self.assertEqual(rig.launches,1);self.assertEqual(rig.provider_phases,['precheck','capture','action'])
            self.assertEqual(rig.helper_ops,['reserve','read','link-admission','read','link-run'])
            self.assertFalse(rig.objects)
            originals=next(e['data']['originals'] for e in chain['events'] if e['kind']=='SOURCE_FINISHED')
            self.assertEqual(len(originals),23)
            for path,event in zip(sorted(rig.scheduler.j.directory.glob('*.json')),chain['events']):self.assertEqual(path.read_bytes(),w.canonical(event))
            with self.assertRaises((w.Rejected,OSError)):rig.run()
            self.assertEqual(rig.launches,1)
    def test_bad_child_and_metadata_never_redispatch(self):
        for fault in ('missing_calendar','reused_calendar','duplicate_raw','phase_order','container_oom'):
            with self.subTest(fault=fault),tempfile.TemporaryDirectory() as folder:
                rig=SyntheticSource(folder,fault)
                with self.assertRaises(w.Rejected):rig.run()
                self.assertEqual(rig.scheduler.j.events[-1]['kind'],'FAILED')
                self.assertNotIn('LINKED',[e['kind'] for e in rig.scheduler.j.events])
                before=list(rig.provider_phases)
                with self.assertRaises((w.Rejected,OSError)):rig.run()
                self.assertEqual(rig.provider_phases,before);self.assertEqual(rig.launches,1)

class HarnessTests(unittest.TestCase):
    def harness(self):
        spec=importlib.util.spec_from_file_location('paper_local_harness',ROOT/'scripts/ops/owner-beta-paper-evidence-local-integration.py')
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module);return module
    def test_default_has_no_runtime_or_filesystem_side_effects(self):
        h=self.harness();out=io.StringIO()
        with mock.patch.object(sys,'argv',['harness']),mock.patch.object(h,'context',side_effect=AssertionError('context touched')),mock.patch.object(h.subprocess,'run',side_effect=AssertionError('runtime touched')),contextlib.redirect_stdout(out):
            self.assertEqual(h.main(),0)
        result=json.loads(out.getvalue());self.assertEqual(len(result['lanes']),8)
        self.assertEqual(set(result['lanes'].values()),{'NOT RUN'});self.assertEqual(result['overall'],'INCOMPLETE')
        self.assertTrue(result['synthetic']);self.assertFalse(result['production_eligible']);self.assertIsNone(result['artifacts'])
    def test_marker_denies_production_wrong_vm_or_missing_scope(self):
        h=self.harness();vm=str(uuid.uuid4());args=mock.Mock(vm_id=vm,lane='kernel_timerfd')
        for field,value in [('production',True),('provider_network',True),('vm_id',str(uuid.uuid4())),('schema','wrong')]:
            marker=dict(schema='lagrange-disposable-paper-vm-v1',vm_id=vm,production=False,provider_network=False);marker[field]=value
            with mock.patch.object(h.os,'geteuid',return_value=0),mock.patch.object(h.w,'read_file',return_value=w.canonical(marker)),mock.patch.object(h,'command') as command,self.assertRaises(h.w.Rejected):h.context(args)
            command.assert_not_called()
    def test_mutation_flags_required_before_clock_or_suspend(self):
        h=self.harness()
        for lane,target in [('clock_step_cancel','clock_cancel'),('suspend','suspend_lane')]:
            with tempfile.TemporaryDirectory() as folder:
                with mock.patch.object(sys,'argv',['harness','--execute','--vm-id',str(uuid.uuid4()),'--lane',lane]),mock.patch.object(h,'context',return_value=(Path(folder),{})),mock.patch.object(h,target) as action,self.assertRaises(h.w.Rejected):h.main()
                action.assert_not_called();self.assertEqual(json.loads((Path(folder)/'failed.json').read_text())['status'],'FAIL_OR_UNKNOWN')
    def test_suspend_requires_measured_suspend_delta(self):
        h=self.harness();before=dict(realtime_ns=10**12,boottime_ns=10**11,monotonic_ns=10**11)
        for delta,accepted in [(0,False),(3*10**9,True)]:
            after=dict(realtime_ns=before['realtime_ns']+4*10**9,boottime_ns=before['boottime_ns']+4*10**9,monotonic_ns=before['monotonic_ns']+4*10**9-delta)
            timer=mock.Mock()
            with mock.patch.object(h.w,'sample',side_effect=[before,after]),mock.patch.object(h.w,'Timer',return_value=timer),mock.patch.object(h,'command') as command:
                if accepted:self.assertEqual(h.suspend_lane(),'PASS')
                else:
                    with self.assertRaises(h.w.Rejected):h.suspend_lane()
                self.assertEqual(command.call_args.args[0],['/usr/sbin/rtcwake','--mode','mem','--seconds','3']);timer.close.assert_called_once()
    def test_clock_cancel_requires_exact_kernel_errno(self):
        h=self.harness();before=dict(realtime_ns=10**12,boottime_ns=10**11,monotonic_ns=10**11)
        outcomes=[(OSError(code,'synthetic timer error'),code==h.errno.ECANCELED)
                  for code in (h.errno.ECANCELED,h.errno.EAGAIN,h.errno.EBADF,h.errno.EIO)]
        outcomes.append((struct.pack('Q',1),False))
        for outcome,accepted in outcomes:
            with self.subTest(outcome=repr(outcome)):
                timer=mock.Mock(fd=7)
                read=mock.Mock(side_effect=outcome) if isinstance(outcome,OSError) else mock.Mock(return_value=outcome)
                with mock.patch.object(h.w,'sample',return_value=before),mock.patch.object(h.w,'Timer',return_value=timer),mock.patch.object(h.select,'select',return_value=([7],[],[])),mock.patch.object(h.os,'read',read),mock.patch.object(h.time,'clock_settime_ns') as change_clock,mock.patch.object(h.time,'clock_gettime_ns',return_value=before['boottime_ns']+10**9):
                    if accepted:self.assertEqual(h.clock_cancel(),'PASS')
                    else:
                        with self.assertRaises(h.w.Rejected):h.clock_cancel()
                read.assert_called_once_with(7,8);timer.close.assert_called_once()
                self.assertEqual(change_clock.call_count,2)
    def test_clock_cancel_closes_timer_when_restoration_fails(self):
        h=self.harness();before=dict(realtime_ns=10**12,boottime_ns=10**11,monotonic_ns=10**11)
        timer=mock.Mock(fd=7)
        restore_error=OSError(h.errno.EPERM,'synthetic restoration failure')
        with mock.patch.object(h.w,'sample',return_value=before),mock.patch.object(h.w,'Timer',return_value=timer),mock.patch.object(h.select,'select',return_value=([7],[],[])),mock.patch.object(h.os,'read',side_effect=OSError(h.errno.ECANCELED,'synthetic cancellation')),mock.patch.object(h.time,'clock_settime_ns',side_effect=[None,restore_error]) as change_clock,mock.patch.object(h.time,'clock_gettime_ns',return_value=before['boottime_ns']+10**9):
            with self.assertRaises(OSError) as raised:h.clock_cancel()
        self.assertIs(raised.exception,restore_error)
        self.assertEqual(change_clock.call_count,2)
        timer.close.assert_called_once()
    def test_foreign_unit_is_not_an_owned_cleanup_target(self):
        h=self.harness();unit='lagrange-paper-disposable-'+str(uuid.uuid4())+'.service'
        raw=f'Id={unit}\nFragmentPath=/etc/systemd/system/{unit}\nExecStart=/bin/other\nMainPID=42\n'.encode()
        with mock.patch.object(h,'command',return_value=raw) as command,self.assertRaises(h.w.Rejected):h.unit_show(unit,Path('/run/case'))
        self.assertEqual(command.call_count,1);self.assertEqual(command.call_args.args[0][1],'show')
    def test_pg_config_must_match_out_of_band_pin(self):
        h=self.harness()
        with mock.patch.object(h.w,'read_file',return_value=b'{}'),mock.patch.object(h,'command') as command,self.assertRaises(h.w.Rejected):h.pg_environment(mock.Mock(pg_config='/fixture',vm_id=str(uuid.uuid4())),{'postgres_config_sha256':'sha256:'+'0'*64},Path('/unused'))
        command.assert_not_called()

# All operations below are syscall/daemon mocks. No resource-altering lane runs.
class LifetimeDocker:
    def __init__(self,image,fault=None):
        self.image=image;self.fault=fault;self.value=None;self.calls=[];self.creations=0
    def object(self,kind,identity,**kwargs):
        if kind=='image':return {'Id':self.image}
        assert self.value is not None and identity==self.value['Id']
        return copy.deepcopy(self.value)
    def call(self,args,timeout=5):
        assert 0<timeout<=20
        self.calls.append((list(args),timeout))
        if args[0]=='ps':return self.value['Id'] if self.value else ''
        if args[0]=='run':
            self.creations+=1;name=args[args.index('--name')+1];label=args[args.index('--label')+1].split('=',1)
            stamp=dt.datetime.now(dt.timezone.utc).isoformat().replace('+00:00','Z')
            running=self.fault in ('partial','unkillable','kill_error')
            self.value=dict(Id='c'*64,Name='/'+name,Image=self.image,Created=stamp,
                Config=dict(Entrypoint=['/bin/sh'],Cmd=['-c','exit 0'],Labels={label[0]:label[1]}),
                HostConfig=dict(NetworkMode='none',ReadonlyRootfs=True,CapDrop=['ALL'],CapAdd=None,Privileged=False,
                    SecurityOpt=['no-new-privileges:true'],Memory=128*1024**2,NanoCpus=10**9,PidsLimit=32,AutoRemove=False),
                State=dict(Status='running' if running else 'exited',Running=running,Error='',OOMKilled=False,
                    ExitCode=0,StartedAt=stamp,FinishedAt=stamp))
            if self.fault=='foreign_image':self.value['Image']='sha256:'+'f'*64
            if self.fault=='foreign_label':self.value['Config']['Labels']={}
            if self.fault in ('partial','unkillable','kill_error'):raise w.subprocess.TimeoutExpired('synthetic run',10)
            if self.fault=='absent':self.value=None;raise w.subprocess.TimeoutExpired('synthetic run',10)
            return ''
        if args[0]=='kill':
            assert args[-1]==self.value['Id']
            if self.fault=='kill_error':raise OSError('synthetic kill failure')
            if args[1]=='--signal=KILL' and self.fault!='unkillable':self.value['State'].update(Running=False,Status='exited')
            return ''
        if args[0]=='wait':
            if self.value['State']['Running']:raise w.subprocess.TimeoutExpired('synthetic wait',timeout)
            return '0'
        if args[0]=='rm':
            assert args[-1]==self.value['Id'] and not self.value['State']['Running']
            if self.fault=='rm_error':raise OSError('synthetic rm failure')
            if self.fault=='replace_after_rm':self.value['Id']='d'*64
            else:self.value=None
            return ''
        raise AssertionError(args)

class LifetimeTests(unittest.TestCase):
    def harness(self):return HarnessTests.harness(self)
    def docker_lane(self,fault=None,publish_fault=False):
        h=self.harness();docker=LifetimeDocker('sha256:'+'a'*64,fault);runtime=mock.Mock();runtime.Docker.return_value=docker
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);original=h.publish
            def publish(path,value):
                if publish_fault and path.name=='docker-observation.json':raise OSError('synthetic publication failure')
                original(path,value)
            with mock.patch.object(h.w,'load_runtime',return_value=runtime),mock.patch.object(h,'publish',side_effect=publish):
                if fault or publish_fault:
                    with self.assertRaises((h.w.Rejected,OSError,w.subprocess.SubprocessError)):h.docker_lifetime(docker.image,root)
                else:self.assertEqual(h.docker_lifetime(docker.image,root),'PASS')
            receipt=json.loads((root/'docker-cleanup.json').read_text())
            intent=json.loads((root/'docker-intent.json').read_text())
        return docker,receipt,intent
    def test_docker_partial_launch_is_reconciled_and_bounded(self):
        docker,receipt,intent=self.docker_lane('partial')
        self.assertEqual(docker.creations,1);self.assertIsNone(docker.value)
        self.assertEqual(receipt['status'],'CLEAN');self.assertEqual(receipt['original_failure'],'TimeoutExpired')
        self.assertEqual(receipt['pin'][0],'c'*64);self.assertIn('TimeoutExpired',receipt['wait_errors'])
        actions=[a for a,t in docker.calls if a[0] in ('kill','wait','rm')]
        self.assertEqual([a[1] for a in actions if a[0]=='kill'],['--signal=TERM','--signal=KILL'])
        self.assertTrue(all(a[-1]=='c'*64 for a in actions));self.assertTrue(all(t<=5 for a,t in docker.calls if a[0]!='run'))
        for arg,value in [('--cpus','1'),('--memory','128m'),('--pids-limit','32')]:self.assertEqual(intent['argv'][intent['argv'].index(arg)+1],value)
    def test_docker_normal_and_publication_failure_both_cleanup(self):
        for publish_fault in (False,True):
            with self.subTest(publish_fault=publish_fault):
                docker,receipt,_=self.docker_lane(publish_fault=publish_fault)
                self.assertIsNone(docker.value);self.assertEqual(receipt['status'],'CLEAN');self.assertEqual(docker.creations,1)
                self.assertEqual(receipt['original_failure'],'OSError' if publish_fault else None)
    def test_docker_foreign_identity_is_retained_without_signal(self):
        for fault in ('foreign_image','foreign_label'):
            with self.subTest(fault=fault):
                docker,receipt,_=self.docker_lane(fault)
                self.assertEqual(receipt['status'],'UNKNOWN');self.assertIsNotNone(docker.value)
                self.assertFalse(any(a[0] in ('kill','rm') for a,t in docker.calls))
    def test_docker_cleanup_failure_and_absent_partial_launch_are_unknown(self):
        for fault in ('unkillable','kill_error','rm_error','replace_after_rm','absent'):
            with self.subTest(fault=fault):
                docker,receipt,_=self.docker_lane(fault)
                self.assertEqual(receipt['status'],'UNKNOWN');self.assertEqual(docker.creations,1)
                self.assertLessEqual(sum(a[0]=='kill' for a,t in docker.calls),2)
                if fault=='replace_after_rm':
                    self.assertEqual(docker.value['Id'],'d'*64)
                    self.assertFalse(any(a[-1]=='d'*64 for a,t in docker.calls if a[0] in ('kill','rm')))
    def test_container_created_or_cid_replacement_refused(self):
        for field,value in [('Id','d'*64),('Created','2020-01-01T00:00:00Z'),('Name','/foreign')]:
            with self.subTest(field=field):
                docker=LifetimeDocker('sha256:'+'a'*64)
                docker.call(['run','--name','owned','--label','lagrange.paper.disposable=n'])
                lease=w.OwnedContainer(docker,'owned',docker.image,w.time.time_ns(),lambda _:None)
                lease.observe(w.time.monotonic()+5);docker.value[field]=value;docker.calls=[]
                with self.assertRaises(w.Rejected):lease.cleanup(lambda _:None)
                self.assertFalse(any(a[0] in ('kill','rm') for a,t in docker.calls))
    def owned(self,group=False):
        owned=object.__new__(w.OwnedChild);owned.child=mock.Mock(pid=123);owned.identity=dict(pid=123,start_ticks=456)
        owned.group=group;owned.fd=9;owned.reaped=False;return owned
    def test_child_term_ignored_kill_reaps_only_exact_pidfd(self):
        for dies in (False,True):
            with self.subTest(dies=dies):
                owned=self.owned();now=[0.0];killed=[False]
                def send(fd,sig):
                    self.assertEqual(fd,9)
                    if sig==w.signal.SIGKILL:killed[0]=True
                def exit_status(*args):return object() if killed[0] and dies else None
                with mock.patch.object(owned,'check'),mock.patch.object(w.os,'waitid',side_effect=exit_status),mock.patch.object(w.signal,'pidfd_send_signal',side_effect=send) as signal,mock.patch.object(w.time,'monotonic',side_effect=lambda:now[0]),mock.patch.object(w.time,'sleep',side_effect=lambda n:now.__setitem__(0,now[0]+n)):
                    if dies:owned.cleanup()
                    else:
                        with self.assertRaisesRegex(w.Rejected,'UNKNOWN'):owned.cleanup()
                self.assertEqual([c.args[1] for c in signal.call_args_list],[w.signal.SIGTERM,w.signal.SIGKILL])
                self.assertLessEqual(now[0],10.01);owned.child.poll.assert_not_called()
                if dies:
                    owned.child.wait.assert_called_once();self.assertLessEqual(owned.child.wait.call_args.kwargs['timeout'],5)
                else:owned.child.wait.assert_not_called()
    def test_child_reused_pid_wrong_parent_or_session_is_not_signalled(self):
        for group,parent,pgrp,sid,identity in [(False,999,123,123,456),(True,100,999,123,456),(True,100,123,999,456),(False,100,123,123,789)]:
            owned=self.owned(group);parts=['S',str(parent),str(pgrp),str(sid)]+['0']*16
            with mock.patch.object(w,'process',return_value=dict(pid=123,start_ticks=identity)),mock.patch.object(w.os,'getpid',return_value=100),mock.patch.object(Path,'read_text',return_value='123 (child) '+' '.join(parts)),mock.patch.object(w.os,'killpg') as group_signal,mock.patch.object(w.signal,'pidfd_send_signal') as single_signal,self.assertRaises(w.Rejected):owned.send(w.signal.SIGTERM)
            group_signal.assert_not_called();single_signal.assert_not_called();owned.child.wait.assert_not_called()
    def test_session_leader_not_reaped_until_descendants_stopped(self):
        owned=self.owned(True);alive=[True];now=[0.0]
        def send(pid,sig):
            self.assertEqual(pid,123)
            if sig==w.signal.SIGKILL:alive[0]=False
        with mock.patch.object(owned,'check'),mock.patch.object(owned,'exited',return_value=True),mock.patch.object(owned,'live_group',side_effect=lambda:alive[0]),mock.patch.object(w.os,'killpg',side_effect=send) as signal,mock.patch.object(w.time,'monotonic',side_effect=lambda:now[0]),mock.patch.object(w.time,'sleep',side_effect=lambda n:now.__setitem__(0,now[0]+n)):
            owned.cleanup()
        self.assertEqual([c.args[1] for c in signal.call_args_list],[w.signal.SIGTERM,w.signal.SIGKILL])
        owned.child.wait.assert_called_once();self.assertFalse(alive[0])
    def test_nonblocking_output_overflow_and_read_stall_have_deadlines(self):
        for stall in (False,True):
            with self.subTest(stall=stall):
                owned=self.owned();owned.child.stdout.fileno.return_value=7;now=[0.0]
                def selected(*args):now[0]+=.25;return ([],[],[]) if stall else ([7],[],[])
                with mock.patch.object(w.time,'monotonic',side_effect=lambda:now[0]),mock.patch.object(w.select,'select',side_effect=selected),mock.patch.object(w.os,'set_blocking') as nonblocking,mock.patch.object(w.os,'read',return_value=b'x'*4096),self.assertRaises(w.Rejected):w.read_child_output(owned,3,limit=4096)
                nonblocking.assert_called_once_with(7,False);self.assertLessEqual(now[0],3.01);owned.child.wait.assert_not_called()
    def test_relay_read_failure_closes_stream_and_pidfd_on_second_cleanup_error(self):
        for cleanup_error in (False,True):
            child=mock.Mock();owned=mock.Mock(child=child)
            if cleanup_error:owned.cleanup.side_effect=w.Rejected('UNKNOWN')
            with mock.patch.object(w,'process',return_value=dict(pid=123,start_ticks=456)),mock.patch.object(w,'ipc_request',return_value={'name':'owned'}) as ipc,mock.patch.object(w.subprocess,'Popen',return_value=child),mock.patch.object(w,'OwnedChild',return_value=owned),mock.patch.object(w,'read_child_output',side_effect=w.Rejected('SOURCE_INVALID')),self.assertRaises(w.Rejected):w.relay_compose('capture',['docker','run','--rm'])
            owned.cleanup.assert_called_once();owned.close.assert_called_once();child.stdout.close.assert_called_once()
            self.assertEqual(ipc.call_count,1)  # never send success/end or redispatch
    def test_ipc_error_closes_private_duplicate(self):
        channel=mock.Mock();channel.recvmsg.side_effect=OSError('synthetic read failure')
        with mock.patch.dict(os.environ,{'LAGRANGE_PAPER_WITNESS_FD':'7'}),mock.patch.object(w.os,'fstat',return_value=mock.Mock(st_mode=0o140600)),mock.patch.object(w.os,'dup',return_value=8),mock.patch.object(w.socket,'socket',return_value=channel),self.assertRaises(w.Rejected):w.ipc_request({'op':'raw'})
        channel.close.assert_called_once()
    def test_scheduler_preserves_failure_claim_and_closes_fds_when_cleanup_fails(self):
        with tempfile.TemporaryDirectory() as folder:
            rig=SyntheticSource(folder)
            def fail(kind,j):
                if kind=='RUNNING':raise w.Rejected('CLOCK_INVALID')
            rig.event_hook=fail;timers=[];sockets=(mock.Mock(),mock.Mock());owners=[]
            with rig.patches():
                timer=w.Timer;owner=w.OwnedChild
                def create_timer(due):result=timer(due);result.close=mock.Mock();timers.append(result);return result
                def create_owner(*args,**kwargs):
                    result=owner(*args,**kwargs);result.cleanup=mock.Mock(side_effect=w.Rejected('UNKNOWN'));result.close=mock.Mock();owners.append(result);return result
                with mock.patch.object(w,'Timer',side_effect=create_timer),mock.patch.object(w,'OwnedChild',side_effect=create_owner),mock.patch.object(w.socket,'socketpair',return_value=sockets),mock.patch.object(rig.scheduler,'abort_owned',side_effect=w.Rejected('ORIGIN_UNPROVEN')),self.assertRaisesRegex(w.Rejected,'UNKNOWN'):rig.scheduler.run_session(rig.session)
            self.assertEqual(rig.scheduler.j.events[-1]['data'],{'code':'CLOCK_INVALID'})
            notes=[json.loads(p.read_bytes()) for p in (rig.root/'evidence/.control').glob('*-cleanup-*.json')]
            self.assertTrue(any(n['status']=='UNKNOWN' and n['original_failure']=='CLOCK_INVALID' and len(n['errors'])==2 for n in notes))
            for timer in timers:timer.close.assert_called_once()
            for channel in sockets:channel.close.assert_called_once()
            owners[0].close.assert_called_once();self.assertEqual(rig.launches,1);self.assertEqual(rig.provider_phases,[])
            rig.event_hook=None
            with self.assertRaises((w.Rejected,OSError)):rig.run()
            self.assertEqual(rig.launches,1);self.assertEqual(rig.provider_phases,[])
    def test_second_timer_constructor_failure_closes_first(self):
        with tempfile.TemporaryDirectory() as folder:
            rig=SyntheticSource(folder);first=mock.Mock()
            with rig.patches(),mock.patch.object(w,'Timer',side_effect=[first,w.Rejected('CLOCK_INVALID')]),self.assertRaises(w.Rejected):rig.scheduler.run_session(rig.session)
            first.close.assert_called_once();self.assertEqual(rig.launches,0)
            self.assertEqual(rig.scheduler.j.events[-1]['data']['code'],'CLOCK_INVALID')
    def test_helper_partial_launch_cleanup_does_not_retry_or_dispatch(self):
        with tempfile.TemporaryDirectory() as folder:
            rig=SyntheticSource(folder);call=rig.call
            def fail(args,**kwargs):
                result=call(args,**kwargs)
                if args[0]=='run':raise rig.runtime.RuntimeErrorCode('SYNTHETIC_PARTIAL_LAUNCH')
                return result
            rig.call=fail
            with self.assertRaises(w.Rejected):rig.run()
            self.assertEqual(rig.helper_ops,['reserve']);self.assertEqual(rig.launches,0);self.assertFalse(rig.objects)
            self.assertEqual(rig.scheduler.j.events[-1]['kind'],'FAILED')

    def test_max_source_deadline_reaches_cleanup_without_provider_dispatch(self):
        with tempfile.TemporaryDirectory() as folder:
            rig=SyntheticSource(folder);now=[0.0];owners=[]
            with rig.patches():
                owner=w.OwnedChild
                def create(*args,**kwargs):
                    result=owner(*args,**kwargs)
                    def exited():now[0]=61;return False
                    result.exited=exited;result.cleanup=mock.Mock();result.close=mock.Mock();owners.append(result);return result
                with mock.patch.object(w,'OwnedChild',side_effect=create),mock.patch.object(w.time,'monotonic',side_effect=lambda:now[0]),self.assertRaisesRegex(w.Rejected,'UNKNOWN'):rig.scheduler.run_session(rig.session)
            self.assertEqual(rig.launches,1);self.assertEqual(rig.provider_phases,[])
            owners[0].cleanup.assert_called_once();owners[0].close.assert_called_once()
            self.assertEqual(rig.scheduler.j.events[-1]['kind'],'FAILED')
    def test_descriptor_close_error_preserves_original_failure_and_closes_rest(self):
        with tempfile.TemporaryDirectory() as folder:
            rig=SyntheticSource(folder);timers=[]
            def fail(kind,j):
                if kind=='ARMED':raise w.Rejected('CLOCK_INVALID')
            rig.event_hook=fail
            with rig.patches():
                timer=w.Timer
                def create(due):
                    result=timer(due);result.close=mock.Mock(side_effect=OSError('synthetic close failure') if not timers else None);timers.append(result);return result
                with mock.patch.object(w,'Timer',side_effect=create),self.assertRaisesRegex(w.Rejected,'UNKNOWN'):rig.scheduler.run_session(rig.session)
            for timer in timers:timer.close.assert_called_once()
            self.assertEqual(rig.scheduler.j.events[-1]['data']['code'],'CLOCK_INVALID')
            notes=[json.loads(p.read_bytes()) for p in (rig.root/'evidence/.control').glob('*-cleanup-*.json')]
            self.assertTrue(any(n['scope']=='descriptors' and n['status']=='UNKNOWN' for n in notes))
    def test_helper_original_failure_survives_second_cleanup_failure(self):
        with tempfile.TemporaryDirectory() as folder:
            rig=SyntheticSource(folder);call=rig.call
            def fail(args,**kwargs):
                if args[0]=='rm':raise OSError('synthetic cleanup failure')
                result=call(args,**kwargs)
                if args[0]=='run':raise w.Rejected('DB_UNAVAILABLE')
                return result
            rig.call=fail
            with self.assertRaisesRegex(w.Rejected,'UNKNOWN'):rig.run()
            self.assertEqual(rig.scheduler.j.events[-1]['data']['code'],'DB_UNAVAILABLE')
            self.assertEqual(rig.launches,0);self.assertEqual(rig.helper_ops,['reserve']);self.assertEqual(len(rig.objects),1)
            notes=[json.loads(p.read_bytes()) for p in (rig.root/'evidence/.control').glob('*-cleanup-*.json')]
            self.assertTrue(any(n['status']=='UNKNOWN' and n['original_failure']=='DB_UNAVAILABLE' and n.get('pin') for n in notes))
    def test_successful_nonblocking_relay_reports_exact_bytes_and_closes(self):
        owned=self.owned();owned.wait=mock.Mock(return_value=0);owned.close=mock.Mock();owned.child.stdout.fileno.return_value=7
        owned.acquire=mock.Mock()  # the supplied lease is already a synthetic acquired child
        replies=[dict(name='owned'),dict(ok=True)];out=io.BytesIO()
        with mock.patch.object(w,'process',return_value=owned.identity),mock.patch.object(w,'ipc_request',side_effect=replies) as ipc,mock.patch.object(w.subprocess,'Popen',return_value=owned.child),mock.patch.object(w,'OwnedChild',return_value=owned),mock.patch.object(w.select,'select',return_value=([7],[],[])),mock.patch.object(w.os,'set_blocking'),mock.patch.object(w.os,'read',side_effect=[b'{"synthetic":true}\n',b'']),mock.patch.object(w.sys,'stdout',mock.Mock(buffer=out)):
            self.assertEqual(w.relay_compose('capture',['docker','run','--rm']),0)
        self.assertEqual(out.getvalue(),b'{"synthetic":true}\n')
        self.assertEqual(ipc.call_args.args[0]['output'],out.getvalue().decode());self.assertEqual(ipc.call_count,2)
        owned.close.assert_called_once();owned.child.stdout.close.assert_called_once()

    def test_pidfd_opened_before_identity_check_and_closed_on_refusal(self):
        child=mock.Mock(pid=123)
        with mock.patch.object(w,'process',return_value=dict(pid=123,start_ticks=456)),mock.patch.object(w.os,'pidfd_open',return_value=9) as opened,mock.patch.object(w.OwnedChild,'check',side_effect=w.Rejected('ORIGIN_UNPROVEN')),mock.patch.object(w.os,'close') as closed,mock.patch.object(w.os,'killpg') as group_signal,mock.patch.object(w.signal,'pidfd_send_signal') as single_signal,self.assertRaises(w.Rejected):w.OwnedChild(child,group=True)
        opened.assert_called_once_with(123,0);closed.assert_called_once_with(9)
        group_signal.assert_not_called();single_signal.assert_not_called();child.wait.assert_not_called()
    def test_final_close_failure_never_leaves_linked_as_terminal_success(self):
        with tempfile.TemporaryDirectory() as folder:
            rig=SyntheticSource(folder);timers=[]
            with rig.patches():
                timer=w.Timer
                def create(due):
                    result=timer(due);result.close=mock.Mock(side_effect=OSError('synthetic close failure') if not timers else None);timers.append(result);return result
                with mock.patch.object(w,'Timer',side_effect=create),self.assertRaisesRegex(w.Rejected,'UNKNOWN'):rig.scheduler.run_session(rig.session)
            self.assertEqual(rig.scheduler.j.events[-2]['kind'],'RUN_LINKED')
            self.assertNotIn('LINKED',[event['kind'] for event in rig.scheduler.j.events])
            self.assertEqual(rig.scheduler.j.events[-1]['kind'],'FAILED')
            with self.assertRaises(w.Rejected):w.verify_chain(rig.scheduler.j.events,'LINKED')
            for timer in timers:timer.close.assert_called_once()
            self.assertFalse(rig.objects)

    def test_socket_constructor_failure_closes_untransferred_duplicate(self):
        with mock.patch.dict(os.environ,{'LAGRANGE_PAPER_WITNESS_FD':'7'}),mock.patch.object(w.os,'fstat',return_value=mock.Mock(st_mode=0o140600)),mock.patch.object(w.os,'dup',return_value=8),mock.patch.object(w.socket,'socket',side_effect=OSError('synthetic constructor failure')),mock.patch.object(w.os,'close') as close,self.assertRaises(w.Rejected):w.ipc_request({'op':'raw'})
        close.assert_called_once_with(8)

class UnitLifetimeTests(unittest.TestCase):
    def harness(self):return HarnessTests.harness(self)
    def show(self,unit,case,expected,state='active',foreign=None):
        values=dict(Id=unit,LoadState='loaded',FragmentPath='/run/systemd/transient/'+unit,
            ExecStart='{ path='+expected[0]+' ; argv[]='+' '.join(expected)+' ; ignore_errors=no ; pid=123 ; code=exited ; status=0 }',
            MainPID='123' if state=='active' else '0',ControlPID='0',ActiveState=state,Result='success',
            ReadWritePaths=str(case),KillMode='control-group',Type='exec',Restart='no')
        if state=='absent':values.update(LoadState='not-found',ExecStart='',FragmentPath='',MainPID='0',ActiveState='inactive')
        if foreign=='path':values['FragmentPath']='/etc/systemd/system/'+unit
        if foreign=='argv':values['ExecStart']=values['ExecStart'].replace(' ; ignore_errors=no',' --foreign ; ignore_errors=no')
        if foreign=='run_root':values['ReadWritePaths']=str(case)+'-foreign'
        if foreign=='unit':values['Id']='foreign.service'
        return ''.join(k+'='+v+'\n' for k,v in values.items()).encode()
    def test_partial_systemd_run_error_still_cleans_exact_unit(self):
        for fault in ('launch_timeout','launch_error','absent','foreign_argv','foreign_unit','stop_error','reset_error'):
            with self.subTest(fault=fault),tempfile.TemporaryDirectory() as folder:
                h=self.harness();root=Path(folder);case=root/'ARMED';calls=[];state=['active'];identity={}
                def command(argv,**kwargs):
                    calls.append((list(argv),kwargs));self.assertLessEqual(kwargs.get('timeout',5),10)
                    if argv[0]=='/usr/bin/systemd-run':
                        identity['unit']=next(a[7:] for a in argv if a.startswith('--unit='));identity['expected']=argv[argv.index('/usr/bin/python3'):]
                        self.assertIn('--property=TimeoutStopSec=5s',argv)
                        if fault=='absent':state[0]='absent'
                        if fault=='launch_error':raise h.w.Rejected('COMMAND_FAILED')
                        raise w.subprocess.TimeoutExpired('synthetic systemd-run',10)
                    if argv[1]=='show':
                        foreign='argv' if fault=='foreign_argv' else 'unit' if fault=='foreign_unit' else None
                        return self.show(identity['unit'],case,identity['expected'],state[0],foreign)
                    self.assertEqual(argv[-1],identity['unit'])
                    if argv[1]=='stop':
                        if fault=='stop_error':raise OSError('synthetic stop failure')
                        state[0]='inactive';return b''
                    if argv[1]=='reset-failed':
                        if fault=='reset_error':raise OSError('synthetic reset failure')
                        state[0]='absent';return b''
                    raise AssertionError(argv)
                with mock.patch.object(h,'BOUNDARIES',('ARMED',)),mock.patch.object(h,'load_synthetic',return_value=mock.Mock()),mock.patch.object(h,'command',side_effect=command),self.assertRaises((h.w.Rejected,w.subprocess.TimeoutExpired)):h.systemd_crashes(mock.Mock(vm_id=str(uuid.uuid4())),root)
                receipt=json.loads((case/'unit-cleanup.json').read_text())
                self.assertEqual(receipt['status'],'CLEAN' if fault in ('launch_timeout','launch_error') else 'UNKNOWN')
                self.assertIn(receipt['original_failure'],('TimeoutExpired','Rejected'))
                mutations=[a[1] for a,k in calls if a[0]=='/usr/bin/systemctl' and a[1]!='show']
                if fault in ('absent','foreign_argv','foreign_unit'):self.assertEqual(mutations,[])
                self.assertEqual(sum(a[0]=='/usr/bin/systemd-run' for a,k in calls),1)
                self.assertNotIn('restart',mutations);self.assertNotIn('kill',mutations)
    def test_exact_unit_execstart_and_runroot_required_before_stop(self):
        h=self.harness();unit='lagrange-paper-disposable-'+str(uuid.uuid4())+'.service';case=Path('/run/case')
        expected=['/usr/bin/python3','/script','--run-root',str(case)]
        for foreign in ('path','argv','run_root','unit'):
            with self.subTest(foreign=foreign),mock.patch.object(h,'command',return_value=self.show(unit,case,expected,foreign=foreign)) as command,self.assertRaises(h.w.Rejected):h.cleanup_unit(unit,case,expected)
            self.assertEqual([c.args[0][1] for c in command.call_args_list],['show'])
    def test_unit_replacement_between_stop_and_reset_is_not_mutated(self):
        h=self.harness();unit='lagrange-paper-disposable-'+str(uuid.uuid4())+'.service';case=Path('/run/case');expected=['/usr/bin/python3','/script']
        replies=[self.show(unit,case,expected),b'',self.show(unit,case,expected,state='inactive',foreign='argv')]
        with mock.patch.object(h,'command',side_effect=replies) as command,self.assertRaises(h.w.Rejected):h.cleanup_unit(unit,case,expected)
        self.assertEqual([c.args[0][1] for c in command.call_args_list],['show','stop','show'])
    def test_unit_stop_stall_is_finite_unknown(self):
        h=self.harness();unit='lagrange-paper-disposable-'+str(uuid.uuid4())+'.service';case=Path('/run/case');expected=['/usr/bin/python3','/script'];now=[0.0]
        def command(argv,**kwargs):return self.show(unit,case,expected) if argv[1]=='show' else b''
        with mock.patch.object(h,'command',side_effect=command) as calls,mock.patch.object(h.time,'monotonic',side_effect=lambda:now[0]),mock.patch.object(h.time,'sleep',side_effect=lambda n:now.__setitem__(0,now[0]+n)),self.assertRaises(h.w.Rejected):h.cleanup_unit(unit,case,expected)
        self.assertLessEqual(now[0],20.01);self.assertFalse(any(c.args[0][1]=='reset-failed' for c in calls.call_args_list))
    def test_systemd_normal_crash_sequence_checks_ownership_before_each_mutation(self):
        h=self.harness();calls=[];state=['active'];identity={}
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder);case=root/'ARMED'
            def source(path):(path/'evidence').mkdir()
            def command(argv,**kwargs):
                calls.append(argv)
                if argv[0]=='/usr/bin/systemd-run':
                    identity['unit']=next(a[7:] for a in argv if a.startswith('--unit='));identity['expected']=argv[argv.index('/usr/bin/python3'):];return b''
                if argv[1]=='show':return self.show(identity['unit'],case,identity['expected'],state[0])
                self.assertEqual(calls[-2][1],'show');self.assertEqual(argv[-1],identity['unit'])
                if argv[1]=='stop':state[0]='inactive'
                if argv[1]=='reset-failed':state[0]='absent'
                return b''
            module=mock.Mock();module.SyntheticSource.side_effect=source
            with mock.patch.object(h,'BOUNDARIES',('ARMED',)),mock.patch.object(h,'load_synthetic',return_value=module),mock.patch.object(h,'command',side_effect=command),mock.patch.object(h,'wait_file'),mock.patch.object(h.w,'read_file',return_value=w.canonical(dict(launches=0,provider_phases=[]))):
                self.assertEqual(h.systemd_crashes(mock.Mock(vm_id=str(uuid.uuid4())),root),'SYNTHETIC_PASS')
            self.assertEqual(json.loads((case/'unit-cleanup.json').read_text())['status'],'CLEAN')
            self.assertEqual([a[1] for a in calls if a[0]=='/usr/bin/systemctl' and a[1]!='show'],['kill','restart','stop','reset-failed'])

CURRENT_FIXTURES=ROOT/'tests/fixtures/owner_beta_paper_current_binding'
def current_fixture(name='valid-current.json'):return w.decode((CURRENT_FIXTURES/name).read_bytes())

def helper_projection_fixture(binding,root='/synthetic/artifacts/historical-price-beta-root'):
    # Ordinary immutable metadata only; this never creates Current authority.
    return w._HelperProjection(w.digest(w.canonical(binding)),binding['production_env_sha256'],
        binding['cohort']['cohort_sha256'],root)

def binding_projection_fixture(binding):
    return binding,w.canonical(binding),helper_projection_fixture(binding)

class PublicationMemory:
    """In-memory syscall model: never chown, OFD-lock or open a kernel FD."""
    def __init__(self):
        self.value=current_fixture();self.b=self.value['binding'];self.clock=dict(self.value['observed']);self.nodes={};self.fds={};self.next_fd=100;self.next_inode=1000
        self.audit=[];self.fail=None;self.locks=set();self.named_lock=11
        self.fds[11]=dict(node=dict(inode=11,data=b'',uid=0,gid=0,mode=0o600,nlink=1),pos=0,flags=os.O_RDWR,inheritable=False)
        self.fds[12]=dict(node=dict(inode=12,data=b'',uid=0,gid=0,mode=0o600,nlink=1),pos=0,flags=os.O_RDWR,inheritable=False)
    def check(self,operation):
        self.audit.append((operation,))
        if self.fail==operation:raise OSError('SYNTHETIC_'+operation)
    def fsync(self,fd):
        self.check('fsync')
        if self.fail=='directory_fsync' and fd==10:raise OSError('SYNTHETIC_directory_fsync')
    @contextlib.contextmanager
    def opened(self,path,**kwargs):
        yield self.named_lock if str(path).endswith('/.control/lock') else 10
    def open(self,name,flags,mode=0o777,dir_fd=None):
        self.check('open');assert flags&os.O_NOFOLLOW and flags&os.O_CLOEXEC
        if flags&os.O_CREAT:
            assert name not in self.nodes and flags&os.O_EXCL and flags&os.O_RDWR
            node=dict(inode=self.next_inode,data=b'',uid=0,gid=0,mode=mode,nlink=1);self.next_inode+=1;self.nodes[name]=node
        else:
            if name not in self.nodes:raise FileNotFoundError(name)
            node=self.nodes[name]
        fd=self.next_fd;self.next_fd+=1;self.fds[fd]=dict(node=node,pos=0,flags=flags,inheritable=False);return fd
    def info(self,node):
        return mock.Mock(st_dev=1,st_ino=node['inode'],st_uid=node['uid'],st_gid=node['gid'],st_mode=w.stat.S_IFREG|node['mode'],st_nlink=node['nlink'],st_size=len(node['data']),st_mtime_ns=1,st_ctime_ns=1)
    def fstat(self,fd):return self.info(self.fds[fd]['node'])
    def stat(self,name,**kwargs):return self.info(self.nodes[name])
    def read(self,fd,size):
        entry=self.fds[fd];start=entry['pos'];entry['pos']+=size;return entry['node']['data'][start:start+size]
    def write(self,fd,data):self.check('write');self.fds[fd]['node']['data']+=data;return len(data)
    def close(self,fd):self.audit.append(('close',fd));self.fds.pop(fd);self.locks.discard(fd)
    def chown(self,fd,uid,gid):self.check('chown');self.fds[fd]['node'].update(uid=uid,gid=gid)
    def chmod(self,fd,mode):self.check('chmod');self.fds[fd]['node']['mode']=mode
    def replace(self,source,target,**kwargs):self.check('replace');self.nodes[target]=self.nodes.pop(source)
    def unlink(self,name,**kwargs):self.nodes.pop(name)
    def ofd(self,fd,operation,raw):
        self.check('ofd');assert operation==w.OFD_SETLK==37
        lock=w.OfdFlock.from_buffer_copy(raw);assert (lock.type,lock.whence,lock.start,lock.length,lock.pid)==(w.fcntl.F_WRLCK,os.SEEK_SET,0,0,0)
        assert self.fds[fd]['flags']&os.O_RDWR and self.fds[fd]['node']['mode']==0o600
        self.locks.add(fd);self.audit.append(('locked',fd));return raw
    def advance(self,seconds):self.clock={k:v+int(seconds*10**9) for k,v in self.clock.items()}
    def leaf(self):return w.decode(self.nodes[w.CURRENT_LEAF]['data'])
    @contextlib.contextmanager
    def patches(self):
        read_text=Path.read_text
        with contextlib.ExitStack() as stack:
            patches=[(w,'opened',self.opened),(w,'sample',lambda:dict(self.clock)),(w,'process',lambda _:dict(pid=100,start_ticks=200)),
                (Path,'read_text',lambda path,*a,**k:self.value['boot_id'] if str(path)=='/proc/sys/kernel/random/boot_id' else read_text(path,*a,**k)),
                (w.os,'open',self.open),(w.os,'close',self.close),(w.os,'fstat',self.fstat),(w.os,'stat',self.stat),
                (w.os,'read',self.read),(w.os,'write',self.write),(w.os,'fchown',self.chown),(w.os,'fchmod',self.chmod),
                (w.os,'fsync',self.fsync),(w.os,'replace',self.replace),(w.os,'unlink',self.unlink),
                (w.os,'listxattr',lambda fd:[]),(w.os,'set_inheritable',lambda fd,v:self.fds[fd].update(inheritable=v)),
                (w.os,'get_inheritable',lambda fd:self.fds[fd]['inheritable']),
                (w.os,'link',mock.Mock(side_effect=AssertionError('mutable publication must not hardlink'))),
                (w.fcntl,'fcntl',self.ofd),(w.fcntl,'flock',lambda fd,flags:self.audit.append(('global_lock',fd,flags)))]
            for obj,name,value in patches:stack.enter_context(mock.patch.object(obj,name,value))
            yield
    def authority(self):
        scheduler=mock.Mock(b=self.b,path='/synthetic/private/operator-binding.json',pin=w.canonical(self.b))
        return w.CurrentAuthority(scheduler,11)

class CurrentWireTests(unittest.TestCase):
    def test_frozen_shared_valid_four_and_rejected_nineteen(self):
        for name in ('valid-current.json','valid-refresh.json','valid-invalid-starting.json','valid-invalid-stopped.json'):w.validate_current(current_fixture(name))
        self.assertEqual(w.CURRENT_SCHEMA['$defs'],{k:w.SCHEMA['$defs'][k] for k in w.CURRENT_SCHEMA['$defs']})
        cases=json.loads((CURRENT_FIXTURES/'rejections.json').read_bytes());self.assertEqual(len(cases),19)
        # Historical layout mutations retain stale semantic hashes. Amendment1
        # layout decisions additionally use fully rehashed cases below.
        for case in cases:
            with self.subTest(case=case['name']):
                value=current_fixture();parts=case['path'][1:].split('/');cursor=value
                for key in parts[:-1]:cursor=cursor[key]
                cursor[parts[-1]]=case['value']
                if parts[0]=='binding' and value['binding'] is not None:value['binding_sha256']=w.digest(w.canonical(value['binding']))
                with self.assertRaises(w.Rejected):w.validate_current(value)
    def test_current_canonical_duplicate_and_secret_rejection(self):
        raw=w.canonical(current_fixture())
        for wire in (b' '+raw,raw.replace(b'"sequence":1',b'"sequence":1,"sequence":2'),raw.replace(b'"sequence":1',b'"sequence":1.0'),raw.replace(b'"sequence":1',b'"sequence":true')):
            with self.assertRaises(w.Rejected):w.validate_current(w.decode(wire))
        for binding,digest in [(None,'sha256:'+'0'*64),(current_fixture()['binding'],None)]:
            value=current_fixture('valid-invalid-starting.json');value.update(binding=binding,binding_sha256=digest)
            with self.assertRaises(w.Rejected):w.validate_current(value)
    def test_layout_exact_directory_projections_and_no_secret_ancestors(self):
        b=current_fixture()['binding'];runtime=w.load_runtime();slot=fixture()['events'][0]['slot_id']
        for op in ('reserve','link-admission','link-run','read'):
            args=runtime.paper_evidence_helper_args(b,op,slot,('identity',) if op.startswith('link-') else (),'lagrange-paper-fixture',worker_historical_root='/synthetic/artifacts/historical-price-beta-root')
            mounts={}
            for i,arg in enumerate(args):
                if arg=='--mount':
                    self.assertTrue(args[i+1].endswith(',readonly'));values=dict(x.split('=',1) for x in args[i+1].split(',') if '=' in x);mounts[values['dst']]=values['src']
            self.assertEqual(mounts['/data/owner-beta-paper-evidence/current'],b['evidence_root']+'/current')
            self.assertNotIn('/run/owner-beta-paper/calendar',mounts)
            self.assertNotIn('/run/owner-beta-paper/release',mounts)
            self.assertNotIn(b['calendar_dir'],mounts.values());self.assertNotIn(b['release_root'],mounts.values())
            self.assertEqual(mounts['/run/owner-beta-daily/config.json'],b['daily_config_file'])
            self.assertEqual(set(mounts),{'/data/owner-beta-paper-evidence/current','/data/owner-beta-paper-evidence/events',
                '/run/owner-beta-daily/config.json','/run/owner-beta-daily/policy.json','/data/owner-beta-artifacts',
                '/data/owner-beta-daily/admitted','/data/raw','/run/secrets/'+('db_daily_input_admitter_password' if op in ('reserve','link-admission') else 'db_worker_password')})
            self.assertNotIn(b['production_env_file'],mounts.values());self.assertNotIn(b['secret_root'],mounts.values())
        for field,value in [('production_env_file',b['evidence_root']+'/current/env'),('secret_root',b['data_root']+'/raw/secrets'),('daily_config_file','/run/owner-beta-daily/config.json')]:
            bad=dict(b,**{field:value})
            with self.assertRaises(w.Rejected):w.binding_structure(bad)
            with self.assertRaises(runtime.RuntimeErrorCode):runtime.paper_evidence_helper_args(bad,'read',slot,(),'lagrange-paper-fixture',worker_historical_root='/synthetic/artifacts/historical-price-beta-root')
    def test_actual_helper_mount_allowlist_rejects_leaf_extra_writable_and_env(self):
        with tempfile.TemporaryDirectory() as folder:
            rig=SyntheticSource(folder)
            with rig.patches():
                rig.scheduler.j=w.Journal(rig.b,rig.session)
                args=rig.runtime.paper_evidence_helper_args(rig.b,'reserve',rig.scheduler.j.slot,(),'lagrange-paper-fixture',worker_historical_root='/synthetic/artifacts/historical-price-beta-root')
                rig.database=lambda _:{}
                rig.call(args);value=next(iter(rig.objects.values()))
                rig.runtime.verify_paper_evidence_helper(value,rig.b,'reserve',rig.scheduler.j.slot,(),'lagrange-paper-fixture')
                for fault in ('leaf','extra','writable','env','full_release','calendar'):
                    changed=copy.deepcopy(value)
                    if fault=='leaf':next(x for x in changed['Mounts'] if x['Destination'].endswith('/current'))['Source']+='/current-binding.json'
                    elif fault=='extra':changed['Mounts'].append(dict(Type='bind',RW=False,Source='/extra',Destination='/extra'))
                    elif fault=='writable':changed['Mounts'][0]['RW']=True
                    elif fault in ('full_release','calendar'):
                        key,target=('release_root','release') if fault=='full_release' else ('calendar_dir','calendar')
                        changed['Mounts'].append(dict(Type='bind',RW=False,Source=rig.b[key],Destination='/run/owner-beta-paper/'+target))
                    else:changed['Config']['Env'].append('KIS_APP_SECRET=SYNTHETIC_SECRET')
                    with self.assertRaises(rig.runtime.RuntimeErrorCode):rig.runtime.verify_paper_evidence_helper(changed,rig.b,'reserve',rig.scheduler.j.slot,(),'lagrange-paper-fixture')

class CurrentCompatibilityTests(unittest.TestCase):
    def envelope(self,key,path):
        value=current_fixture();value['binding'][key]=path
        value['binding']['cohort']['semantic_config_sha256']=w.semantic_hash(value['binding'])
        value['binding_sha256']=w.digest(w.canonical(value['binding']))
        return value
    def test_fully_hashed_private_layout_accepts_unmounted_roots_only(self):
        b=current_fixture()['binding'];runtime=w.load_runtime();slot=fixture()['events'][0]['slot_id']
        accepted=[('production_env_file',b['release_root']+'/deploy/compose/.env'),('secret_root',b['release_root']+'/private-secrets'),
            ('production_env_file',b['calendar_dir']+'/private.env'),('calendar_file','/synthetic/private/separate-calendar.json')]
        denied=[]
        for public in (b['evidence_root']+'/current',b['evidence_root']+'/events',b['data_root']+'/raw',b['data_root']+'/owner-beta-daily/admitted',b['data_root']+'/owner-beta-daily/historical-base/admitter'):
            for key in ('production_env_file','secret_root'):denied.extend([(key,public),(key,public+'/private')])
        for public in (b['daily_config_file'],b['release_root']+'/.owner-beta-daily/policy.json'):
            for key in ('production_env_file','secret_root'):denied.append((key,public))
        for allowed,cases in ((True,accepted),(False,denied)):
            for key,path in cases:
                with self.subTest(allowed=allowed,key=key,path=path):
                    value=self.envelope(key,path);selected=value['binding']
                    self.assertEqual(value['binding_sha256'],w.digest(w.canonical(selected)))
                    self.assertEqual(selected['cohort']['semantic_config_sha256'],w.semantic_hash(selected))
                    if allowed:
                        w.validate_current(value);w.binding_structure(selected)
                        args=runtime.paper_evidence_helper_args(selected,'read',slot,(),'lagrange-paper-fixture',worker_historical_root='/synthetic/artifacts/historical-price-beta-root')
                        self.assertNotIn('dst=/run/owner-beta-paper/release',str(args));self.assertNotIn('dst=/run/owner-beta-paper/calendar',str(args))
                    else:
                        with self.assertRaises(w.Rejected):w.validate_current(value)
                        with self.assertRaises(w.Rejected):w.binding_structure(selected)
                        with self.assertRaises(runtime.RuntimeErrorCode):runtime.paper_evidence_helper_args(selected,'read',slot,(),'lagrange-paper-fixture',worker_historical_root='/synthetic/artifacts/historical-price-beta-root')
    def test_nested_private_release_files_are_never_public_mount_ancestors(self):
        b=current_fixture()['binding'];runtime=w.load_runtime();slot=fixture()['events'][0]['slot_id']
        # Even when the selected env is elsewhere, an unselected root600 env
        # may exist inside the release. No file contents need to be inspected.
        private_paths=[b['release_root']+'/deploy/compose/.env',b['release_manifest_file'],b['calendar_file'],b['production_env_file']]
        for op in ('reserve','link-admission','link-run','read'):
            args=runtime.paper_evidence_helper_args(b,op,slot,('identity',) if op.startswith('link-') else (),'lagrange-paper-fixture',worker_historical_root='/synthetic/artifacts/historical-price-beta-root')
            mounts=[dict(x.split('=',1) for x in args[i+1].split(',') if '=' in x) for i,arg in enumerate(args) if arg=='--mount']
            self.assertEqual(len(mounts),8)
            for mount in mounts:
                for path in private_paths:self.assertFalse(path==mount['src'] or path.startswith(mount['src']+'/'))
            self.assertEqual(sum(m['dst'].startswith('/run/secrets/') for m in mounts),1)
    def test_operator_binding_stays_private_under_actual_mounts(self):
        b=current_fixture()['binding'];wire=w.canonical(b)
        private=b['release_root']+'/.private/operator-binding.json'
        with mock.patch.object(w,'read_file',return_value=wire) as read:
            self.assertEqual(w.structural_binding(private),(b,wire))
            read.assert_called_once_with(private,w.SOURCE_LIMIT,exact=(0,0,0o600))
        for public in w.public_mount_paths(b):
            for path in (public,public+'/binding.json'):
                with mock.patch.object(w,'read_file',return_value=wire),self.assertRaises(w.Rejected):w.structural_binding(path)

class CurrentPublicationTests(unittest.TestCase):
    def test_production_publication_emits_exact_frozen_shared_wires(self):
        fs=PublicationMemory()
        expected={name:(CURRENT_FIXTURES/name).read_bytes() for name in ('valid-invalid-starting.json','valid-current.json','valid-refresh.json','valid-invalid-stopped.json')}
        with fs.patches(),mock.patch.object(w,'process',return_value=fs.value['publisher']),mock.patch.object(w.uuid,'uuid4',return_value=uuid.UUID(fs.value['generation'])):
            pub=w.CurrentPublication(fs.b,11)
            for name,status,reason,binding,advance in [('valid-invalid-starting.json','INVALID','STARTING',None,0),('valid-current.json','CURRENT','NONE',fs.b,0),('valid-refresh.json','CURRENT','NONE',fs.b,15),('valid-invalid-stopped.json','INVALID','STOPPED',fs.b,15)]:
                fs.advance(advance);pub.emit(status,reason,binding)
                self.assertEqual(fs.nodes[w.CURRENT_LEAF]['data'],expected[name])
            self.assertFalse(fs.locks);self.assertEqual(pub.fd,-1)
    def test_atomic_new_inode_write_fence_and_monotonic_generation(self):
        fs=PublicationMemory()
        with fs.patches():
            pub=w.CurrentPublication(fs.b,11);a=pub.emit('INVALID','STARTING',None)
            self.assertEqual(a['sequence'],0);self.assertEqual(pub.fd,-1)
            b=pub.emit('CURRENT','NONE',fs.b);first_fd=pub.fd;first_inode=pub.expected[:2]
            self.assertIn(first_fd,fs.locks);self.assertFalse(fs.fds[first_fd]['inheritable']);self.assertEqual(fs.fds[first_fd]['node']['nlink'],1)
            fs.advance(15);c=pub.emit('CURRENT','NONE',fs.b)
            self.assertEqual((b['generation'],b['sequence']),(c['generation'],1));self.assertEqual(c['sequence'],2)
            self.assertNotEqual(first_inode,pub.expected[:2]);self.assertNotIn(first_fd,fs.fds)
            self.assertLess(fs.audit.index(('locked',pub.fd)),fs.audit.index(('close',first_fd)))
            pub.emit('INVALID','STOPPED',fs.b);self.assertEqual(pub.fd,-1);self.assertFalse(fs.locks)
            self.assertEqual(fs.leaf()['reason'],'STOPPED');self.assertEqual(fs.leaf()['sequence'],3)
    def test_publish_failure_always_closes_previous_authority(self):
        for failure in ('write','fsync','ofd','chown','chmod','replace','directory_fsync'):
            with self.subTest(failure=failure):
                fs=PublicationMemory()
                with fs.patches():
                    pub=w.CurrentPublication(fs.b,11);pub.emit('CURRENT','NONE',fs.b);fs.fail=failure
                    with self.assertRaises(OSError):pub.emit('CURRENT','NONE',fs.b)
                    self.assertEqual(pub.fd,-1);self.assertFalse(fs.locks)
                    self.assertFalse(any(n.startswith('.') for n in fs.nodes))
    def test_unsupported_ofd_api_fails_closed_before_publication(self):
        fs=PublicationMemory()
        with fs.patches(),mock.patch.object(w,'OFD_SETLK',None):
            pub=w.CurrentPublication(fs.b,11)
            with self.assertRaisesRegex(w.Rejected,'ORIGIN_UNPROVEN'):pub.emit('CURRENT','NONE',fs.b)
            self.assertEqual(pub.fd,-1);self.assertFalse(fs.nodes);self.assertFalse(fs.locks)
    def test_old_finally_cannot_invalidate_new_generation(self):
        fs=PublicationMemory()
        with fs.patches():
            old=w.CurrentPublication(fs.b,11);old.emit('CURRENT','NONE',fs.b)
            new=w.CurrentPublication(fs.b,11);new.emit('INVALID','STARTING',None);new.emit('CURRENT','NONE',fs.b)
            wire=fs.nodes[w.CURRENT_LEAF]['data'];fd=new.fd
            with self.assertRaisesRegex(w.Rejected,'REPLACED'):old.emit('INVALID','STOPPED',fs.b)
            self.assertEqual(fs.nodes[w.CURRENT_LEAF]['data'],wire);self.assertIn(fd,fs.locks);self.assertEqual(old.fd,-1);new.close()
    def test_global_lock_replacement_and_metadata_tamper_fail_closed(self):
        for fault in ('lock','hardlink','acl','mode'):
            fs=PublicationMemory()
            with fs.patches():
                pub=w.CurrentPublication(fs.b,11);pub.emit('CURRENT','NONE',fs.b)
                if fault=='lock':fs.named_lock=12
                elif fault=='hardlink':fs.nodes[w.CURRENT_LEAF]['nlink']=2
                elif fault=='mode':fs.nodes[w.CURRENT_LEAF]['mode']=0o640
                with mock.patch.object(w.os,'listxattr',return_value=['system.posix_acl_access'] if fault=='acl' else []),self.assertRaises(w.Rejected):pub.emit('INVALID','STOPPED',fs.b)
                self.assertFalse(fs.locks);self.assertEqual(pub.fd,-1)
    def test_signed_sequence_overflow_never_wraps(self):
        fs=PublicationMemory()
        with fs.patches():
            pub=w.CurrentPublication(fs.b,11);pub.emit('CURRENT','NONE',fs.b);pub.sequence=2**63-2
            with self.assertRaises(w.Rejected):pub.emit('CURRENT','NONE',fs.b)
            self.assertEqual(pub.fd,-1);self.assertFalse(fs.locks)
            value=pub.emit('INVALID','UNKNOWN',fs.b);self.assertEqual(value['sequence'],2**63-1)
            with self.assertRaises(w.Rejected):pub.emit('INVALID','UNKNOWN',fs.b)

class CurrentAuthorityTests(unittest.TestCase):
    def test_starting_precedes_complete_validation_and_idle_refresh(self):
        fs=PublicationMemory();calls=[]
        def binding(path):calls.append(fs.leaf()['status']);return binding_projection_fixture(fs.b)
        with fs.patches(),mock.patch.object(w,'_binding_with_helper_projection',side_effect=binding):
            authority=fs.authority();authority.start()
            self.assertEqual(calls[0],'INVALID');self.assertEqual(fs.leaf()['status'],'CURRENT')
            generation=fs.leaf()['generation']
            for i in range(4):fs.advance(15);authority.poll()
            self.assertEqual(fs.leaf()['sequence'],5);self.assertEqual(fs.leaf()['generation'],generation)
            self.assertGreaterEqual(len(calls),6);authority.revoke();self.assertFalse(fs.locks)
    def test_future_session_guard_keeps_guarded_current_live(self):
        fs=PublicationMemory();b=fs.b;contents={b['production_env_file']:b'synthetic env',b['calendar_file']:b'synthetic calendar',b['release_manifest_file']:b'synthetic manifest'}
        b['production_env_sha256']=w.digest(contents[b['production_env_file']]);b['cohort']['calendar_sha256']=w.digest(contents[b['calendar_file']]);b['cohort']['release_manifest_sha256']=w.digest(contents[b['release_manifest_file']]);b['cohort']['semantic_config_sha256']=w.semantic_hash(b)
        scheduler=w.Scheduler.__new__(w.Scheduler);scheduler.b=b;scheduler.pin=w.canonical(b);scheduler.path='/synthetic/private/binding';contents[scheduler.path]=scheduler.pin
        scheduler.j=mock.Mock(common={'arm':{'boot_id':fs.value['boot_id']}});scheduler.cancel=mock.Mock(fd=7);scheduler.previous_clock=dict(fs.clock);scheduler.initial_clock=dict(fs.clock)
        with fs.patches(),mock.patch.object(w,'_binding_with_helper_projection',return_value=(b,scheduler.pin,helper_projection_fixture(b))) as binding,mock.patch.object(w,'read_file',side_effect=lambda path,*a,**k:contents[str(path)]),mock.patch.object(w.select,'select',return_value=([],[],[])):
            authority=fs.authority();authority.start();scheduler.authority=authority
            for _ in range(244):fs.advance(.25);scheduler.guard()
            self.assertEqual(fs.leaf()['sequence'],5);self.assertGreaterEqual(binding.call_count,246)
            self.assertEqual(fs.leaf()['observed']['monotonic_ns'],fs.clock['monotonic_ns']-10**9)
            scheduler.cancel.consume.assert_not_called();authority.revoke();self.assertFalse(fs.locks)
    def test_startup_invalid_binding_never_mints_current(self):
        fs=PublicationMemory()
        with fs.patches(),mock.patch.object(w,'_binding_with_helper_projection',side_effect=w.Rejected('CONFIG_CHANGED')):
            authority=fs.authority()
            with self.assertRaises(w.Rejected):authority.start()
            self.assertEqual(fs.leaf()['status'],'INVALID');self.assertIsNone(fs.leaf()['binding']);self.assertEqual(fs.leaf()['reason'],'CONFIG_CHANGED');self.assertFalse(fs.locks)
    def test_current_asset_clock_boot_and_refresh_failure_closes_authority(self):
        for fault in ('assets','bytes','boot','clock','overdue','write'):
            with self.subTest(fault=fault):
                fs=PublicationMemory()
                with fs.patches(),mock.patch.object(w,'_binding_with_helper_projection',return_value=binding_projection_fixture(fs.b)) as binding:
                    authority=fs.authority();authority.start();fs.advance(15)
                    if fault=='assets':binding.side_effect=w.Rejected('CONFIG_CHANGED')
                    elif fault=='bytes':binding.return_value=(fs.b,b'changed',helper_projection_fixture(fs.b))
                    elif fault=='boot':fs.value['boot_id']='33333333-3333-4333-8333-333333333333'
                    elif fault=='clock':fs.clock['realtime_ns']+=2*10**9
                    elif fault=='overdue':fs.advance(16)
                    else:fs.fail='replace'
                    with self.assertRaises((w.Rejected,OSError)):authority.poll()
                    self.assertTrue(authority.stopped);self.assertEqual(authority.publication.fd,-1);self.assertFalse(fs.locks)
                    with self.assertRaises(w.Rejected):authority.poll()
    def test_invalidation_failure_does_not_keep_current_live(self):
        fs=PublicationMemory()
        with fs.patches(),mock.patch.object(w,'_binding_with_helper_projection',return_value=binding_projection_fixture(fs.b)):
            authority=fs.authority();authority.start();fs.fail='write'
            with self.assertRaises(OSError):authority.revoke('STOPPED')
            self.assertEqual(fs.leaf()['status'],'CURRENT');self.assertEqual(authority.publication.fd,-1);self.assertFalse(fs.locks)
            authority.revoke('STOPPED')  # no old finally overwrite/retry
    def test_long_command_refreshes_with_asset_guards_without_fd_inheritance(self):
        fs=PublicationMemory();now=[0.0];child=mock.Mock();child.stdout.fileno.return_value=80;child.stderr.fileno.return_value=81
        owned=mock.Mock(child=child);owned.settled.side_effect=lambda:now[0]>=75;owned.wait.return_value=0
        def selected(read,*args):now[0]+=.25;fs.advance(.25);return (read if now[0]>=75 else [],[],[])
        with fs.patches(),mock.patch.object(w,'_binding_with_helper_projection',return_value=binding_projection_fixture(fs.b)) as binding:
            authority=fs.authority();authority.start()
            original_read=fs.read
            with mock.patch.object(w.subprocess,'Popen',return_value=child) as popen,mock.patch.object(w,'OwnedChild',return_value=owned),mock.patch.object(w.os,'set_blocking'),mock.patch.object(w.os,'read',side_effect=lambda fd,n:b'' if fd in (80,81) else original_read(fd,n)),mock.patch.object(w.select,'select',side_effect=selected),mock.patch.object(w.time,'monotonic',side_effect=lambda:now[0]):
                result=w.guarded_command(['synthetic-helper'],180,authority.poll,lambda error:authority.revoke(w.error_tag(error)))
            self.assertEqual(result.returncode,0);self.assertGreaterEqual(binding.call_count,7)
            self.assertEqual(fs.leaf()['sequence'],6);self.assertTrue(popen.call_args.kwargs['close_fds']);self.assertEqual(popen.call_args.kwargs['pass_fds'],())
            self.assertNotIn(authority.publication.fd,popen.call_args.kwargs['pass_fds']);owned.cleanup.assert_not_called();owned.close.assert_called_once()
            authority.revoke();self.assertFalse(fs.locks)

class CurrentCommandTests(unittest.TestCase):
    def test_long_wait_guard_failure_closes_authority_before_owned_cleanup(self):
        for fault in ('asset','deadline','output','cleanup'):
            with self.subTest(fault=fault):
                fs=PublicationMemory();now=[0.0];child=mock.Mock();child.stdout.fileno.return_value=80;child.stderr.fileno.return_value=81
                owned=mock.Mock(child=child);owned.settled.return_value=False;owned.wait.return_value=0;cleanup=[]
                def selected(read,*args):
                    step=.001 if fault=='output' else .25;now[0]+=step;fs.advance(step)
                    return ([80] if fault=='output' else [],[],[])
                def binding(path):
                    if fault in ('asset','cleanup') and now[0]>=15:raise w.Rejected('CONFIG_CHANGED')
                    return binding_projection_fixture(fs.b)
                def clean():
                    self.assertFalse(fs.locks);self.assertTrue(authority.stopped);cleanup.append('owned')
                    if fault=='cleanup':raise w.Rejected('UNKNOWN')
                owned.cleanup.side_effect=clean
                with fs.patches(),mock.patch.object(w,'_binding_with_helper_projection',side_effect=binding):
                    authority=fs.authority();authority.start();original_read=fs.read
                    with mock.patch.object(w.subprocess,'Popen',return_value=child) as popen,mock.patch.object(w,'OwnedChild',return_value=owned),mock.patch.object(w.os,'set_blocking'),mock.patch.object(w.os,'read',side_effect=lambda fd,n:b'x'*n if fd==80 else original_read(fd,n)),mock.patch.object(w.select,'select',side_effect=selected),mock.patch.object(w.time,'monotonic',side_effect=lambda:now[0]):
                        with self.assertRaises(w.Rejected):w.guarded_command(['synthetic-helper'],2 if fault=='deadline' else 180,authority.poll,authority.failed)
                    self.assertEqual(cleanup,['owned']);self.assertEqual(authority.publication.fd,-1);self.assertFalse(fs.locks)
                    self.assertLessEqual(now[0],15.5);popen.assert_called_once();owned.close.assert_called_once();child.stdout.close.assert_called_once();child.stderr.close.assert_called_once()
    def test_original_guard_error_survives_invalidation_write_error(self):
        fs=PublicationMemory()
        with fs.patches(),mock.patch.object(w,'_binding_with_helper_projection',return_value=binding_projection_fixture(fs.b)):
            authority=fs.authority();authority.start();fs.advance(15);fs.clock['realtime_ns']+=2*10**9;fs.fail='write'
            with self.assertRaisesRegex(w.Rejected,'CLOCK_INVALID'):authority.poll()
            self.assertEqual(authority.fatal,'CLOCK_INVALID');self.assertEqual(authority.publication.fd,-1);self.assertFalse(fs.locks)
            authority.scheduler.cleanup_errors.append.assert_called_once_with('current:OSError')
            self.assertEqual(fs.leaf()['status'],'CURRENT')  # stale bytes have no fence

class CurrentAssetTests(unittest.TestCase):
    def test_every_referenced_asset_and_selector_is_revalidated(self):
        for fault in (None,'env','calendar','manifest','daily','research_image','helper_image','config_hash','owner','raw_uid'):
            with self.subTest(fault=fault):
                b=current_fixture()['binding'];files={b['production_env_file']:b'synthetic env',b['calendar_file']:b'synthetic calendar',b['release_manifest_file']:b'synthetic manifest',b['daily_config_file']:b'synthetic daily'}
                b['production_env_sha256']=w.digest(files[b['production_env_file']]);b['cohort']['calendar_sha256']=w.digest(files[b['calendar_file']]);b['cohort']['release_manifest_sha256']=w.digest(files[b['release_manifest_file']]);b['daily_config_sha256']=w.digest(files[b['daily_config_file']]);b['cohort']['semantic_config_sha256']=w.semantic_hash(b)
                config=dict(owner=b['cohort']['owner_user_id'],raw_owner_uid=b['raw_owner_uid']);images={'research-worker':b['cohort']['research_image_id'],'owner-beta-runner':b['cohort']['helper_image_id']};config_hash=b['daily_config_sha256']
                keys={'env':'production_env_file','calendar':'calendar_file','manifest':'release_manifest_file','daily':'daily_config_file'}
                if fault in keys:files[b[keys[fault]]]+=b' changed'
                elif fault=='research_image':images['research-worker']='sha256:'+'0'*64
                elif fault=='helper_image':images['owner-beta-runner']='sha256:'+'0'*64
                elif fault=='config_hash':config_hash='sha256:'+'0'*64
                elif fault=='owner':config['owner']='33333333-3333-4333-8333-333333333333'
                elif fault=='raw_uid':config['raw_owner_uid']=10003
                operations=mock.Mock();operations.read_manifest.return_value=images;operations.install.verify_config.return_value=(config,config_hash)
                spec=mock.Mock(name='paper_daily_operations')
                with mock.patch.object(w,'structural_binding',return_value=(b,w.canonical(b))),mock.patch.object(w,'read_file',side_effect=lambda path,*a,**k:files[str(path)]) as read,mock.patch.object(w.importlib.util,'spec_from_file_location',return_value=spec),mock.patch.object(w.importlib.util,'module_from_spec',return_value=operations):
                    if fault is None:
                        self.assertEqual(w.binding('/private/binding'),(b,w.canonical(b)));self.assertEqual(read.call_count,4)
                        self.assertEqual(read.call_args_list[0].kwargs['exact'],(0,0,0o600))
                    else:
                        with self.assertRaises(w.Rejected):w.binding('/private/binding')
    def test_constructor_only_discovers_structure_never_authority_before_lock(self):
        b=current_fixture()['binding']
        with mock.patch.object(w.os,'geteuid',return_value=0),mock.patch.object(w,'structural_binding',return_value=(b,w.canonical(b))) as structure,mock.patch.object(w,'binding',side_effect=AssertionError('early full validation')),mock.patch.object(w,'CurrentPublication',side_effect=AssertionError('early publication')),mock.patch.object(w,'load_runtime',return_value=mock.Mock()):
            scheduler=w.Scheduler('/synthetic/private/binding')
        structure.assert_called_once();self.assertIsNone(scheduler.authority)

class CurrentLifecycleTests(unittest.TestCase):
    def test_global_lock_covers_start_and_final_invalidation(self):
        b=current_fixture()['binding'];scheduler=w.Scheduler.__new__(w.Scheduler);scheduler.b=b;scheduler.docker=mock.Mock();old_call=scheduler.docker.call
        held=[];events=[]
        @contextlib.contextmanager
        def opened(*a,**k):
            held.append(11)
            try:yield 11
            finally:self.assertEqual(events[-1],'invalidate');held.pop()
        authority=mock.Mock()
        def started():self.assertEqual(held,[11]);events.append('start')
        def revoked(reason):self.assertEqual(held,[11]);self.assertEqual(reason,'STOPPED');events.append('invalidate')
        authority.start.side_effect=started;authority.revoke.side_effect=lambda reason:None if events[-1:] == ['invalidate'] else revoked(reason)
        scheduler.command=mock.Mock(side_effect=w.Rejected('INTERRUPTED'))
        with mock.patch.object(w,'opened',opened),mock.patch.object(w.fcntl,'flock') as lock,mock.patch.object(w,'CurrentAuthority',return_value=authority),mock.patch.object(w.signal,'signal'),mock.patch.object(w,'due_ns',return_value=2**62),self.assertRaisesRegex(w.Rejected,'INTERRUPTED'):scheduler.run()
        lock.assert_called_once_with(11,w.fcntl.LOCK_EX|w.fcntl.LOCK_NB);self.assertFalse(held);self.assertIs(scheduler.docker.call,old_call)
        self.assertEqual(events,['start','invalidate']);scheduler.command.assert_called_once()
    def test_source_and_relay_pass_only_private_socket_never_authority_fd(self):
        with tempfile.TemporaryDirectory() as folder:
            rig=SyntheticSource(folder);spawn=rig.popen;calls=[]
            def popen(argv,**kwargs):calls.append((argv,kwargs));return spawn(argv,**kwargs)
            rig.popen=popen;chain=rig.run();w.verify_chain(chain['events'],'LINKED')
            self.assertEqual(len(calls),4)
            self.assertTrue(all(kwargs['close_fds'] is True for _,kwargs in calls))
            self.assertEqual(len(calls[0][1]['pass_fds']),1)
            self.assertEqual(str(calls[0][1]['pass_fds'][0]),calls[0][1]['env']['LAGRANGE_PAPER_WITNESS_FD'])
            self.assertTrue(all(kwargs['pass_fds']==() for _,kwargs in calls[1:]))
    def test_failure_revokes_before_helper_cleanup_and_never_relaunches(self):
        with tempfile.TemporaryDirectory() as folder:
            rig=SyntheticSource(folder);events=[]
            with rig.patches():
                scheduler=rig.scheduler;scheduler.authority.revoke.side_effect=lambda reason:events.append(('revoke',reason))
                cleanup=scheduler.cleanup_container
                def clean(lease,scope):
                    self.assertTrue(events and events[0][0]=='revoke');self.assertTrue(scheduler.in_cleanup);events.append(('cleanup',scope));return cleanup(lease,scope)
                scheduler.cleanup_container=clean
                call=rig.call
                def docker(args,**kwargs):
                    if args[0]=='run':call(args,**kwargs);raise w.Rejected('DB_UNAVAILABLE')
                    return call(args,**kwargs)
                with mock.patch.object(rig,'call',side_effect=docker),self.assertRaises(w.Rejected):scheduler.run_session(rig.session)
                self.assertEqual(scheduler.j.events[-1]['kind'],'FAILED');self.assertEqual(rig.launches,0)
                self.assertEqual(events[0],('revoke','DB_UNAVAILABLE'));self.assertFalse(rig.objects)
            with self.assertRaises((w.Rejected,OSError)):rig.run()
            self.assertEqual(rig.launches,0)
    def test_inactive_install_prepares_invalid_inside_both_locks_only(self):
        b=current_fixture()['binding'];args=w.argparse.Namespace(binding='/synthetic/private/binding',release_root=b['release_root'],commit=b['cohort']['verifier_revision'],env_file=b['production_env_file'],calendar_dir=b['calendar_dir'],state_dir=b['state_dir'],lock_file=b['lock_file'],systemd_dir='/synthetic/systemd',mode='install-disabled')
        held=[];events=[];commands=[];publication=mock.Mock()
        @contextlib.contextmanager
        def opened(path,**kwargs):
            path=str(path);held.append(path);yield 11 if path.endswith('/.control/lock') else 12;held.pop()
        def emit(status,reason,selected):
            self.assertIn(b['lock_file'],held);self.assertIn(b['evidence_root']+'/.control/lock',held)
            self.assertEqual((status,reason,selected),('INVALID','STOPPED',b));events.append('INVALID')
        publication.emit.side_effect=emit
        def command(argv,**kwargs):
            commands.append(argv);return mock.Mock(returncode=0,stdout=b'disabled' if 'UnitFileState' in ' '.join(argv) else b'inactive' if 'show' in argv else b'')
        stream=mock.MagicMock();stream.__enter__.return_value=stream;stream.fileno.return_value=13
        with contextlib.ExitStack() as stack:
            for obj,name,value in [(w,'binding',lambda path:(b,w.canonical(b))),(w.os,'geteuid',lambda:0),(w,'opened',opened),
                (Path,'exists',lambda path:not(str(path).endswith('.service') or str(path).endswith('/current'))),(Path,'is_symlink',lambda path:False),
                (w.os,'mkdir',mock.Mock()),(w.os,'chown',mock.Mock()),(w.os,'chmod',mock.Mock()),(w.os,'fsync',mock.Mock()),(w.os,'open',mock.Mock(return_value=13)),
                (w.os,'fdopen',lambda *a,**k:stream),(w.os,'fchmod',mock.Mock()),(w.fcntl,'flock',mock.Mock()),(w,'CurrentPublication',mock.Mock(return_value=publication)),(w.subprocess,'run',command)]:stack.enter_context(mock.patch.object(obj,name,value))
            with contextlib.redirect_stdout(io.StringIO()):self.assertEqual(w.install_witness(args),0)
        self.assertEqual(events,['INVALID']);publication.close.assert_called_once();self.assertFalse(held)
        self.assertEqual([a[1] for a in commands],['daemon-reload','show','show'])
        self.assertIn('KillMode=mixed',w.render_service(b,args.binding,args.state_dir,args.lock_file))

class LaunchRollbackTests(unittest.TestCase):
    def setUp(self):
        self.pending=list(w.UNRESOLVED_CHILDREN)
    def tearDown(self):
        w.UNRESOLVED_CHILDREN[:]=self.pending
    def child(self):
        child=mock.Mock(pid=123,returncode=None);child._waitpid_lock=threading.Lock()
        return child
    def procstat(self,parent=None,start=456,pgrp=123,session=123):
        return '123 (synthetic child) '+' '.join(['S',str(os.getpid() if parent is None else parent),str(pgrp),str(session)]+['0']*15+[str(start)])
    @contextlib.contextmanager
    def kernel(self,child,events,*,fault=errno.ENOSYS,proc_failure=False,stat_text=None,signal_error=False):
        dead=[False]
        def pidfd(pid,flags):events.append('pidfd');raise OSError(fault,'synthetic setup error')
        def observe(pid):
            if proc_failure:raise OSError(errno.EACCES,'synthetic proc error')
            return dict(pid=pid,start_ticks=456)
        def send(pid,sig):
            self.assertEqual(pid,123);self.assertTrue(child._waitpid_lock.locked());events.append('signal')
            if signal_error:raise PermissionError('synthetic signal refusal')
            dead[0]=True
        def wait(**kwargs):
            self.assertFalse(child._waitpid_lock.locked());self.assertTrue(dead[0]);events.append('reap')
            child.returncode=-15;return -15
        child.wait.side_effect=wait
        with mock.patch.object(w.subprocess,'Popen',return_value=child) as popen, mock.patch.object(w,'process',side_effect=observe),mock.patch.object(w.os,'pidfd_open',side_effect=pidfd),mock.patch.object(w,'unreaped_child_policy'),mock.patch.object(Path,'read_text',return_value=stat_text or self.procstat()),mock.patch.object(w.os,'waitid',side_effect=lambda *args:types.SimpleNamespace(si_pid=123) if dead[0] else None) as waitid,mock.patch.object(w.os,'kill',side_effect=send) as kill,mock.patch.object(w.os,'killpg') as group_signal,mock.patch.object(w.signal,'pidfd_send_signal') as pidfd_signal:
            yield popen,waitid,kill,group_signal,pidfd_signal
    def test_pidfd_enosys_enfile_revoke_before_direct_rollback_and_reap(self):
        for fault in (errno.ENOSYS,errno.ENFILE):
            with self.subTest(errno=fault):
                child=self.child();events=[]
                with self.kernel(child,events,fault=fault) as calls,self.assertRaises(OSError) as error:
                    w.guarded_command(['never-respawn'],1,lambda:None,lambda error:events.append('revoke'))
                self.assertEqual(error.exception.errno,fault);self.assertEqual(events,['pidfd','revoke','signal','reap'])
                self.assertIs(error.exception.child_custody.child,child);self.assertTrue(error.exception.child_custody.reaped)
                self.assertFalse(error.exception.child_custody.acquired);calls[0].assert_called_once();calls[3].assert_not_called();calls[4].assert_not_called()
                self.assertTrue(all(c.args==(w.os.P_PID,123,w.os.WEXITED|w.os.WNOHANG|w.os.WNOWAIT) for c in calls[1].call_args_list))
                child.poll.assert_not_called();child.stdout.close.assert_called_once();child.stderr.close.assert_called_once()
    def test_first_process_observation_failure_keeps_custody_without_pidfd(self):
        child=self.child();events=[]
        with self.kernel(child,events,proc_failure=True),self.assertRaises(OSError) as error:
            w.guarded_command(['not-logged'],1,lambda:None,lambda error:events.append('revoke'))
        self.assertEqual(events,['revoke','signal','reap']);self.assertEqual(error.exception.errno,errno.EACCES)
        self.assertEqual(error.exception.child_custody.diagnostic()['start_ticks'],456)
    def test_wrong_parent_or_start_never_signals_and_retains_unknown_custody(self):
        for stat_text in (self.procstat(parent=999999),self.procstat(start=789)):
            child=self.child();events=[]
            with self.kernel(child,events,stat_text=stat_text) as calls,self.assertRaisesRegex(w.Rejected,'UNKNOWN') as error:
                w.guarded_command(['SECRET_SENTINEL'],1,lambda:None,lambda error:events.append('revoke'))
            calls[2].assert_not_called();calls[3].assert_not_called();child.wait.assert_not_called()
            owned=error.exception.child_custody;self.assertIs(owned.child,child);self.assertIn(owned,w.UNRESOLVED_CHILDREN)
            self.assertEqual(error.exception.original_errno,errno.ENOSYS);self.assertEqual(error.exception.original_failure,'OSError')
            self.assertNotIn('SECRET_SENTINEL',json.dumps(owned.diagnostic()));self.assertFalse(owned.reaped)
    def test_signal_failure_and_invalidation_failure_do_not_hide_first_failure(self):
        for invalidate_error in (False,True):
            child=self.child();events=[]
            def revoke(error):
                events.append('revoke')
                if invalidate_error:raise OSError('synthetic invalidate')
            with self.kernel(child,events,signal_error=True),self.assertRaisesRegex(w.Rejected,'UNKNOWN') as error:
                w.guarded_command(['not-logged'],1,lambda:None,revoke)
            self.assertEqual(events,['pidfd','revoke','signal']);self.assertEqual(error.exception.original_errno,errno.ENOSYS)
            self.assertIs(error.exception.child_custody.child,child);self.assertTrue(error.exception.cleanup_errors)
            child.stdout.close.assert_called_once();child.stderr.close.assert_called_once();child.wait.assert_not_called()
    def test_echild_or_autoreap_policy_cannot_authorize_numeric_signal(self):
        for cause in ('echild','autoreap'):
            child=self.child();owned=w.OwnedChild(child,observe=False)
            with mock.patch.object(w,'unreaped_child_policy',side_effect=w.Rejected('ORIGIN_UNPROVEN') if cause=='autoreap' else None),mock.patch.object(w.os,'waitid',side_effect=ChildProcessError()),mock.patch.object(w.os,'kill') as kill,mock.patch.object(w.os,'killpg') as killpg,self.assertRaises((w.Rejected,ChildProcessError)):owned.cleanup()
            kill.assert_not_called();killpg.assert_not_called();child.poll.assert_not_called();child.wait.assert_not_called()
            self.assertFalse(child._waitpid_lock.locked())
    def test_group_fallback_keeps_zombie_leader_until_group_quiesces(self):
        child=self.child();owned=w.OwnedChild(child,group=True,observe=False);now=[0.0];alive=[True];events=[]
        def kill(pid,sig):
            self.assertEqual(pid,123);self.assertTrue(child._waitpid_lock.locked());events.append(sig)
            if sig==w.signal.SIGKILL:alive[0]=False
        def reap(**kwargs):self.assertFalse(alive[0]);self.assertFalse(child._waitpid_lock.locked());child.returncode=-9;return -9
        child.wait.side_effect=reap
        with mock.patch.object(w,'unreaped_child_policy'),mock.patch.object(w.os,'waitid',return_value=types.SimpleNamespace(si_pid=123)),mock.patch.object(Path,'read_text',return_value=self.procstat()),mock.patch.object(owned,'live_group',side_effect=lambda:alive[0]),mock.patch.object(w.os,'killpg',side_effect=kill),mock.patch.object(w.os,'kill') as single,mock.patch.object(w.time,'monotonic',side_effect=lambda:now[0]),mock.patch.object(w.time,'sleep',side_effect=lambda n:now.__setitem__(0,now[0]+n)):owned.cleanup()
        self.assertEqual(events,[w.signal.SIGTERM,w.signal.SIGKILL]);self.assertTrue(owned.reaped);self.assertLessEqual(now[0],10)
        single.assert_not_called();child.wait.assert_called_once();child.poll.assert_not_called()
    def test_wrong_group_session_refuses_fallback_signal(self):
        child=self.child();owned=w.OwnedChild(child,group=True,observe=False)
        with mock.patch.object(w,'unreaped_child_policy'),mock.patch.object(w.os,'waitid',return_value=None),mock.patch.object(Path,'read_text',return_value=self.procstat(session=999)),mock.patch.object(w.os,'killpg') as killpg,self.assertRaises(w.Rejected):owned.cleanup()
        killpg.assert_not_called();child.wait.assert_not_called()
    def test_fallback_term_and_kill_ignored_stops_at_ten_seconds_unreaped(self):
        child=self.child();owned=w.OwnedChild(child,observe=False);now=[0.0]
        with mock.patch.object(w,'unreaped_child_policy'),mock.patch.object(w.os,'waitid',return_value=None),mock.patch.object(Path,'read_text',return_value=self.procstat()),mock.patch.object(w.os,'kill') as kill,mock.patch.object(w.time,'monotonic',side_effect=lambda:now[0]),mock.patch.object(w.time,'sleep',side_effect=lambda n:now.__setitem__(0,now[0]+n)),self.assertRaisesRegex(w.Rejected,'UNKNOWN'):owned.cleanup()
        self.assertEqual([c.args for c in kill.call_args_list],[(123,w.signal.SIGTERM),(123,w.signal.SIGKILL)])
        self.assertLessEqual(now[0],10);self.assertFalse(owned.reaped);child.wait.assert_not_called();child.poll.assert_not_called()
    def test_sigchld_query_refuses_autoreap_custom_handler_and_other_threads(self):
        for flags,handler,threads in ((0,None,1),(2,None,1),(0,1,1),(0,None,2)):
            libc=mock.Mock()
            def query(sig,action,old):
                self.assertEqual(sig,w.signal.SIGCHLD);self.assertIsNone(action)
                old._obj.flags=flags;old._obj.handler=handler;return 0
            libc.sigaction.side_effect=query
            with mock.patch.object(w.os,'uname',return_value=types.SimpleNamespace(machine='x86_64')),mock.patch.object(w.os,'confstr',return_value='glibc 2.39'),mock.patch.object(Path,'iterdir',return_value=iter(range(threads))),mock.patch.object(w.signal,'getsignal',return_value=w.signal.SIG_DFL),mock.patch.object(w.ctypes,'CDLL',return_value=libc):
                if flags or handler or threads!=1:
                    with self.assertRaises(w.Rejected):w.unreaped_child_policy()
                else:w.unreaped_child_policy();libc.sigaction.assert_called_once()
    def test_relay_setup_failure_has_no_end_no_respawn_and_exact_custody(self):
        child=self.child();events=[]
        with self.kernel(child,events),mock.patch.object(w,'ipc_request',return_value={'name':'owned'}) as ipc,self.assertRaises(OSError) as error:
            w.relay_compose('capture',['docker','run','--rm'])
        self.assertEqual(ipc.call_count,1);self.assertEqual(events,['pidfd','signal','reap']);self.assertTrue(error.exception.child_custody.reaped)
    def test_wrapper_acquire_failure_preserves_failed_claim_before_cleanup(self):
        with tempfile.TemporaryDirectory() as folder:
            rig=SyntheticSource(folder);owners=[]
            with rig.patches():
                owner=w.OwnedChild
                def create(*args,**kwargs):
                    result=owner(*args,**kwargs);owners.append(result)
                    result.acquire=mock.Mock(side_effect=OSError(errno.ENFILE,'synthetic wrapper'))
                    original=result.cleanup
                    def cleanup():
                        self.assertEqual(rig.scheduler.j.events[-1]['kind'],'FAILED')
                        rig.scheduler.authority.revoke.assert_called();original()
                    result.cleanup=mock.Mock(side_effect=cleanup);return result
                with mock.patch.object(w,'OwnedChild',side_effect=create),self.assertRaisesRegex(w.Rejected,'UNKNOWN') as error:rig.scheduler.run_session(rig.session)
            self.assertEqual(rig.launches,1);self.assertEqual(rig.provider_phases,[]);self.assertEqual(error.exception.original_errno,errno.ENFILE)
            self.assertIs(rig.scheduler.owned_child,owners[0]);self.assertTrue(owners[0].reaped);owners[0].cleanup.assert_called_once()
            self.assertNotIn('RUNNING',[e['kind'] for e in rig.scheduler.j.events])
    def test_startup_unknown_has_private_handoff_without_journal(self):
        child=self.child();owned=w.OwnedChild(child,observe=False)
        error=w.child_failure(OSError(errno.ENOSYS,'never log this'),owned,['child:UNKNOWN'])
        scheduler=w.Scheduler.__new__(w.Scheduler);scheduler.j=None;scheduler.child=None;scheduler.owned_child=None
        scheduler.command_custodies=[];scheduler.cleanup_errors=[];scheduler.b={'evidence_root':'/synthetic/evidence'}
        with mock.patch.object(w,'guarded_command',side_effect=error),mock.patch.object(w,'publish') as publish,self.assertRaises(w.Rejected) as caught:scheduler.command(['NEVER_LOG_ARGV'],1)
        self.assertIs(caught.exception.child_custody.child,child);self.assertEqual(scheduler.command_custodies,[owned]);publish.assert_called_once()
        args=publish.call_args.args;self.assertEqual(args[0],Path('/synthetic/evidence/.control'));self.assertTrue(args[1].startswith('startup-cleanup-'));self.assertTrue(args[3])
        note=json.loads(args[2]);self.assertIsNone(note['invocation_id']);self.assertEqual(note['status'],'UNKNOWN')
        self.assertEqual(note['command_children'][0]['pid'],123);self.assertNotIn(b'NEVER_LOG_ARGV',args[2]);self.assertNotIn(b'never log this',args[2])
    def test_pidfd_partial_acquisition_closes_fd_once_and_retains_custody(self):
        child=self.child();owned=w.OwnedChild(child,observe=False)
        with mock.patch.object(w,'process',return_value=dict(pid=123,start_ticks=456)),mock.patch.object(w.os,'pidfd_open',return_value=9),mock.patch.object(owned,'check',side_effect=w.Rejected('ORIGIN_UNPROVEN')),mock.patch.object(w.os,'close') as close,self.assertRaises(w.Rejected) as error:owned.acquire()
        owned.close();close.assert_called_once_with(9);self.assertIs(error.exception.child_custody,owned);self.assertFalse(owned.acquired)
    def test_startup_handoff_write_failure_retains_exception_and_scheduler_handle(self):
        child=self.child();owned=w.OwnedChild(child,observe=False)
        error=w.child_failure(OSError(errno.ENFILE,'private-message'),owned,['child:UNKNOWN'])
        scheduler=w.Scheduler.__new__(w.Scheduler);scheduler.j=None;scheduler.child=None;scheduler.owned_child=None
        scheduler.command_custodies=[];scheduler.cleanup_errors=[];scheduler.b={'evidence_root':'/synthetic/evidence'}
        with mock.patch.object(w,'guarded_command',side_effect=error),mock.patch.object(w,'publish',side_effect=OSError('synthetic full disk')),self.assertRaises(w.Rejected) as caught:scheduler.command(['private-argv'],1)
        self.assertIs(caught.exception,error);self.assertIs(scheduler.command_custodies[0].child,child)
        self.assertEqual(error.handoff_error,'OSError');self.assertEqual(error.original_errno,errno.ENFILE)
    def test_relay_top_level_unknown_handoff_has_no_argv_or_environment(self):
        child=self.child();owned=w.OwnedChild(child,observe=False)
        error=w.child_failure(OSError(errno.ENOSYS,'private-message'),owned,['child:UNKNOWN']);out=io.StringIO()
        with mock.patch.object(w.sys,'argv',['witness','compose','capture','private-argv']),mock.patch.object(w,'relay_compose',side_effect=error),contextlib.redirect_stderr(out):self.assertEqual(w.main(),1)
        rows=out.getvalue().splitlines();self.assertEqual(rows[-1],'PAPER_SOURCE_WITNESS_REJECTED')
        note=json.loads(rows[0].removeprefix('PAPER_SOURCE_CHILD_UNKNOWN '));self.assertEqual(note['child']['pid'],123)
        self.assertNotIn('private-',out.getvalue());self.assertIs(error.child_custody.child,child)

class HelperProjectionTests(unittest.TestCase):
    def context_reader(self):
        # Only the actual read_context definition runs with memory dependencies.
        # Unrelated operations top-level loaders are not imported by these tests.
        path=ROOT/'scripts/ops/lib/owner-beta-daily-operations.py'
        tree=ast.parse(path.read_text())
        function=next(n for n in tree.body if isinstance(n,ast.FunctionDef) and n.name=='read_context')
        constants={n.targets[0].id:ast.literal_eval(n.value) for n in tree.body
            if isinstance(n,ast.Assign) and isinstance(n.targets[0],ast.Name)
            and n.targets[0].id in ('CONTEXT_KEYS','ENVIRONMENT')}
        runtime=w.load_runtime()
        ns=dict(constants,ROOT=str(ROOT),runtime=runtime,require=runtime.require,
            install=mock.Mock(),subprocess=mock.Mock(),hashlib=w.hashlib,re=w.re)
        exec(compile(ast.Module(body=[function],type_ignores=[]),str(path),'exec'),ns)
        return ns
    def context_values(self,b):
        return dict(LAGRANGE_CODE_COMMIT=b['cohort']['verifier_revision'],
            LAGRANGE_DATA_DIR=b['data_root'],LAGRANGE_ARTIFACTS_DIR='/synthetic/artifacts',
            LAGRANGE_RUNTIME_SECRET_DIR=b['secret_root'],POSTGRES_DB=b['database'])
    def operations(self,b):
        return types.SimpleNamespace(runtime=w.load_runtime(),
            read_context=mock.Mock(return_value=(self.context_values(b),[])))
    @contextlib.contextmanager
    def helper_case(self):
        with tempfile.TemporaryDirectory() as folder:
            rig=SyntheticSource(folder)
            with rig.patches():
                first=fixture()['events'][0];reserved=fixture()['events'][3]
                rig.scheduler.guard=mock.Mock()
                rig.scheduler.j=types.SimpleNamespace(slot=first['slot_id'],common=first,
                    events=[first,reserved],append=mock.Mock())
                rig.database('reserve')  # ordinary model readback, not a DB call
                yield rig
    def test_context_pin_rejects_before_child_and_legacy_tuple_is_unchanged(self):
        ns=self.context_reader();b=current_fixture()['binding']
        values=dict.fromkeys(ns['CONTEXT_KEYS'],'');values.update(self.context_values(b))
        raw=b'synthetic protected environment'
        output=''.join(k+'\0'+values[k]+'\0' for k in ns['CONTEXT_KEYS']).encode()
        read=ns['install'].read_root_file;run=ns['subprocess'].run
        read.return_value=raw;run.return_value=types.SimpleNamespace(returncode=0,stdout=output)
        for pin in (None,w.digest(raw)):
            read.reset_mock();run.reset_mock()
            self.assertEqual(ns['read_context']('/release',values['LAGRANGE_CODE_COMMIT'],'quiescent',
                expected_env_sha256=pin),(values,[]))
            self.assertEqual(read.call_count,2);run.assert_called_once()
            self.assertEqual(run.call_args.args[0][-3:],['/release',values['LAGRANGE_CODE_COMMIT'],'quiescent'])
            self.assertEqual(run.call_args.kwargs['env'],ns['ENVIRONMENT'])
            self.assertEqual(run.call_args.kwargs['timeout'],15)
        for pin in ('sha256:'+'0'*64,'sha256:ABC',True):
            read.reset_mock();run.reset_mock()
            with self.subTest(pin=pin),self.assertRaisesRegex(ns['runtime'].RuntimeErrorCode,'DAILY_OPERATION_CONTEXT'):
                ns['read_context']('/release',values['LAGRANGE_CODE_COMMIT'],'quiescent',expected_env_sha256=pin)
            run.assert_not_called()
        read.side_effect=[raw,raw+b' changed'];run.reset_mock()
        with self.assertRaisesRegex(ns['runtime'].RuntimeErrorCode,'DAILY_OPERATION_CONTEXT'):
            ns['read_context']('/release',values['LAGRANGE_CODE_COMMIT'],'quiescent',expected_env_sha256=w.digest(raw))
        run.assert_called_once()
        original=OSError('synthetic read failure');read.side_effect=original;run.reset_mock()
        with self.assertRaises(OSError) as caught:
            ns['read_context']('/release',values['LAGRANGE_CODE_COMMIT'],'quiescent',expected_env_sha256=w.digest(raw))
        self.assertIs(caught.exception,original);run.assert_not_called()
    def test_projection_comes_from_pinned_context_and_matches_binding(self):
        b=current_fixture()['binding'];raw=w.canonical(b);operations=self.operations(b)
        with mock.patch.object(w,'_binding_assets',return_value=(b,raw,operations)) as assets:
            selected,wire,projection=w._binding_with_helper_projection('/private/binding')
        assets.assert_called_once_with('/private/binding')
        operations.read_context.assert_called_once_with(b['release_root'],b['cohort']['verifier_revision'],
            'quiescent',expected_env_sha256=b['production_env_sha256'])
        self.assertEqual((selected,wire),(b,raw))
        self.assertEqual(projection.worker_historical_root,'/synthetic/artifacts/historical-price-beta-root')
        self.assertTrue(w._helper_projection_matches(projection,b,raw))
        self.assertNotIn('synthetic',repr(projection));self.assertNotIn(projection.binding_sha256,repr(projection))
        with self.assertRaises(AttributeError):projection.worker_historical_root='/changed'
        with self.assertRaises(AttributeError):del projection.binding_sha256
        with self.assertRaises(TypeError):json.dumps(projection)
        for key in ('LAGRANGE_CODE_COMMIT','LAGRANGE_DATA_DIR','LAGRANGE_RUNTIME_SECRET_DIR','POSTGRES_DB'):
            values=self.context_values(b);values[key]+='-changed';operations.read_context.return_value=(values,[])
            with mock.patch.object(w,'_binding_assets',return_value=(b,raw,operations)),self.subTest(key=key),self.assertRaisesRegex(w.Rejected,'CONFIG_CHANGED'):
                w._binding_with_helper_projection('/private/binding')
        operations.read_context.return_value=(self.context_values(b),['-f','/foreign'])
        with mock.patch.object(w,'_binding_assets',return_value=(b,raw,operations)),self.assertRaisesRegex(w.Rejected,'CONFIG_CHANGED'):
            w._binding_with_helper_projection('/private/binding')
        default=copy.deepcopy(b);default['database']='lagrange'
        values=self.context_values(default);values['POSTGRES_DB']='';operations.read_context.return_value=(values,[])
        with mock.patch.object(w,'_binding_assets',return_value=(default,w.canonical(default),operations)):
            self.assertEqual(w._binding_with_helper_projection('/private/binding')[:2],(default,w.canonical(default)))
    def test_all_roles_select_exact_historical_source(self):
        with self.helper_case() as rig:
            b=rig.b;root='/synthetic/artifacts/historical-price-beta-root';slot=rig.scheduler.j.slot
            for op in ('reserve','link-admission','read','link-run'):
                extra=('identity',) if op.startswith('link-') else ();admitter=op in ('reserve','link-admission')
                args=rig.runtime.paper_evidence_helper_args(b,op,slot,extra,'lagrange-paper-fixture',worker_historical_root=root)
                mounts={dict(v.split('=',1) for v in args[i+1].split(',') if '=' in v)['dst']:
                    dict(v.split('=',1) for v in args[i+1].split(',') if '=' in v)['src']
                    for i,v in enumerate(args) if v=='--mount'}
                self.assertEqual(mounts['/data/owner-beta-artifacts'],
                    b['data_root']+'/owner-beta-daily/historical-base/admitter' if admitter else root)
                self.assertEqual(mounts['/data/raw'],b['data_root']+'/raw');self.assertEqual(len(mounts),8)
                self.assertEqual(args[args.index('--user')+1],'10003:10003' if admitter else '10001:10001')
                self.assertIn('DB_USER='+('daily_input_admitter' if admitter else 'worker'),args)
                rig.call(args);value=copy.deepcopy(rig.objects['lagrange-paper-fixture'])
                rig.runtime.verify_paper_evidence_helper(value,b,op,slot,extra,'lagrange-paper-fixture',worker_historical_root=root)
                rig.objects.clear()
    def test_plain_binding_worker_refuses_and_private_ancestors_reject(self):
        b=current_fixture()['binding'];runtime=w.load_runtime();slot=fixture()['events'][0]['slot_id']
        for op in ('read','link-run'):
            with self.assertRaisesRegex(runtime.RuntimeErrorCode,'PAPER_HELPER_LAYOUT'):
                runtime.paper_evidence_helper_args(b,op,slot,('id',) if op=='link-run' else (),'lagrange-paper-fixture')
        fs=PublicationMemory()
        with fs.patches():
            scheduler=w.Scheduler.__new__(w.Scheduler);scheduler.authority=fs.authority()
            scheduler.guard=mock.Mock();scheduler.docker=mock.Mock()
            with self.assertRaisesRegex(w.Rejected,'CONFIG_CHANGED'):scheduler.helper('read')
            scheduler.docker.object.assert_not_called();scheduler.docker.call.assert_not_called()
        root='/synthetic/artifacts/historical-price-beta-root'
        for field in ('production_env_file','secret_root','binding'):
            for private in (root,root+'/private','/synthetic'):
                selected=copy.deepcopy(b);path='/private/binding'
                if field=='binding':path=private
                else:selected[field]=private
                operations=self.operations(selected)
                with mock.patch.object(w,'_binding_assets',return_value=(selected,w.canonical(selected),operations)),self.subTest(field=field,private=private),self.assertRaisesRegex(w.Rejected,'CONFIG_CHANGED'):
                    w._binding_with_helper_projection(path)
        for bad in ('','/','/unsafe,root','/unsafe\nroot'):
            operations=self.operations(b);values=self.context_values(b);values['LAGRANGE_ARTIFACTS_DIR']=bad
            operations.read_context.return_value=(values,[])
            with mock.patch.object(w,'_binding_assets',return_value=(b,w.canonical(b),operations)),self.assertRaisesRegex(w.Rejected,'CONFIG_CHANGED'):
                w._binding_with_helper_projection('/private/binding')
    def test_refresh_replaces_equal_snapshot_but_rejects_changed_projection(self):
        fs=PublicationMemory();root=['/synthetic/artifacts/historical-price-beta-root']
        def selected(_):
            b=copy.deepcopy(fs.b);return b,w.canonical(b),helper_projection_fixture(b,root[0])
        with fs.patches(),mock.patch.object(w,'_binding_with_helper_projection',side_effect=selected):
            authority=fs.authority();authority.start();first=authority.helper_projection();generation=fs.leaf()['generation']
            fs.advance(15);authority.poll();second=authority.helper_projection()
            self.assertIsNot(first,second);self.assertEqual(first,second);self.assertEqual(fs.leaf()['generation'],generation)
            root[0]='/changed/historical-price-beta-root';fs.advance(15)
            with self.assertRaisesRegex(w.Rejected,'CONFIG_CHANGED'):authority.poll()
            self.assertTrue(authority.stopped);self.assertIsNone(authority._helper_projection);self.assertFalse(fs.locks)
            with self.assertRaises(w.Rejected):authority.helper_projection()
        for fault in ('missing','hash','env','cohort','fatal','stopped'):
            fs=PublicationMemory()
            with fs.patches(),mock.patch.object(w,'_binding_with_helper_projection',side_effect=lambda _:binding_projection_fixture(fs.b)):
                authority=fs.authority();authority.start();p=authority.helper_projection()
                if fault=='missing':authority._helper_projection=None
                elif fault=='hash':authority._helper_projection=w._HelperProjection('sha256:'+'0'*64,p.production_env_sha256,p.cohort_sha256,p.worker_historical_root)
                elif fault=='env':authority._helper_projection=w._HelperProjection(p.binding_sha256,'sha256:'+'0'*64,p.cohort_sha256,p.worker_historical_root)
                elif fault=='cohort':authority._helper_projection=w._HelperProjection(p.binding_sha256,p.production_env_sha256,'sha256:'+'0'*64,p.worker_historical_root)
                elif fault=='fatal':authority.fatal='CONFIG_CHANGED'
                else:authority.stopped=True
                with self.subTest(fault=fault),self.assertRaises(w.Rejected):authority.helper_projection()
                if fault=='stopped':authority.stopped=False
                authority.revoke();self.assertIsNone(authority._helper_projection)
        fs=PublicationMemory()
        with fs.patches(),mock.patch.object(w,'_binding_with_helper_projection',side_effect=lambda _:binding_projection_fixture(fs.b)):
            authority=fs.authority();authority.start();fs.fail='write'
            with self.assertRaises(OSError):authority.revoke()
            self.assertIsNone(authority._helper_projection);self.assertFalse(fs.locks)
    def test_launch_and_inspection_capture_same_projection(self):
        with self.helper_case() as rig:
            scheduler=rig.scheduler;leases=[];observed=[];original=w.OwnedContainer;call=rig.call
            def lease(*args,**kwargs):
                value=original(*args,**kwargs);leases.append(value);return value
            def launch(args,**kwargs):
                result=call(args,**kwargs)
                if args[0]=='run':
                    observed.append(copy.deepcopy(next(iter(rig.objects.values()))))
                    scheduler.b=copy.deepcopy(scheduler.b);scheduler.b['database']='changed'
                    scheduler.authority.helper_projection.side_effect=AssertionError('late projection read')
                return result
            rig.call=launch
            with mock.patch.object(w,'OwnedContainer',side_effect=lease):scheduler.helper('read')
            self.assertEqual(len(leases),1);self.assertEqual(scheduler.authority.helper_projection.call_count,2)
            self.assertEqual(scheduler.guard.call_count,3);self.assertFalse(rig.objects)
            leases[0].verify(observed[0])
            for fault in ('admitter','foreign','writable','extra'):
                value=copy.deepcopy(observed[0]);mount=next(m for m in value['Mounts'] if m['Destination']=='/data/owner-beta-artifacts')
                if fault=='admitter':mount['Source']=rig.b['data_root']+'/owner-beta-daily/historical-base/admitter'
                elif fault=='foreign':mount['Source']='/foreign/historical-price-beta-root'
                elif fault=='writable':mount['RW']=True
                else:value['Mounts'].append(dict(Type='bind',RW=False,Source='/extra',Destination='/extra'))
                with self.subTest(fault=fault),self.assertRaises(rig.runtime.RuntimeErrorCode):leases[0].verify(value)
    def test_cleanup_uses_captured_projection_after_revocation(self):
        for secondary in (False,True):
            with self.subTest(secondary=secondary),self.helper_case() as rig:
                scheduler=rig.scheduler;call=rig.call;events=[];primary=w.Rejected('DB_UNAVAILABLE')
                scheduler.cleanup_note=mock.Mock()
                def revoke(reason):
                    events.append(('revoke',reason))
                    scheduler.authority.helper_projection.side_effect=AssertionError('cleanup projection read')
                scheduler.authority.revoke.side_effect=revoke
                def fail(args,**kwargs):
                    if args[0]=='rm':
                        self.assertEqual(events[0],('revoke','DB_UNAVAILABLE'))
                        if secondary:raise OSError('synthetic cleanup failure')
                    result=call(args,**kwargs)
                    if args[0]=='run':raise primary
                    return result
                rig.call=fail
                with mock.patch.object(w,'_binding_with_helper_projection',side_effect=AssertionError('cleanup context read')) as context:
                    with self.assertRaises(w.Rejected) as caught:scheduler.helper('read')
                context.assert_not_called();self.assertEqual(scheduler.authority.helper_projection.call_count,2)
                self.assertEqual(scheduler.failure_code,'DB_UNAVAILABLE');self.assertEqual(rig.helper_ops,['read'])
                if secondary:
                    self.assertEqual(str(caught.exception),'UNKNOWN');self.assertTrue(scheduler.cleanup_errors)
                    self.assertEqual(len(rig.objects),1)
                    self.assertTrue(any(c.args[1]=='UNKNOWN' for c in scheduler.cleanup_note.call_args_list))
                else:self.assertIs(caught.exception,primary);self.assertFalse(rig.objects)
    def test_wire_roundtrip_and_public_binding_contract_unchanged(self):
        fs=PublicationMemory();before=w.canonical(fs.b);semantic=w.semantic_hash(fs.b)
        with fs.patches(),mock.patch.object(w,'_binding_with_helper_projection',side_effect=lambda _:binding_projection_fixture(fs.b)):
            authority=fs.authority();authority.start();wire=w.canonical(fs.leaf());value=w.decode(wire)
            w.validate_current(value)
            self.assertEqual(w.canonical(value['binding']),before);self.assertEqual(w.semantic_hash(value['binding']),semantic)
            self.assertNotIn('worker_historical_root',wire.decode());self.assertNotIn('_HelperProjection',wire.decode())
            self.assertEqual(w.canonical(w.decode(before)),before);authority.revoke()
        operations=mock.Mock()
        with mock.patch.object(w,'_binding_assets',return_value=(fs.b,before,operations)) as assets:
            self.assertEqual(w.binding('/private/binding'),(fs.b,before))
        assets.assert_called_once_with('/private/binding')

if __name__=='__main__':
    if sys.argv[1:]==['--emit-synthetic-chain']:
        with tempfile.TemporaryDirectory() as folder:
            chain=SyntheticSource(folder).run();w.verify_chain(chain['events'],'LINKED')
            sys.stdout.buffer.write(w.canonical(chain))
    else:unittest.main()
