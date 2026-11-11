#!/usr/bin/env python3
"""Connected host lifecycle checks with native/Root/DB/process effects mocked."""
import contextlib
import copy
import fcntl
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import types
import unittest
from unittest import mock

sys.dont_write_bytecode = True
ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location('owner_beta_paper_lifecycle', ROOT/'scripts/ops/lib/owner-beta-paper-lifecycle.py')
m = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = m
spec.loader.exec_module(m)
ops, j = m.ops, m.journal
I = '11111111-1111-4111-8111-111111111111'
A = '22222222-2222-4222-8222-222222222222'
G = '33333333-3333-4333-8333-333333333333'
E = '44444444-4444-4444-8444-444444444444'
H = 'sha256:' + 'a'*64
C = 'b'*40

_PAPER_OPERATIONS_TEST_MODULE = "_owner_beta_paper_operations_fixture_tests"


def _paper_operations_test_module():
    module = sys.modules.get(_PAPER_OPERATIONS_TEST_MODULE)
    if module is None:
        path = Path(__file__).with_name("test_owner_beta_paper_operations.py")
        spec = importlib.util.spec_from_file_location(_PAPER_OPERATIONS_TEST_MODULE, path)
        if spec is None or spec.loader is None:
            raise ImportError("cannot load Paper operations test fixtures")
        module = importlib.util.module_from_spec(spec)
        sys.modules[_PAPER_OPERATIONS_TEST_MODULE] = module
        spec.loader.exec_module(module)
    return module


def reset():
    return dict(account_id=A,event_id=E,kind='CONFIGURED',control_epoch=0,occurred_at_us=100,payload_sha256=H)

def selection():
    return dict(schema='owner-beta-paper-reset-selection-v1',installation_id=I,reset=reset(),release_manifest_sha256=H,cohort_sha256=H)

def anchor():
    return dict(selection(),schema='owner-beta-paper-reset-anchor-v1',boot_id=G,
                observed=dict(realtime_ns=100001,monotonic_ns=200,boottime_ns=210))

def query(kind='RESET'):
    return ops._canonical(dict(schema='owner-beta-paper-exact-request-v1',kind=kind,account_id=A if kind=='RESET' else None,
                               args={'expected_epoch':0} if kind=='RESET' else {'request_key':'caller'}))

def readback(q, status='RESET', result=None):
    value=dict(schema='owner-beta-paper-exact-readback-v1',kind=json.loads(q)['kind'],request_sha256=ops._digest(q),status=status,
               account_id=A,current_epoch=0,current_state='OFF',event_id=None,event_epoch=None,result_canonical=None,result_sha256=None,
               reset=reset() if status=='RESET' else None)
    if status=='COMMITTED':
        raw=ops._canonical(result or dict(outcome='OK'))
        value.update(event_id=E,event_epoch=0,result_canonical=raw.decode(),result_sha256=ops._digest(raw))
    return ops._canonical(value)

class PureTests(unittest.TestCase):
    def test_deadline_is_absolute_int_never_object_or_extended(self):
        with mock.patch.object(m.time,'monotonic_ns',return_value=10):
            for value in (True,10,0,types.SimpleNamespace(value=100),10.0):
                with self.subTest(value=value),self.assertRaises(m.LifecycleError):m._Deadline(value)
            deadline=m._Deadline(2_000_000_010)
            self.assertEqual(deadline.remaining(15),2)
            self.assertEqual(deadline.remaining(0.5),0.5)
        with mock.patch.object(m.time,'monotonic_ns',return_value=2_000_000_010),self.assertRaises(m.LifecycleError):deadline.remaining()
    def test_reset_exact_query_needs_no_request_key_and_preserves_event_null(self):
        q=query();v=m._exact_readback(readback(q),q)
        self.assertEqual(v['reset'],reset());self.assertIsNone(v['event_id'])
        self.assertEqual(json.loads(q)['args'],{'expected_epoch':0})
        for key,value in [('request_sha256','sha256:'+'f'*64),('current_epoch',1),('event_id',E),('status','COMMITTED')]:
            changed=json.loads(readback(q));changed[key]=value
            with self.subTest(key=key),self.assertRaises(m.LifecycleError):m._exact_readback(ops._canonical(changed),q)
    def test_reset_boundary_original_and_clock_mutants(self):
        raw=ops._canonical(anchor());self.assertEqual(m._anchor(raw,selection())['boot_id'],G)
        for key,value in [('cohort_sha256','sha256:'+'f'*64),('installation_id',G),('observed',{'realtime_ns':100000,'monotonic_ns':200,'boottime_ns':210})]:
            changed=anchor();changed[key]=value
            with self.subTest(key=key),self.assertRaises(m.LifecycleError):m._anchor(ops._canonical(changed),selection())
    def test_single_action_roots_match_frozen_journal_without_begin(self):
        for purpose,key in [('CONFIGURE','caller'),('ENABLE',None),('RECOVERY',None)]:
            raw=b'{}';intent=m._intent(I,H,purpose,None if purpose=='CONFIGURE' else A,0,raw,generation=G,key=key)
            if purpose=='RECOVERY':intent['resume_target_id']=E
            j._validate_bound_root(intent,'caller')
            self.assertNotIn(':begin',intent['request_key'])
    def test_process_gate_absence_does_not_block_configure(self):
        owner=object.__new__(m.Lifecycle);owner._mutate=mock.Mock(return_value='native')
        raw=ops._canonical(dict(schema='owner-beta-paper-configure-operation-v1',strategy_config_id=G,account_name='Owner-beta Paper',contract={}))
        self.assertEqual(owner.configure('caller',raw),'native')
        owner._mutate.assert_called_once_with('caller','CONFIGURE',raw,None,0)
    def test_foreign_owner_cannot_invoke_lifecycle(self):
        with self.assertRaises(m.LifecycleError):m._assert_call(object(),object(),'configure',None)

class FakeStore:
    def __init__(self,context):
        self.pending_at_open=False;self.stage_root='/private/staging';self.originals=[];self.published=[];self.dirs={'anchors':501}
    def query(self,*args):return 400
    def retain(self,*args,**kwargs):pass
    def revalidate(self):pass
    def close(self):pass
    def publish(self,kind,raw):self.published.append((kind,raw))

class ConnectedTests(unittest.TestCase):
    def setUp(self):
        self.stack=contextlib.ExitStack();self.addCleanup(self.stack.close)
        for name in ('Popen','run'):
            self.stack.enter_context(mock.patch.object(subprocess,name,side_effect=AssertionError('subprocess forbidden')))
        self.dispatched=False;self.verbs=[];self.closed=[];self.native_outcome='OK';self.ack_loss=False;self.cleanup_fail=False
        self.native_result=dict(schema=ops.RESULT_SCHEMA,verb='configure',outcome='OK',event_id=E,generation=None,control_epoch=0,artifact_sha256=None,error_code='NONE')
        self.q=query('CONFIGURE')
        self.sql_result=dict(outcome='OK')
        rig=self
        self.action=types.SimpleNamespace(_closed=False,_action={'verb':'configure'},_artifact_bytes=b'{}',_binding_bytes=b'{}',
            borrow_output_fd=lambda:700,revalidate=lambda:None)
        self.custody=types.SimpleNamespace(_sql=self.q,_intent=b'{}',_artifact=b'{}',_action=self.action,_closed=False,
            _check=lambda:None,borrow_output_fd=lambda:700)
        self.custody.close=lambda:setattr(self.custody,'_closed',True)
        self.action.close=lambda:setattr(self.action,'_closed',True)
        self.action.import_dispatch=lambda:self.custody
        context=types.SimpleNamespace(_invocations=set(),manifest_sha256=H,install_root='/install',commit=C,
            revalidate=lambda:None,close=lambda:None,verified=types.SimpleNamespace(process_sha256=None))
        context._register_imported_custody=lambda *args:None
        context._invoke_lifecycle=lambda *args:ops._PaperContext._invoke_lifecycle(context,*args)
        self.context=context
        class Native:
            def __init__(self,token,ctx,verb,custody):
                self._context=ctx;self.verb=verb;self.custody=custody;self._closed=False
                self._preserve=False;self._imported=False;self._lifecycle_owner=None
            def __enter__(self):
                rig.verbs.append(self.verb)
                if self.verb=='configure':
                    rig.dispatched=True
                    if rig.ack_loss:raise ops.PaperOperationsError('PAPER_OPS_ATTACH_UNKNOWN')
                    result=dict(rig.native_result,outcome=rig.native_outcome)
                    self._result_bytes=rig.action._artifact_bytes
                elif self.verb=='readback':
                    result=dict(rig.native_result,schema=ops.QUERY_RESULT_SCHEMA,verb='readback',outcome='REPLAY')
                    self._result_bytes=readback(rig.q,'COMMITTED',rig.sql_result)
                else:raise AssertionError(self.verb)
                result['artifact_sha256']=ops._digest(self._result_bytes)
                self._stdout=json.dumps(result,separators=(',',':')).encode()+b'\n';self.result=result
                return self
            def import_dispatch(self):self._imported=True;return rig.custody
            def readback_artifact(self):self._imported=True;return self._result_bytes
            def close(self):
                rig.closed.append(self.verb)
                if rig.cleanup_fail and self.verb=='configure':raise m.LifecycleError('CLEANUP_UNKNOWN')
                self._closed=True;self._context._invocations.discard(self)
        self.stack.enter_context(mock.patch.object(ops,'_NativeInvocation',Native))
        self.stack.enter_context(mock.patch.object(ops,'_PaperContext',wraps=ops._PaperContext))
        # Constructor intercepted, private invocation method remains the real one.
        ops._PaperContext.side_effect=lambda *a,**k:context
        self.stack.enter_context(mock.patch.object(m,'_Store',FakeStore))
        self.stack.enter_context(mock.patch.object(j,'_immutable'))
        real_stat=os.stat
        def stat_(name,*args,**kwargs):
            if name=='dispatch.json' and kwargs.get('dir_fd')==700:
                if rig.dispatched:return types.SimpleNamespace()
                raise FileNotFoundError()
            return real_stat(name,*args,**kwargs)
        self.stack.enter_context(mock.patch.object(os,'stat',side_effect=stat_))
        self.owner=m.Lifecycle(types.SimpleNamespace(),17,m.time.monotonic_ns()+90_000_000_000)
        operation=types.SimpleNamespace(generation=G,intent_sha256=H,_intent_bytes=b'{}',close=lambda:None)
        def bind(key,raw):
            intent=j._validate_intent(raw);j._validate_bound_root(intent,key)
            operation._intent=intent;operation._intent_bytes=raw;operation.generation=intent['generation']
            operation.intent_sha256=ops._digest(raw)
            return operation
        def action(raw,evidence):
            value=j._validate_action(raw,operation._intent,0,None)
            self.action._action=value;self.action._artifact_bytes=evidence;self.action._operation=operation
            return self.action
        operation.prepare_action=action
        self.journal=types.SimpleNamespace(installation_id=I,epoch_floor=0,bind_caller=bind,revalidate=lambda:None,advance_floor=mock.Mock())
        self.owner._journal=lambda **kwargs:self.journal
        self.configure=ops._canonical(dict(schema='owner-beta-paper-configure-operation-v1',strategy_config_id=G,account_name='Owner-beta Paper',contract={}))
    def test_connected_configure_native_import_independent_readback_and_cleanup(self):
        result=self.owner.configure('caller',self.configure)
        self.assertEqual(self.verbs,['configure','readback']);self.assertEqual(self.closed,self.verbs)
        self.assertEqual(result.outcome,'OK');self.assertEqual(result.cleanup,'COMPLETE')
        self.assertEqual(json.loads(result.readback)['status'],'COMMITTED')
        self.assertEqual(self.action._action['sequence'],0);self.assertIsNone(self.action._action['stage_sha256'])
        self.owner.close()
    def test_existing_dispatch_reads_original_only(self):
        self.dispatched=True
        result=self.owner.configure('caller',self.configure)
        self.assertEqual(self.verbs,['readback']);self.assertEqual(result.outcome,'REPLAY')
    def test_ack_loss_imports_original_and_reads_without_resubmission(self):
        self.ack_loss=True
        result=self.owner.configure('caller',self.configure)
        self.assertEqual(self.verbs,['configure','readback']);self.assertEqual(result.outcome,'COMMIT_UNKNOWN')
        self.assertIsNone(result.native_result);self.assertEqual(json.loads(result.readback)['status'],'COMMITTED')
    def test_late_cleanup_cannot_be_upgraded_by_committed_readback(self):
        self.cleanup_fail=True
        result=self.owner.configure('caller',self.configure)
        self.assertEqual(result.outcome,'COMMIT_UNKNOWN');self.assertEqual(result.cleanup,'UNKNOWN')
        with self.assertRaises(m.LifecycleError):self.owner.close()
    def test_original_result_corruption_is_rejected(self):
        self.q=query('CONFIGURE')
        self.sql_result={'outcome':'BLOCKED'}
        with self.assertRaises(m.LifecycleError):self.owner.configure('caller',self.configure)
    def test_cli_context_native_import_and_only_after_exit_print(self):
        @contextlib.contextmanager
        def locked(*args):yield types.SimpleNamespace(),17
        @contextlib.contextmanager
        def opened(*args,**kwargs):
            yield self.owner
            self.owner.close();self.closed.append('owner')
        original=mock.Mock(raw=self.configure)
        with mock.patch.object(m,'_locked_context',locked),mock.patch.object(m,'open_lifecycle',opened),\
             mock.patch.object(m.daily.install,'is_commit',return_value=True),\
             mock.patch.object(m.daily.install,'root_directory',return_value=contextlib.nullcontext(99)),\
             mock.patch.object(m,'_Original',return_value=original),contextlib.redirect_stdout(io.StringIO()) as out:
            self.assertEqual(m.main(['configure','--install-root','/install','--caller-key','caller','--proposal','/private/proposal.json']),0)
        self.assertEqual(self.closed,['configure','readback','owner']);self.assertEqual(out.getvalue(),'PAPER_LIFECYCLE_OK:COMPLETE\n')
        original.close.assert_called_once()

class PublicationTests(unittest.TestCase):
    def owner(self,outcome='OK',cleanup='COMPLETE'):
        owner=object.__new__(m.Lifecycle);owner.store=FakeStore(None);owner.context=types.SimpleNamespace(manifest_sha256=H)
        owner._selection=lambda **kwargs:(selection(),{'account_id':A},b'{}',{'current_state':'OFF','current_epoch':0});owner._anchors=lambda _:()
        owner.revalidate=mock.Mock();owner.last=None;owner.pending_anchors=[]
        owner._run=mock.Mock(return_value=m.LifecycleResult(outcome,b'native',None,ops._canonical(anchor()),cleanup))
        return owner
    def test_anchor_only_published_after_native_ok_complete_cleanup(self):
        for outcome,cleanup,expected in [('OK','COMPLETE',1),('BLOCKED','COMPLETE',0),('COMMIT_UNKNOWN','COMPLETE',0),('OK','UNKNOWN',0)]:
            owner=self.owner(outcome,cleanup)
            with mock.patch.object(m,'_Call'),mock.patch.object(Path,'read_text',return_value=G):result=owner.observe_reset()
            self.assertEqual(len(owner.pending_anchors),expected);self.assertEqual(owner.store.published,[])
            self.assertEqual(result.outcome,outcome)
    def test_existing_boot_anchor_reused_without_late_observation(self):
        owner=self.owner();owner._anchors=lambda _:(ops._canonical(anchor()),)
        with mock.patch.object(Path,'read_text',return_value=G):result=owner.observe_reset()
        owner._run.assert_not_called();self.assertEqual(result.outcome,'REPLAY');self.assertEqual(owner.store.published,[])
    def test_enabled_current_epoch_one_reuses_reset_zero_anchor_for_next_source(self):
        owner=self.owner();owner._selection=lambda **kwargs:(selection(),{'account_id':A},b'{}',{'current_state':'ENABLED','current_epoch':1})
        owner._anchors=lambda _:(ops._canonical(anchor()),)
        with mock.patch.object(Path,'read_text',return_value=G):result=owner.observe_reset()
        owner._run.assert_not_called();self.assertEqual(result.outcome,'REPLAY')
        self.assertEqual(json.loads(result.artifact)['reset']['control_epoch'],0)
    def test_enabled_new_boot_without_original_requires_drain_reset(self):
        owner=self.owner();owner._selection=lambda **kwargs:(selection(),{'account_id':A},b'{}',{'current_state':'ENABLED','current_epoch':1})
        with mock.patch.object(Path,'read_text',return_value=G),self.assertRaisesRegex(m.LifecycleError,'LIFECYCLE_DRAIN_RESET_REQUIRED'):
            owner.observe_reset()
        owner._run.assert_not_called();self.assertEqual(owner.pending_anchors,[])
    def test_source_reset_query_uses_retained_actual_floor_and_does_not_normalize_it(self):
        owner=self.owner();owner._selection=m.Lifecycle._selection.__get__(owner)
        owner._process=lambda:({'control_epoch':1,'account_id':A,'daily_cohort_sha256':H},b'{}')
        owner._journal=lambda:types.SimpleNamespace(installation_id=I,epoch_floor=1)
        q=ops._canonical(dict(schema='owner-beta-paper-exact-request-v1',kind='RESET',account_id=A,args={'expected_epoch':1}))
        value=json.loads(readback(q));value.update(current_epoch=1,current_state='ENABLED')
        owner._run=mock.Mock(return_value=m.LifecycleResult('OK',b'n',ops._canonical(value),ops._canonical(value),'COMPLETE'))
        with mock.patch.object(m,'_Call') as call:
            selected,_,_,actual=owner._selection(for_enable=False)
        self.assertEqual(call.call_args.kwargs['sql'],q);self.assertEqual(selected['reset']['control_epoch'],0)
        self.assertEqual(actual['current_epoch'],1)
        with self.assertRaisesRegex(m.LifecycleError,'LIFECYCLE_FLOOR'):owner._selection(for_enable=True)

    def test_final_context_cleanup_rejects_previously_successful_result(self):
        owner=mock.Mock(last=m.LifecycleResult('OK',b'n',b'q',b'a','COMPLETE'))
        owner.close.side_effect=m.LifecycleError('late')
        with mock.patch.object(m,'Lifecycle',return_value=owner),self.assertRaises(m.LifecycleError) as caught:
            with m.open_lifecycle(object(),3,deadline=12):pass
        self.assertEqual(caught.exception.code,'CLEANUP_UNKNOWN');self.assertEqual(caught.exception.result.outcome,'COMMIT_UNKNOWN')
    def test_enabled_replay_never_reopens_current_or_reset(self):
        owner=object.__new__(m.Lifecycle);owner._prior_enable=mock.Mock(return_value='original-readback');owner._selection=mock.Mock(side_effect=AssertionError('eligibility'))
        self.assertEqual(owner.enable('same-K'),'original-readback');owner._selection.assert_not_called()

class UndispatchedEnableTests(unittest.TestCase):
    def test_prepared_enable_retry_reopens_original_action_without_busy_or_new_epoch(self):
        fixtures = _paper_operations_test_module()
        fixture_journal_dir_info = fixtures.fixture_journal_dir_info
        fixture_journal_file_info = fixtures.fixture_journal_file_info
        raw=b'{}'
        for selected_epoch in (0,1):
            with self.subTest(selected_epoch=selected_epoch),tempfile.TemporaryDirectory() as root,contextlib.ExitStack() as stack:
                install=Path(root)/'install';install.mkdir(mode=0o700)
                lock=install/j.LOCK_NAME;lock.touch(mode=0o600)
                fd=os.open(lock,os.O_RDWR|os.O_CLOEXEC);stack.callback(os.close,fd)
                fcntl.flock(fd,fcntl.LOCK_EX|fcntl.LOCK_NB)
                stack.enter_context(mock.patch.object(j,'_require_root',lambda:None))
                stack.enter_context(mock.patch.object(j,'_dir_info',fixture_journal_dir_info))
                stack.enter_context(mock.patch.object(j,'_file_info',fixture_journal_file_info))
                store=stack.enter_context(j.open_journal(str(install),fd,bootstrap=True))
                intent=m._intent(store.installation_id,H,'ENABLE',A,0,raw,generation=G)
                operation=store.bind_caller('same-K',ops._canonical(intent))
                binding=dict(schema='owner-beta-paper-action-binding-v1',installation_id=store.installation_id,
                    generation=G,root_intent_sha256=operation.intent_sha256,sequence=0,previous_binding_sha256=None,
                    verb='enable',request_key=intent['request_key'],expected_epoch=0,artifact_sha256=ops._digest(raw),stage_sha256=None)
                action=operation.prepare_action(ops._canonical(binding),raw)
                original=(operation.generation,operation.intent_sha256,action._binding_bytes,action._artifact_bytes)
                action.close();operation.close()
                owner=object.__new__(m.Lifecycle)
                owner.actions=[];owner.operations=[];owner.imports=[]
                owner._journal=mock.Mock(return_value=store);owner.revalidate=mock.Mock()
                owner.context=types.SimpleNamespace(manifest_sha256=H)
                owner.store=types.SimpleNamespace(pending_at_open=False)
                selected=selection();selected['reset']['control_epoch']=selected_epoch
                owner._selection=mock.Mock(return_value=(selected,{'account_id':A},raw,b'readback'))
                owner._anchors=mock.Mock(return_value=(b'original-anchor',))
                result=m.LifecycleResult('BLOCKED',None,None,None,'COMPLETE')
                owner.last=None
                def blocked_result(_call):
                    owner.last=result
                    return result
                owner._run=mock.Mock(side_effect=blocked_result)
                try:
                    with mock.patch.object(m,'_Call',return_value=object()):
                        if selected_epoch:
                            with self.assertRaisesRegex(j.JournalError,'JOURNAL_CONFLICT'):
                                owner.enable('same-K')
                            owner._run.assert_not_called()
                        else:
                            self.assertIs(owner.enable('same-K'),result)
                            owner._run.assert_called_once()
                            self.assertEqual(len(owner.operations),1)
                            self.assertEqual(len(owner.actions),1)
                            actual=owner.operations[0];actual_action=owner.actions[0]
                            self.assertEqual((actual.generation,actual.intent_sha256,actual_action._binding_bytes,actual_action._artifact_bytes),original)
                            with self.assertRaises(FileNotFoundError):
                                os.stat('dispatch.json',dir_fd=actual_action.borrow_output_fd(),follow_symlinks=False)
                finally:
                    for held in reversed(owner.actions):held.close()
                    for held in reversed(owner.operations):held.close()

class StagingTests(unittest.TestCase):
    def test_actual_invocation_argv_caps_groups_and_mount_set(self):
        for verb,groups in [('configure',()),('observe-reset',()),('readback',()),('enable',('10001',)),('recover',('10001',))]:
            with self.subTest(verb=verb):
                ctx=types.SimpleNamespace(image='sha256:'+'a'*64,network_id='n')
                call=ops._NativeInvocation(ops._INVOCATION_TOKEN,ctx,verb,None)
                call._lifecycle_owner=types.SimpleNamespace(groups=groups,revalidate=lambda:None)
                call._mounts=(('/private/native',ops.PRIVATE,True),('/private/secret','/run/secrets',True),('/private/output',ops.OUTPUT_ALIAS,False))
                args=call._create_args()
                self.assertNotIn('--cap-add',args);self.assertIn('--cap-drop',args);self.assertIn('ALL',args)
                self.assertEqual([args[i+1] for i,v in enumerate(args) if v=='--group-add'],list(groups))
                mounts=[args[i+1] for i,v in enumerate(args) if v=='--mount']
                self.assertEqual(mounts,['type=bind,source=/private/native,target='+ops.PRIVATE+',readonly',
                    'type=bind,source=/private/secret,target=/run/secrets,readonly',
                    'type=bind,source=/private/output,target='+ops.OUTPUT_ALIAS])
                self.assertNotIn('/var/run/docker.sock',' '.join(mounts));self.assertEqual(args[-1],verb)
    def test_every_owned_stage_uses_private_source_sandbox_root(self):
        call=ops._NativeInvocation(ops._INVOCATION_TOKEN,types.SimpleNamespace(_lifecycle_stage_root='/install/.owner-beta-paper-lifecycle/staging'),'inspect',None)
        with mock.patch.object(ops,'_Stage') as stage:
            call._new_stage('query-')
            stage.assert_called_once_with('query-',root='/install/.owner-beta-paper-lifecycle/staging')
    def test_native_result_parser_accepts_new_verbs_but_rejects_altered_query_schema(self):
        for verb in ('configure','enable','recover','observe-reset'):
            schema=ops.QUERY_RESULT_SCHEMA if verb=='observe-reset' else ops.RESULT_SCHEMA
            value=dict(schema=schema,verb=verb,outcome='BLOCKED',event_id=None,generation=None,control_epoch=None,artifact_sha256=None,error_code='EVIDENCE_UNAVAILABLE')
            raw=json.dumps(value,separators=(',',':')).encode()+b'\n'
            self.assertEqual(ops._validate_result(raw,verb,3),value)
            value['schema']=ops.RESULT_SCHEMA if verb=='observe-reset' else ops.QUERY_RESULT_SCHEMA
            with self.assertRaises(ops.PaperOperationsError):ops._validate_result(json.dumps(value,separators=(',',':')).encode(),verb,3)
    def test_final_source_recheck_runs_after_consumer_and_view_failure(self):
        base=m._load('owner_beta_lifecycle_base','owner-beta-daily-base.py')
        for failure in ('consumer','view',None):
            events=[]
            @contextlib.contextmanager
            def source(path):
                events.append(('enter',path));yield {'manifest.json':b'manifest','bars.ndjson':b'bars'};events.append('original-final')
            @contextlib.contextmanager
            def directory(*args):yield 20
            view=mock.Mock()
            def recheck():
                events.append('view-final')
                if failure=='view':raise ValueError('view changed')
            view.revalidate.side_effect=recheck
            with mock.patch.object(base,'source_artifact',source),mock.patch.object(base.install,'child_directory',directory),\
                 mock.patch.object(base.install,'require_root'),mock.patch.object(base.install,'directory_info'),\
                 mock.patch.object(base.os,'mkdir'),mock.patch.object(base.os,'fsync'),\
                 mock.patch.object(base.install,'create_file') as create,mock.patch.object(base,'NativeRootView',return_value=view):
                def consume():
                    with base.native_root_view('/actual/projection/historical-price-beta-root',10) as actual:
                        self.assertIs(actual,view);events.append('consume')
                        if failure=='consumer':raise ValueError('consumer')
                if failure:
                    with self.assertRaises(ValueError):consume()
                else:consume()
                self.assertEqual(create.call_count,2)
                for args in create.call_args_list:self.assertEqual(args.args[-3:],(0,0,0o600))
            self.assertEqual(events,[('enter','/actual/projection/historical-price-beta-root'),'consume','view-final','original-final'])
    def test_input_cleanup_failure_keeps_owned_stage_and_cannot_publish(self):
        inputs=object.__new__(m._Inputs);inputs.closed=False;inputs.revalidate=mock.Mock();inputs.originals=[]
        inputs.stack=mock.Mock();inputs.stack.close.side_effect=m.LifecycleError('SOURCE_REPLACED')
        with self.assertRaises(m.LifecycleError):inputs.close()
        self.assertFalse(inputs.closed)

class FinalPublicationTests(unittest.TestCase):
    def test_anchor_waits_for_context_cleanup_and_rollback_preserves_old_anchors(self):
        for fail in ('context','store',None):
            owner=object.__new__(m.Lifecycle);events=[]
            owner.closed=False;owner.calls=[];owner.imports=[];owner.actions=[];owner.operations=[];owner.journal_manager=None
            owner.revalidate=lambda:None;owner.deadline=mock.Mock();owner.pending_anchors=[ops._canonical(anchor())]
            owner.context=mock.Mock();owner.store=mock.Mock(dirs={'anchors':50})
            def context_close():
                events.append('context-clean')
                if fail=='context':raise ValueError('late context')
            def store_close():
                events.append('store-clean')
                if fail=='store':raise ValueError('late store')
            owner.context.close.side_effect=context_close;owner.store.close.side_effect=store_close
            owner.store.publish.side_effect=lambda *a:events.append('published')
            with mock.patch.object(m.os,'dup',return_value=51),mock.patch.object(m.os,'stat',side_effect=FileNotFoundError()),\
                 mock.patch.object(m.os,'close'),mock.patch.object(m.os,'unlink') as unlink,mock.patch.object(m.os,'fsync'),\
                 mock.patch.object(j,'_read_named',return_value=owner.pending_anchors[0]):
                if fail:
                    with self.assertRaises(ValueError):owner.close()
                else:owner.close()
                if fail=='context':self.assertEqual(events,['context-clean']);unlink.assert_not_called()
                elif fail=='store':self.assertEqual(events,['context-clean','published','store-clean']);unlink.assert_called_once()
                else:self.assertEqual(events,['context-clean','published','store-clean']);unlink.assert_not_called()

class RetainedAnchorSelectionTests(unittest.TestCase):
    def chains(self):
        import datetime
        evidence=m._load('owner_beta_lifecycle_evidence','owner-beta-paper-evidence.py')
        raw=(ROOT/'crates/job-queue/tests/fixtures/owner_beta_paper_source_evidence/valid_chain.json').read_text()
        result=[]
        for index,boot in enumerate((I,A,G,E)):
            day='2026-11-%02d' % (index+2)
            events=json.loads(raw.replace('2026-10-29',day))['events']
            delta=(datetime.date.fromisoformat(day)-datetime.date(2026,10,29)).days*86400*10**9
            previous=None;source_hash=None;original_hash=None
            for event in events:
                event['due_realtime_ns']+=delta
                event['arm']['boot_id']=boot
                event['slot_id']=evidence.slot_id(event['cohort'],day)
                for key in event['observed']:event['observed'][key]+=delta
                event['previous_sha256']=previous
                if event['kind']=='SOURCE_FINISHED':
                    original_hash=evidence.original_hash(event['data']['originals'])
                    event['data']['original_set_sha256']=original_hash
                if event['kind']=='ADMISSION_LINKED':
                    event['data']['original_set_sha256']=original_hash
                    event['data']['source_event_sha256']=source_hash
                previous=evidence.digest(evidence.canonical(event))
                if event['kind']=='SOURCE_FINISHED':source_hash=previous
            evidence.verify_chain(events,'LINKED')
            result.append(events)
        return evidence,result
    def test_four_original_boots_select_last_three_actual_validated_chains(self):
        evidence,chains=self.chains()
        slots={events[0]['slot_id'][7:]:100+i for i,events in enumerate(chains)}
        files={100+i:{'%06d.json'%n:evidence.canonical(event) for n,event in enumerate(events)} for i,events in enumerate(chains)}
        inputs=object.__new__(m._Inputs);inputs.events_fd=10;inputs.binding={'cohort':chains[0][0]['cohort']}
        inputs.controller=types.SimpleNamespace(deadline=mock.Mock());inputs.stack=contextlib.ExitStack()
        inputs.checks=[];inputs.originals=[];inputs.closed=False;inputs.view=None
        originals=[]
        for boot in (I,A,G,E):originals.append(ops._canonical(dict(anchor(),boot_id=boot)))
        def scan(fd):return contextlib.nullcontext(iter(types.SimpleNamespace(name=name) for name in (slots if fd==10 else files[fd])))
        with mock.patch.object(m.os,'scandir',side_effect=scan),mock.patch.object(m.os,'open',side_effect=lambda name,*a,**k:slots[name]),\
             mock.patch.object(m.os,'close'),mock.patch.object(m.os,'fstat'),mock.patch.object(m.os,'stat'),\
             mock.patch.object(m.os,'listdir',side_effect=lambda fd:list(files[fd])),mock.patch.object(ops,'_identity',return_value=1),\
             mock.patch.object(m.daily.install,'directory_info'),mock.patch.object(m,'_Original',side_effect=lambda fd,name,**k:mock.Mock(raw=files[fd][name])):
            selected=inputs.anchor_originals(selection(),tuple(originals))
            self.assertEqual({json.loads(raw)['boot_id'] for raw in selected},{A,G,E})
            self.assertEqual(len(originals),4);self.assertEqual(len(inputs.originals),60)
            late=dict(anchor(),boot_id=E,observed=chains[-1][1]['observed'])
            with self.assertRaisesRegex(m.LifecycleError,'LIFECYCLE_LATE_ANCHOR'):
                inputs.anchor_originals(selection(),tuple(originals[:-1]+[ops._canonical(late)]))
            inputs.stack.close()

if __name__=='__main__':unittest.main()
