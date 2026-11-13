"""Effects-mocked private native fixture tests; no native/PG/Source execution."""
import ast
import contextlib
import importlib.util
import io
import os
from pathlib import Path
import sys
import tempfile
import types
import unittest
from unittest import mock

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


class Guarded(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.guards = contextlib.ExitStack()
        for target in ('subprocess.Popen', 'subprocess.run', 'socket.socket', 'os.chown', 'os.fchown',
                       'os.pidfd_open', 'fcntl.flock', 'fcntl.fcntl'):
            cls.guards.enter_context(mock.patch(target, side_effect=AssertionError('UNPLANNED_EFFECT')))
        cls.n = load('native_source_test_subject', HERE / 'lib/owner-beta-paper-native-source.py')
        cls.h = cls.n.history()
        cls.f = cls.h.fixture_module()
        cls.cli = load('native_source_cli_subject', HERE / 'verify-owner-beta-paper-native-source.py')

    @classmethod
    def tearDownClass(cls):
        cls.guards.close()

    def setUp(self):
        self.stack = contextlib.ExitStack()
        self.addCleanup(self.stack.close)

    def patch(self, obj, name, **kwargs):
        return self.stack.enter_context(mock.patch.object(obj, name, **kwargs))


class Selection(Guarded):
    def output(self, name):
        return ('running 1 test\ntest ' + self.n.PREFIX + name + ' ... ok\n\n'
            'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 300 filtered out; finished in 0.30s\n').encode()

    def test_exact_full_names_and_one_real_result(self):
        for name, marker in self.n.CASES:
            self.assertTrue(marker.endswith(b'\n'))
            self.assertEqual(self.n.test_argv(name), [self.n.PREFIX + name, '--exact', '--include-ignored',
                '--test-threads=1', '--show-output'])
            self.n.named_success(self.output(name), name, 0)

    def test_ignored_empty_duplicate_filtered_or_wrong_named_result_is_not_success(self):
        name = self.n.CASES[0][0]
        output = self.output(name)
        wrong = self.n.CASES[1][0].encode()
        for bad in (b'', output.replace(name.encode(), wrong), output + output,
                    output.replace(b'1 passed', b'0 passed'), output.replace(b'0 ignored', b'1 ignored'),
                    output.replace(b' ... ok', b' ... ignored'), output + b'test extra ... ok\n',
                    output + b'\xff', b'x' * 65537):
            with self.subTest(bad=bad[:100]), self.assertRaises(self.n.NativeFixtureError):
                self.n.named_success(bad, name, 0)
        with self.assertRaises(self.n.NativeFixtureError):
            self.n.named_success(output, name, 1)

    def test_no_substring_or_root8_selection(self):
        for name in ('actual_native', self.n.PREFIX + self.n.CASES[0][0], '--list', '', 'ALL'):
            with self.subTest(name=name), self.assertRaises(self.n.NativeFixtureError):
                self.n.test_argv(name)

    def test_cli_rejects_ambiguous_all_or_unowned_source_entrypoints(self):
        args = ['--source-root', '/source', '--commit', 'a' * 40, '--manifest-file', '/manifest',
                '--deadline-monotonic-ns', '123', '--approved-base', '/base', '--raw-root', '/raw',
                '--daily-config', '/config', '--generic-bundle', '/generic', '--generic-session', '2026-11-11']
        self.assertEqual(self.cli.parser().parse_args(args + ['bootstrap']).phase, 'bootstrap')
        for phase in ('all', 'run', 'source-service', '--ready'):
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
                self.cli.parser().parse_args(args + [phase])


class Callback(Guarded):
    def preparation(self):
        owner = object.__new__(self.f.FixtureOwner)
        owner.root = Path('/var/lib/lagrange-paper-operations-fixture-test')
        owner.root_id = (3, 5)
        owner.commit = 'a' * 40
        owner.context = owner.observer = owner.lock_fd = None
        material = types.SimpleNamespace(revalidate=mock.Mock())
        result = self.n.Preparation(owner, material, '/protected/config')
        self.patch(result, 'revalidate')
        owner.observer = types.SimpleNamespace(target=mock.Mock())
        owner.network, owner.pg = 'network', 'database'
        stage = owner.root / 'releases' / ('.staging.' + owner.commit + '.nonce')
        result.config_raw = b'actual-config'
        return result, owner, stage

    def test_real_original_callback_consumes_owned_unpublished_stage_once(self):
        p, o, stage = self.preparation()
        events = []
        install = self.f.ops.daily.install
        self.patch(install, 'stage_config', side_effect=lambda *args: events.append(('stage', args)))
        self.patch(install, 'verify_config', return_value=({}, self.h.digest(p.config_raw)))
        runtime = types.SimpleNamespace(close=mock.Mock())
        self.patch(self.h, '_initialize_runtime_material', side_effect=lambda *args: events.append(('material', args)) or runtime)
        p.prepare(o, stage)
        self.assertIs(p.runtime, runtime)
        self.assertEqual([e[0] for e in events], ['stage', 'material'])
        self.assertIs(events[1][1][0], o)
        self.assertIs(events[1][1][1], p.material)
        self.assertEqual(events[1][1][-1], stage / 'configs/evidence/owner-beta-daily-price-policy-v1.json')
        with self.assertRaises(self.n.NativeFixtureError):
            p.prepare(o, stage)
        self.assertEqual(len(events), 2)

    def test_foreign_stage_owner_existing_context_or_release_rejected_before_writes(self):
        for change in ('context', 'release', 'stage', 'owner', 'db'):
            p, o, stage = self.preparation()
            stage_config = self.patch(self.f.ops.daily.install, 'stage_config')
            passed = o
            if change == 'context': o.context = object()
            if change == 'release': o.release = stage
            if change == 'stage': stage = Path('/foreign/.staging.' + o.commit + '.nonce')
            if change == 'owner': passed = object()
            if change == 'db': o.pg = None
            with self.subTest(change=change), self.assertRaises(self.n.NativeFixtureError):
                p.prepare(passed, stage)
            stage_config.assert_not_called()

    def test_partial_material_failure_cannot_reuse_callback(self):
        p, o, stage = self.preparation()
        self.patch(self.f.ops.daily.install, 'stage_config')
        self.patch(self.f.ops.daily.install, 'verify_config', return_value=({}, self.h.digest(p.config_raw)))
        build = self.patch(self.h, '_initialize_runtime_material', side_effect=RuntimeError('private'))
        with self.assertRaises(RuntimeError): p.prepare(o, stage)
        with self.assertRaises(self.n.NativeFixtureError): p.prepare(o, stage)
        build.assert_called_once()
        self.assertFalse(p.published)

    def test_unpublished_context_is_not_a_published_release(self):
        p, o, stage = self.preparation()
        p.used = True
        p.runtime = types.SimpleNamespace(config_root=None)
        o.release = stage
        o.context = object()
        verify = self.patch(self.f.ops.release, 'verify')
        with self.assertRaises(self.n.NativeFixtureError): p.after_publish()
        verify.assert_not_called()

    def test_prepared_process_is_not_initial_off_reader(self):
        p, o, _ = self.preparation()
        p.used = True
        p.runtime = types.SimpleNamespace(config_root=None)
        o.release = o.root / 'releases' / o.commit
        o.context = object()
        checked = mock.MagicMock(process_sha256='sha256:' + 'b' * 64)
        checked.__enter__.return_value = checked
        self.patch(self.f.ops.release, 'verify', return_value=checked)
        with self.assertRaises(self.n.NativeFixtureError): p.after_publish()
        self.assertFalse(p.published)

    def test_exit_cleanup_failure_retains_registry_and_original_scope(self):
        p, _, _ = self.preparation()
        p.entered = True
        self.n._OWNERS.add(p)
        p.runtime = types.SimpleNamespace(close=mock.Mock(side_effect=RuntimeError('unknown')))
        p.stack = contextlib.ExitStack()
        released = []
        p.stack.callback(lambda: released.append(True))
        with self.assertRaises(RuntimeError): p.__exit__(None, None, None)
        self.assertFalse(p.closed)
        self.assertIn(p, self.n._OWNERS)
        self.assertIn(p, self.n._UNRESOLVED_PREPARATIONS)
        self.assertEqual(released, [])
        self.addCleanup(p.stack.close)
        self.addCleanup(self.n._OWNERS.discard, p)
        self.addCleanup(self.n._UNRESOLVED_PREPARATIONS.discard, p)

    def test_c_archive_rejects_native_owner_before_any_driver_call(self):
        directory = self.stack.enter_context(tempfile.TemporaryDirectory())
        root = Path(directory)
        (root / self.n.OWNER_MARKER).write_bytes(b'not a permit')
        driver = self.patch(self.f.driver, 'run_archive_acceptance')
        runner = object.__new__(self.f.FixtureRunner)
        with self.assertRaisesRegex(self.f.FixtureError, 'FIXTURE_PURPOSE'):
            runner.archive_acceptance(types.SimpleNamespace(root=root), {})
        driver.assert_not_called()


class StaticBoundaries(Guarded):
    def test_default_setup_retains_no_callback_and_native_callback_precedes_publication(self):
        tree = ast.parse((HERE / 'owner-beta-paper-genuine-fixture.py').read_text())
        owner = next(n for n in tree.body if isinstance(n, ast.ClassDef) and n.name == 'FixtureOwner')
        setup = next(n for n in owner.body if isinstance(n, ast.FunctionDef) and n.name == 'setup')
        self.assertEqual(setup.args.kwonlyargs[0].arg, '_prepare_release')
        self.assertIsNone(setup.args.kw_defaults[0].value)
        callback_branch = next(n for n in setup.body if isinstance(n, ast.If)
            and ast.unparse(n.test) == '_prepare_release is not None')
        calls = [(n.lineno, ast.unparse(n.func)) for n in ast.walk(setup) if isinstance(n, ast.Call)]
        line = lambda name: min(number for number, function in calls if function == name)
        self.assertLess(line('self._setup_database'), line('_prepare_release'))
        self.assertLess(line('_prepare_release'), line('ops.release.stage'))
        self.assertLess(line('ops.release.stage'), line('staging.rename'))
        self.assertLess(line('staging.rename'), line('ops.release.verify'))
        self.assertLess(line('ops.release.verify'), line('ops.daily.Context'))
        self.assertTrue(any(isinstance(n, ast.Call) and ast.unparse(n.func) == '_prepare_release'
                            for n in ast.walk(callback_branch)))

    def test_production_gate_and_all_named_tests_remain_ignored(self):
        path = HERE.parents[1] / 'crates/job-queue/src/owner_beta/paper/operations/protected.rs'
        raw = path.read_text()
        self.assertIn('const ENABLE_SQL_RESET_AND_READBACK_READY: bool = false;', raw)
        gate = raw.index('if !ENABLE_SQL_RESET_AND_READBACK_READY')
        shared = raw.index('self.enable_checked_transaction(action, prepared).await')
        self.assertLess(gate, shared)
        for name, _ in self.n.CASES:
            pos = raw.index('async fn ' + name + '(')
            self.assertIn('#[ignore =', raw[raw.rfind('#[tokio::test', 0, pos):pos])
        self.assertIn('actual ARM must precede actual RESET, not just anchor', raw)
        for timestamp in ('ss.reserved_at', 'dr.created_at', 'r.created_at', 'sa.linked_at', 'sr.linked_at'):
            self.assertIn(timestamp + '>e.occurred_at', raw)


class BetaProducer(Guarded):
    def runtime(self):
        owner = '11111111-1111-4111-8111-111111111111'
        identifier = '22222222-2222-4222-8222-222222222222'
        config = dict(benchmark_instrument='069500.KRX', fast_ma=50, slow_ma=200)
        actual = dict(id=identifier, owner=owner, strategy='trend_following', version='1.0.0',
                      config=config, active=True, daily=True, epoch=1)
        runtime = types.SimpleNamespace(config={'owner': owner},
            http=mock.Mock(return_value=(201, {'id': identifier})),
            owner=types.SimpleNamespace(observer=types.SimpleNamespace(json=mock.Mock(return_value=actual))))
        return runtime, actual

    def test_http_returned_beta_id_and_independent_db_read_are_required(self):
        runtime, actual = self.runtime()
        self.assertEqual(self.h.create_beta_config(runtime), actual['id'])
        method, path, body = runtime.http.call_args.args
        self.assertEqual((method, path), ('POST', '/api/v1/strategies/trend_following/configs'))
        self.assertEqual(body['config'], dict(benchmark_instrument='069500.KRX', fast_ma=50, slow_ma=200))
        self.assertTrue(body['owner_beta_daily_enabled'])
        query = runtime.owner.observer.json.call_args.args[0]
        self.assertIn(actual['id'], query)
        self.assertIn('BEGIN READ ONLY', query)
        self.assertNotIn('INSERT', query)
        self.assertNotIn('UPDATE', query)

    def test_wrong_actual_config_owner_strategy_epoch_or_flags_refuses(self):
        for key, changed in (('id', '33333333-3333-4333-8333-333333333333'), ('owner', 'foreign'),
                ('strategy', 'buy_and_hold'), ('version', '2.0.0'), ('config', {}),
                ('active', False), ('daily', False), ('epoch', True), ('epoch', -1)):
            runtime, actual = self.runtime()
            runtime.owner.observer.json.return_value = dict(actual, **{key: changed})
            with self.subTest(key=key, changed=changed), self.assertRaises(self.h.HistoryError):
                self.h.create_beta_config(runtime)

    def test_http_failure_never_reads_or_claims_an_id(self):
        runtime, _ = self.runtime()
        runtime.http.return_value = (409, {'id': '22222222-2222-4222-8222-222222222222'})
        with self.assertRaises(self.h.HistoryError): self.h.create_beta_config(runtime)
        runtime.owner.observer.json.assert_not_called()

    def test_shared_c_bootstrap_keeps_real_generic_and_beta_ids_separate(self):
        source = (HERE / 'lib/owner-beta-paper-genuine-history.py').read_text()
        tree = ast.parse(source)
        bootstrap = next(n for n in tree.body if isinstance(n, ast.FunctionDef) and n.name == 'bootstrap')
        body = ast.unparse(bootstrap)
        self.assertIn('generic_strategy = runtime.strategy_id', body)
        self.assertIn('beta_strategy = create_beta_config(runtime)', body)
        self.assertIn("'strategy_config_id': beta_strategy", body)
        self.assertIn('generic_preview(runtime, generic_strategy, generic_session)', body)

    def test_inline_daily_producer_does_not_wait_for_its_own_parent_link(self):
        calls = []
        runtime = types.SimpleNamespace(config={'owner': '11111111-1111-4111-8111-111111111111'},
            owner=types.SimpleNamespace(commit='a'*40, observer=types.SimpleNamespace(
                json=mock.Mock(return_value='sha256:'+'b'*64))),
            verify_base_views=lambda: calls.append('views'),
            daily_once=lambda role, result: calls.append((role, result)))
        self.assertEqual(self.h.daily_produce(runtime), 'sha256:'+'b'*64)
        self.assertEqual(calls, ['views', ('producer', 'CANDIDATE_READY'), ('admitter', 'ADMITTED'),
                               ('worker', 'SUCCEEDED'), 'views'])
        runtime.owner.observer.json.assert_called_once()


class PhaseCleanup(Guarded):
    def phase(self):
        phase = object.__new__(self.n.OwnedPhase)
        phase.closed = False
        phase.stack = contextlib.ExitStack()
        closed = []
        phase.stack.callback(lambda: closed.append('originals'))
        phase.owner = types.SimpleNamespace(contexts=[], helpers={}, helper_bindings={}, tracees=[],
            threads=[], blockers=[], lock_fd=51, observer=types.SimpleNamespace(absent=mock.Mock()))
        self.patch(phase, 'revalidate')
        self.patch(phase, 'quiescent')
        self.n._PHASES.add(phase)
        self.addCleanup(self.n._PHASES.discard, phase)
        self.addCleanup(self.n._UNRESOLVED_PHASES.discard, phase)
        self.addCleanup(phase.stack.close)
        return phase, closed

    def test_backend_unknown_retains_originals_and_lock(self):
        phase, closed = self.phase()
        phase.owner.observer.absent.side_effect = RuntimeError('unknown')
        with self.assertRaises(RuntimeError): phase.__exit__(None, None, None)
        self.assertEqual(closed, [])
        self.assertEqual(phase.owner.lock_fd, 51)
        self.assertIn(phase, self.n._UNRESOLVED_PHASES)
        self.assertFalse(phase.closed)

    def test_open_borrower_blocks_all_descriptor_disposal(self):
        phase, closed = self.phase()
        phase.owner.contexts = [types.SimpleNamespace(_closed=False)]
        with self.assertRaises(self.n.NativeFixtureError): phase.__exit__(None, None, None)
        self.assertEqual(closed, [])
        phase.owner.observer.absent.assert_not_called()

    def test_success_rechecks_actual_quiescence_before_originals_close(self):
        phase, closed = self.phase()
        phase.quiescent.side_effect = lambda **_: self.assertEqual(closed, [])
        phase.__exit__(None, None, None)
        self.assertTrue(phase.closed)
        self.assertEqual(closed, ['originals'])
        self.assertIsNone(phase.owner.lock_fd)
        phase.quiescent.assert_called_once_with(cleanup=True)


class NativeExecution(Guarded):
    def caller(self):
        phase = types.SimpleNamespace(owner=types.SimpleNamespace(network='owned-network',
            original_custody_uncertain=False, observer=types.SimpleNamespace(absent=mock.Mock())),
            quiescent=mock.Mock(), closed=False)
        caller = object.__new__(self.n.NativeCaller)
        caller.phase, caller.path = phase, Path('/held/test.elf')
        caller.closed = False
        caller.identity_checked = True
        caller.results = []
        caller.controllers, caller.inputs, caller.stages = [], [], []
        caller.stack = contextlib.ExitStack()
        self.addCleanup(caller.stack.close)
        self.addCleanup(self.n._UNRESOLVED_CALLERS.discard, caller)
        self.patch(caller, 'revalidate')
        runtime = types.SimpleNamespace(created=[], uncertain_creations=set(), images={'paper-scheduler':'image'},
            mount=mock.Mock(side_effect=lambda *args: args), create=mock.Mock(return_value='owned-child'),
            inspect=mock.Mock(side_effect=[{'HostConfig':{'GroupAdd':['10001']}},
                {'State':{'Running':False,'OOMKilled':False,'ExitCode':0}}]),
            command=mock.Mock(), close=mock.Mock())
        caller.runtime = runtime
        return caller, runtime

    def output(self, name):
        return ('test '+self.n.PREFIX+name+' ... ok\n'
            'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 400 filtered out; finished in 0.20s\n').encode()

    def test_exact_named_native_call_is_closed_and_uses_fifteen_second_cap(self):
        caller, runtime = self.caller()
        name = self.n.CASES[0][0]
        runtime.command.return_value = self.output(name)
        result = caller.execute(name, [('owned-input', '/run/owner-beta-paper-ops')])
        self.assertEqual(result, self.output(name))
        self.assertEqual(runtime.create.call_args.args[-1], self.n.test_argv(name))
        runtime.command.assert_called_once_with(['start','--attach','--interactive','owned-child'], limit=15)
        runtime.close.assert_called_once()
        caller.phase.owner.observer.absent.assert_called_once_with(cleanup=True)
        caller.phase.quiescent.assert_called_once_with(cleanup=True)

    def test_wrong_named_output_still_closes_actual_owned_child(self):
        caller, runtime = self.caller()
        runtime.command.return_value = self.output(self.n.CASES[1][0])
        with self.assertRaises(self.n.NativeFixtureError): caller.execute(self.n.CASES[0][0], [])
        runtime.close.assert_called_once()
        self.assertEqual(caller.results, [])

    def test_no_compiled_identity_no_child_creation(self):
        caller, runtime = self.caller()
        caller.identity_checked = False
        with self.assertRaises(self.n.NativeFixtureError): caller.execute(self.n.CASES[0][0], [])
        runtime.create.assert_not_called()

    def test_cleanup_unknown_preserves_owner_and_never_reports_pass(self):
        caller, runtime = self.caller()
        runtime.command.return_value = self.output(self.n.CASES[0][0])
        runtime.close.side_effect = RuntimeError('unknown')
        with self.assertRaises(RuntimeError): caller.execute(self.n.CASES[0][0], [])
        self.assertTrue(caller.phase.owner.original_custody_uncertain)
        self.assertIn(caller, self.n._UNRESOLVED_CALLERS)
        with self.assertRaises(self.n.NativeFixtureError): caller.report()

    def test_report_is_unavailable_until_caller_and_phase_close(self):
        caller, _ = self.caller()
        caller.results = [{'status':'OBSERVED_TEST_OK_PENDING_CLOSE','cleanup':'PENDING'}]
        for c, p in ((False, False), (True, False), (False, True)):
            caller.closed, caller.phase.closed = c, p
            with self.assertRaises(self.n.NativeFixtureError): caller.report()
        caller.closed = caller.phase.closed = True
        self.assertEqual(caller.report()[0]['status'], 'PASS_TEST_ORIGIN')
        self.assertEqual(caller.report()[0]['cleanup'], 'COMPLETE')

    def test_source_case_database_change_refuses_success(self):
        caller, _ = self.caller()
        @contextlib.contextmanager
        def inputs(*_):
            yield object(), []
        self.patch(caller, 'source_inputs', side_effect=inputs)
        self.patch(caller, 'execute', return_value=self.output(self.n.CASES[0][0]))
        self.patch(self.n, 'database_image', side_effect=[{'events':[]}, {'events':[{'id':'new'}]}])
        with self.assertRaises(self.n.NativeFixtureError): caller.source_case(object(), self.n.CASES[0][0])
        self.assertEqual(caller.results, [])


class DefaultRoutes(Guarded):
    def test_default_scheduler_barrier_and_hooks_remain_strict(self):
        e = self.h.source_module().evidence()
        scheduler = object.__new__(e.Scheduler)
        scheduler.path, scheduler.authority = 'original-binding', object()
        barrier = self.patch(e, 'source_lifecycle_barrier', return_value='actual-result')
        self.assertEqual(scheduler.source_lifecycle_barrier(), 'actual-result')
        barrier.assert_called_once_with(scheduler.path, scheduler.authority)
        self.assertIsNone(scheduler.armed())
        self.assertIsNone(scheduler.source_finished())
        body = ast.unparse(ast.parse(Path(e.__file__).read_text()))
        self.assertIn('Scheduler(args.binding).run()', body)
        self.assertIn('ExecStart=/usr/bin/python3', Path(e.__file__).read_text())

    def test_private_barrier_rejects_unregistered_scheduler_before_inventory(self):
        phase = object.__new__(self.n.OwnedPhase)
        self.patch(phase, 'revalidate')
        context = self.patch(self.h, 'lifecycle_context')
        with self.assertRaises(self.n.NativeFixtureError): phase.source_barrier(object())
        context.assert_not_called()

    def test_real_daily_off_is_sql_state_not_an_invented_config_field(self):
        raw = (HERE.parents[1] / 'crates/job-queue/src/owner_beta/paper/operations/protected.rs').read_text()
        self.assertIn('normal config has no activation field', raw)
        self.assertIn('AND NOT d.enabled', raw)
        self.assertIn('readback must not mutate Daily or Paper control', raw)
        self.assertNotIn('assert_eq!(daily["enabled"], false)', raw)
        native = (HERE / 'lib/owner-beta-paper-native-source.py').read_text()
        self.assertNotIn('dict(config, enabled=False)', native)
        self.assertIn("'__ACTION__': 'disable'", native)


class Barrier(Guarded):
    def owned(self, configured=True):
        e = self.h.source_module().evidence()
        l = self.h.lifecycle_module()
        boot = '55555555-5555-4555-8555-555555555555'
        phase = object.__new__(self.n.OwnedPhase)
        self.patch(phase, 'revalidate')
        owner = types.SimpleNamespace(root=Path('/owned/root'), release=Path('/owned/root/releases/'+'a'*40),
            commit='a'*40, manifest=b'manifest', lock_fd=51, deadline=types.SimpleNamespace(value=1234),
            contexts=[], context_modules={})
        phase.owner = owner
        b = dict(release_root=str(owner.release), data_root='/owned/root/data',
            cohort=dict(verifier_revision=owner.commit, release_manifest_sha256=self.h.digest(owner.manifest)))
        scheduler = types.SimpleNamespace(b=b, authority=types.SimpleNamespace(poll=mock.Mock()))
        # SimpleNamespace is unhashable; a plain instance is the registered owner.
        scheduler = type('SchedulerDouble', (), dict(b=b, authority=scheduler.authority))()
        self.n._SCHEDULERS[scheduler] = phase
        self.addCleanup(self.n._SCHEDULERS.pop, scheduler, None)
        self.patch(self.h, 'lifecycle_context', return_value=object())
        controller = mock.MagicMock()
        controller.__enter__.return_value = controller
        controller.__exit__.return_value = False
        controller._journal.return_value = types.SimpleNamespace(epoch_floor=0,
            installation_id='11111111-1111-4111-8111-111111111111')
        self.patch(l, 'open_lifecycle', return_value=controller)
        inspection = mock.MagicMock()
        self.patch(l.ops, '_inspect_generation_null', return_value=inspection)
        self.patch(l.ops, '_read_inventory', return_value=(None, None, False, configured))
        self.patch(phase, 'process_proposal', return_value=(dict(control_epoch=1,
            account_id='22222222-2222-4222-8222-222222222222', daily_cohort_sha256='sha256:'+'b'*64), b'proposal'))
        self.patch(l, '_Call', side_effect=lambda owner, verb, **kw: types.SimpleNamespace(verb=verb, **kw))
        self.patch(l, '_exact_readback', return_value=dict(status='RESET', current_state='OFF', reset={'actual':'reset'}))
        self.patch(l, '_anchor', return_value={'boot_id':boot})
        self.patch(self.n.Path, 'read_text', return_value=boot+'\n')
        controller._anchors.return_value = ()
        controller._run.side_effect = [l.LifecycleResult('OK', None, b'readback', None, 'COMPLETE'),
            l.LifecycleResult('OK', None, None, b'actual-anchor', 'COMPLETE')]
        return phase, scheduler, controller, l

    def test_only_actual_unconfigured_inventory_returns_no_anchor(self):
        phase, scheduler, controller, _ = self.owned(configured=False)
        self.assertIsNone(phase.source_barrier(scheduler))
        controller._run.assert_not_called()
        phase.process_proposal.assert_not_called()
        self.assertEqual(scheduler.authority.poll.call_count, 2)

    def test_configured_barrier_observes_reset_with_untrusted_proposal(self):
        phase, scheduler, controller, _ = self.owned()
        result = phase.source_barrier(scheduler)
        self.assertEqual(result.artifact, b'actual-anchor')
        calls = [item.args[0] for item in controller._run.call_args_list]
        self.assertEqual([c.verb for c in calls], ['readback', 'observe-reset'])
        self.assertEqual(calls[1].evidence, b'proposal')
        self.assertEqual(calls[1].selection['reset'], {'actual':'reset'})
        controller._process.assert_not_called()
        controller.pending_anchors.append.assert_called_once_with(b'actual-anchor')

    def test_existing_local_anchor_replays_without_minting_another(self):
        phase, scheduler, controller, _ = self.owned()
        controller._anchors.return_value = (b'held-anchor',)
        result = phase.source_barrier(scheduler)
        self.assertEqual((result.outcome, result.artifact), ('REPLAY', b'held-anchor'))
        controller._run.assert_called_once()
        controller.pending_anchors.append.assert_not_called()

    def test_unknown_native_reset_or_cleanup_is_never_an_anchor(self):
        for outcome, cleanup in (('COMMIT_UNKNOWN','COMPLETE'), ('OK','UNKNOWN')):
            phase, scheduler, controller, l = self.owned()
            controller._run.side_effect = [l.LifecycleResult(outcome,None,b'query',None,cleanup)]
            with self.subTest(outcome=outcome, cleanup=cleanup), self.assertRaises(self.n.NativeFixtureError):
                phase.source_barrier(scheduler)
            controller.pending_anchors.append.assert_not_called()

    def test_stale_journal_floor_stops_before_native_dispatch(self):
        phase, scheduler, controller, _ = self.owned()
        controller._journal.return_value.epoch_floor = 1
        with self.assertRaises(self.n.NativeFixtureError): phase.source_barrier(scheduler)
        controller._run.assert_not_called()


class OfflineProducer(Guarded):
    def fixture(self):
        owner_id = '11111111-1111-4111-8111-111111111111'
        strategy = '22222222-2222-4222-8222-222222222222'
        account = '33333333-3333-4333-8333-333333333333'
        boot = '44444444-4444-4444-8444-444444444444'
        phase = type('PhaseDouble', (), {})()
        phase.revalidate, phase.quiescent = mock.Mock(), mock.Mock()
        phase.facts = dict(owner_user_id=owner_id, strategy_config_id=strategy)
        observer = types.SimpleNamespace(json=mock.Mock(side_effect=[
            dict(state='ENABLED', owner_user_id=owner_id, strategy_config_id=strategy, account_id=account, control_epoch=1),
            [], dict(outcome='OK', control_epoch=2, data={'intent_sha256':'sha256:'+'a'*64}),
            dict(outcome='OK', data=dict(summary={'unresolved_commits':0,'live_claims':0},
                summary_sha256='sha256:'+'b'*64, observation_id=boot)),
            dict(outcome='OK', control_epoch=3, data={'state':'OFF'}),
            dict(enabled=False, owner_user_id=owner_id)]), sql=mock.Mock(return_value=b'DAILY_CONTROL: DISABLED'))
        owner = types.SimpleNamespace(root=Path('/owned/root'), root_id=(1,2), release=Path('/owned/root/releases/'+'a'*40),
            commit='a'*40, manifest=b'manifest', helpers={}, helper_bindings={}, tracees=[], threads=[], blockers=[],
            pg='pg-id', network='network-id', lock_fd=51, original_custody_uncertain=False,
            observer=observer, docker=types.SimpleNamespace(object=mock.Mock(return_value={'Containers':{'pg-id':{}}})))
        observer.generation = ('created','started',0,'image')
        phase.owner = owner
        self.n._PHASES.add(phase)
        self.addCleanup(self.n._PHASES.discard, phase)
        e = self.h.source_module().evidence()
        @contextlib.contextmanager
        def opened(*args, **kw): yield 70
        self.patch(e, 'opened', side_effect=opened)
        self.patch(e, 'decode', side_effect=lambda raw: {'status':'INVALID'} if raw == b'current' else {})
        self.patch(e, 'validate_current')
        self.patch(e, 'sample', return_value={'realtime_ns':1000,'monotonic_ns':2000,'boottime_ns':3000})
        self.patch(e, 'read_file', side_effect=[b'config', b'policy'])
        self.patch(self.n.os, 'pread', return_value=b'current')
        self.stack.enter_context(mock.patch('fcntl.flock'))
        context = mock.MagicMock()
        context.__enter__.return_value = types.SimpleNamespace(installation_id=boot,revalidate=mock.Mock())
        self.patch(self.f.ops.journal, 'open_journal', return_value=context)
        template = (HERE.parents[1] / 'deploy/db/owner-beta-daily-control.sql').read_text()
        # Path.read_text is called as a method; autospec supplies the path.
        self.stack.enter_context(mock.patch.object(self.n.Path, 'read_text', autospec=True,
            side_effect=lambda path, **kw: boot+'\n' if str(path).startswith('/proc/') else template))
        self.patch(self.f.ops.daily.install, 'validate_config', return_value={'owner':owner_id})
        writes = self.patch(self.f, 'write_new')
        return phase, observer, writes

    def test_off_uses_actual_sql_sequence_then_existing_daily_disable(self):
        phase, observer, writes = self.fixture()
        result = self.n.later_off(phase)
        self.assertEqual(result['status'], 'OFF_TEST_ORIGIN')
        queries = [call.args[0] for call in observer.json.call_args_list]
        self.assertIn('begin_owner_beta_paper_operation_v1', queries[2])
        self.assertIn('reconcile_owner_beta_paper_operation_v1', queries[3])
        self.assertIn("transition_owner_beta_paper_v1(2,'OFF'", queries[4])
        self.assertFalse(any('UPDATE' in query or 'INSERT' in query for query in queries))
        self.assertIn("'disable'", observer.sql.call_args.args[0])
        self.assertEqual(len(writes.call_args_list), 1)
        self.assertEqual(writes.call_args.args[0].name, 'native4-stop-originals.json')
        self.assertNotIn(b'"enabled"', writes.call_args.args[1])

    def test_unknown_begin_stops_without_reconcile_retry_or_off(self):
        phase, observer, writes = self.fixture()
        observer.json.side_effect = [
            dict(state='ENABLED',owner_user_id=phase.facts['owner_user_id'],strategy_config_id=phase.facts['strategy_config_id'],
                 account_id='33333333-3333-4333-8333-333333333333',control_epoch=1), [], RuntimeError('unknown')]
        with self.assertRaises(RuntimeError): self.n.later_off(phase)
        self.assertTrue(phase.owner.original_custody_uncertain)
        self.assertEqual(observer.json.call_count, 3)
        observer.sql.assert_not_called()
        writes.assert_not_called()

    def test_active_registered_source_or_caller_refuses_before_any_sql(self):
        phase, observer, writes = self.fixture()
        scheduler = object()
        self.n._SCHEDULERS[scheduler] = phase
        self.addCleanup(self.n._SCHEDULERS.pop, scheduler, None)
        with self.assertRaises(self.n.NativeFixtureError): self.n.later_off(phase)
        observer.json.assert_not_called()
        writes.assert_not_called()


class BorrowedStdin(Guarded):
    def test_default_devnull_and_private_borrowed_stdin_share_owned_cleanup(self):
        e = self.h.source_module().evidence()
        for source in (None, io.BytesIO(b'actual-owned-command-input')):
            with self.subTest(borrowed=source is not None), contextlib.ExitStack() as scope:
                child = mock.Mock()
                child.stdout.fileno.return_value, child.stderr.fileno.return_value = 80, 81
                owned = mock.Mock()
                owned.settled.return_value = True
                owned.wait.return_value = 0
                popen = scope.enter_context(mock.patch.object(e.subprocess, 'Popen', return_value=child))
                scope.enter_context(mock.patch.object(e, 'OwnedChild', return_value=owned))
                scope.enter_context(mock.patch.object(e.os, 'set_blocking'))
                scope.enter_context(mock.patch.object(e.os, 'read', return_value=b''))
                scope.enter_context(mock.patch.object(e.select, 'select', side_effect=lambda active,*_: (active[:],[],[])))
                tick, failure = mock.Mock(), mock.Mock()
                result = e.guarded_command(['owned-command'], 15, tick, failure, stdin=source)
                self.assertEqual(result.returncode, 0)
                self.assertEqual(popen.call_args.kwargs['stdin'], e.subprocess.DEVNULL if source is None else source)
                self.assertEqual(popen.call_args.kwargs['pass_fds'], ())
                owned.close.assert_called_once()
                child.stdout.close.assert_called_once()
                child.stderr.close.assert_called_once()
                failure.assert_not_called()
                if source is not None:
                    self.assertFalse(source.closed)
                    source.close()



class ContinuationContracts(Guarded):
    def test_exact_four_aggregation_refuses_missing_duplicate_and_pending(self):
        rows=[dict(name=self.n.PREFIX+name,status='PASS_TEST_ORIGIN',cleanup='COMPLETE',
                   archive_acceptance=False,database_changed=False,elf_sha256='sha256:'+'e'*64,output_sha256='sha256:'+'f'*64) for name,_ in self.n.CASES]
        self.assertEqual(self.n.aggregate_cases(rows)['count'],4)
        for bad in (rows[:3],rows+[rows[0]],rows[:3]+[dict(rows[3],cleanup='UNKNOWN')]):
            with self.assertRaises(self.n.NativeFixtureError):self.n.aggregate_cases(bad)

    def test_source_environment_is_derived_and_cannot_inject_shell_values(self):
        owner=types.SimpleNamespace(root=Path('/var/lib/lagrange-paper-operations-fixture-a'),
            commit='a'*40,project='owned-project')
        result=self.n.source_environment_bytes(owner,{'entitlement_reference':'fixture-only:x','entitlement_sha256':'sha256:'+'c'*64},
            Path('/private/state'),'b'*32,b'actual-config')
        self.assertIn(b'KIS_READ_COORDINATION_MODE=shared_required\n',result)
        self.assertIn(b'OWNER_BETA_PAPER_MODE=disabled\n',result)
        self.assertIn(b'COMPOSE_PROJECT_NAME=owned-project\n',result)
        with self.assertRaises(self.n.NativeFixtureError):
            self.n.source_environment_bytes(owner,{'entitlement_reference':'x\nBAD=yes','entitlement_sha256':'sha256:'+'c'*64},Path('/private/state'),'b'*32,b'actual-config')

    def test_private_unit_derives_original_renderer_without_default_mutation(self):
        e=self.h.source_module().evidence()
        b=dict(release_root='/private/releases/'+'a'*40,production_env_file='/private/env',
               calendar_dir='/private/calendar',evidence_root='/private/data/evidence',
               cohort={'verifier_revision':'a'*40})
        original=e.render_service(b,'/private/binding','/private/state','/private/lock')
        owner=types.SimpleNamespace(release=Path(b['release_root']),root=Path('/private'))
        private=self.n.private_service_body(owner,b,'/private/binding','/private/state','/private/lock')
        self.assertIn('verify-owner-beta-paper-native-source.py service-run --fixture-root /private',private)
        self.assertIn('owner-beta-paper-evidence.py run --binding /private/binding',original)
        self.assertEqual(e.render_service(b,'/private/binding','/private/state','/private/lock'),original)



class ConnectedContinuation(Guarded):
    def rows(self):
        return [dict(name=self.n.PREFIX + name, status='PASS_TEST_ORIGIN', cleanup='COMPLETE',
            archive_acceptance=False, database_changed=False, elf_sha256='sha256:'+'e'*64, output_sha256='sha256:'+'f'*64) for name, _ in self.n.CASES]

    def test_actual_session_rows_drive_negative_then_positive_and_distinct_originals(self):
        phase=types.SimpleNamespace(owner=types.SimpleNamespace(observer=mock.Mock()))
        scheduler=types.SimpleNamespace(b={'cohort':{'cohort_sha256':'sha256:'+'a'*64}})
        caller=mock.Mock()
        caller.preserve_enable_original.side_effect=[Path('/actual/absent'),Path('/actual/committed')]
        for count in (1,2,3,4):
            phase.owner.observer.json.return_value=['2026-11-%02d'%(x+1) for x in range(count)]
            result=self.n.run_session_cases(phase,scheduler,caller)
            self.assertEqual(result[0],count)
        self.assertEqual(caller.source_case.call_args_list,[
            mock.call(scheduler,self.n.CASES[1][0]),mock.call(scheduler,self.n.CASES[0][0])])
        self.assertEqual(caller.preserve_enable_original.call_args_list,[
            mock.call(scheduler,commit=False),mock.call(scheduler,commit=True)])
        self.assertEqual(result[1],{'absent':'absent','committed':'committed'})
        self.assertIn('owner_beta_paper_source_runs',phase.owner.observer.json.call_args.args[0])

    def test_duplicate_or_missing_actual_session_rows_never_dispatch_native(self):
        phase=types.SimpleNamespace(owner=types.SimpleNamespace(observer=mock.Mock()))
        scheduler=types.SimpleNamespace(b={'cohort':{'cohort_sha256':'sha256:'+'a'*64}})
        caller=mock.Mock()
        for rows in ([],['2026-11-01']*3,{},None):
            phase.owner.observer.json.return_value=rows
            with self.assertRaises(self.n.NativeFixtureError):self.n.run_session_cases(phase,scheduler,caller)
        caller.source_case.assert_not_called()
        caller.preserve_enable_original.assert_not_called()

    def test_committed_original_is_not_attempted_after_absent_uncertainty(self):
        phase=types.SimpleNamespace(owner=types.SimpleNamespace(observer=mock.Mock()))
        phase.owner.observer.json.return_value=['2026-11-%02d'%x for x in range(1,5)]
        scheduler=types.SimpleNamespace(b={'cohort':{'cohort_sha256':'sha256:'+'a'*64}})
        caller=mock.Mock()
        caller.preserve_enable_original.side_effect=RuntimeError('unknown')
        with self.assertRaises(RuntimeError):self.n.run_session_cases(phase,scheduler,caller)
        caller.preserve_enable_original.assert_called_once_with(scheduler,commit=False)

    def private(self):
        p=object.__new__(self.n.PrivateService)
        p.root=Path('/var/lib/lagrange-paper-operations-fixture-test/.native-source')
        p.owner=types.SimpleNamespace(lock_fd=77,root=p.root.parent,root_id=(1,2,0,0,448),commit='a'*40,
            project='private-test',original_custody_uncertain=False,docker=mock.Mock())
        p.phase=types.SimpleNamespace(owner=p.owner,deadline=mock.Mock(),quiescent=mock.Mock(),revalidate=mock.Mock())
        p.phase.deadline.value=900000000000
        p.phase.deadline.seconds.return_value=.1
        p.g=mock.Mock()
        p.g.unit_name.return_value='lagrange-kis-witness-aaaaaaa.service'
        p.e=mock.Mock()
        p.e.sample.return_value={'monotonic_ns':100,'boottime_ns':100,'realtime_ns':100}
        p.e.digest.return_value='sha256:'+'d'*64
        p.binding={'schema':'actual-binding'}
        p.owner.docker.object.return_value={'Containers':{}}
        p.save=mock.Mock()
        return p

    def test_handoff_unlocks_before_start_and_relocks_only_after_actual_stop(self):
        p=self.private();events=[]
        value=dict(state='INSTALLED',attempts=[],backend='b'*64)
        p.load=lambda:value
        p.state=mock.Mock(side_effect=[{'ActiveState':'inactive'},
            {'Job':'0','ActiveState':'active'},
            {'Job':'0','ActiveState':'inactive','Result':'success','ExecMainStatus':'0'}])
        p.g.stopped.side_effect=lambda *_:events.append('stopped')
        p.g.ctl.side_effect=lambda *a:events.append('start')
        def arm(attempt,state):
            events.append('actual-arm')
            attempt['process']={'identity':{'pid':99,'start_ticks':1}}
            attempt['arm_sha256']='sha256:'+'f'*64
        p.observe_start=arm
        p.e.decode.side_effect=lambda raw:dict(generation=value['attempts'][-1]['generation'],
            process={'pid':99,'start_ticks':1},boot_id='boot',cleanup='COMPLETE',session='2026-11-11',
            session_count=3,cases=[])
        with mock.patch.object(Path,'read_text',return_value='boot'),mock.patch('time.sleep'),\
             mock.patch('fcntl.flock',side_effect=lambda fd,mode:events.append('unlock' if mode==8 else 'lock')):
            result=p.session()
        self.assertEqual(result['status'],'SESSION_CLOSED')
        self.assertLess(events.index('unlock'),events.index('start'))
        self.assertLess(events.index('actual-arm'),events.index('lock'))
        self.assertEqual(events[-2:],['stopped','lock'])
        p.phase.revalidate.assert_called_once_with(cleanup=True)
        self.assertEqual(value['attempts'][0]['disposition'],'CLOSED')

    def test_unknown_service_stop_never_releases_original_custody_or_reports_closed(self):
        p=self.private();value=dict(state='INSTALLED',attempts=[],backend='b'*64)
        p.load=lambda:value
        p.state=mock.Mock(side_effect=[{'ActiveState':'inactive'},RuntimeError('lost'),RuntimeError('lost')])
        with mock.patch.object(Path,'read_text',return_value='boot'),mock.patch('fcntl.flock') as flock:
            with self.assertRaisesRegex(self.n.NativeFixtureError,'NATIVE_SERVICE_CLEANUP_UNKNOWN'):p.session()
        self.assertTrue(p.owner.original_custody_uncertain)
        self.assertEqual(flock.call_args_list,[mock.call(77,8)])
        self.assertIsNone(value['attempts'][0]['result_sha256'])
        p.e.read_file.assert_not_called()

    def test_starting_null_binding_cannot_become_actual_arm(self):
        p=self.private()
        process={'identity':{'pid':90,'start_ticks':1},'boot_id':'boot','started_monotonic_us':2}
        p.process=mock.Mock(return_value=process)
        p.g.arm_observation.return_value=None
        attempt=dict(boot_id='boot',observed={'monotonic_ns':1000},process=None,arm_sha256=None)
        self.assertIsNone(p.observe_start(attempt,{}))
        self.assertIsNone(attempt['arm_sha256'])
        self.assertEqual(attempt['process'],process)
        process['started_monotonic_us']=0
        with self.assertRaisesRegex(self.n.NativeFixtureError,'NATIVE_PROCESS_GENERATION'):
            p.observe_start(attempt,{})

    def test_foreign_canonical_unit_refuses_before_install_or_network(self):
        p=self.private();p.unit=Path('/etc/systemd/system/lagrange-kis-witness-aaaaaaa.service')
        p.drop=Path(str(p.unit)+'.d');p.ledger=p.root/'service.json'
        p.g.host_units.side_effect=RuntimeError('foreign')
        with self.assertRaises(RuntimeError):p.install('/untrusted-test')
        p.e.install_witness.assert_not_called()
        p.owner.docker.object.assert_not_called()

    def test_accept_calls_actual_off_then_both_readbacks_then_retirement(self):
        phase=types.SimpleNamespace(owner=types.SimpleNamespace(observer=mock.Mock()),quiescent=mock.Mock())
        phase.owner.observer.json.return_value={'state':'ENABLED'}
        events=[]
        private=mock.Mock(root=Path('/owned'))
        private.observations.return_value=(self.rows()[:2],{'committed':Path('/original/c'),'absent':Path('/original/a')})
        private.retire.side_effect=lambda:events.append('retire')
        caller=mock.MagicMock()
        caller.__enter__.return_value=caller
        caller.__exit__.side_effect=lambda *_:events.append('caller-close')
        caller.readback_case.side_effect=lambda path,committed:events.append('committed' if committed else 'absent')
        self.patch(self.n,'PrivateService',return_value=private)
        self.patch(self.n,'NativeCaller',return_value=caller)
        self.patch(self.n,'later_off',side_effect=lambda _:events.append('off'))
        rows,used=self.n.accept(phase)
        self.assertIs(used,caller)
        self.assertEqual(events,['off','committed','absent','caller-close','retire'])

    def test_accept_missing_real_source_case_refuses_off_and_native(self):
        private=mock.Mock()
        private.observations.return_value=(self.rows()[:1],{})
        self.patch(self.n,'PrivateService',return_value=private)
        off=self.patch(self.n,'later_off')
        native=self.patch(self.n,'NativeCaller')
        with self.assertRaisesRegex(self.n.NativeFixtureError,'NATIVE_SOURCE_CASES_REQUIRED'):
            self.n.accept(object())
        off.assert_not_called();native.assert_not_called()

    def test_failed_readback_does_not_retire_or_dispose_fixture(self):
        phase=types.SimpleNamespace(owner=types.SimpleNamespace(observer=mock.Mock()),quiescent=mock.Mock())
        phase.owner.observer.json.return_value={'state':'ENABLED'}
        private=mock.Mock(root=Path('/owned'))
        private.observations.return_value=(self.rows()[:2],{'committed':Path('/c'),'absent':Path('/a')})
        caller=mock.MagicMock();caller.__enter__.return_value=caller
        caller.readback_case.side_effect=RuntimeError('unknown')
        self.patch(self.n,'PrivateService',return_value=private)
        self.patch(self.n,'NativeCaller',return_value=caller)
        self.patch(self.n,'later_off')
        with self.assertRaises(RuntimeError):self.n.accept(phase)
        private.retire.assert_not_called()

    def test_cli_exposes_owned_continuations_but_no_readiness_option(self):
        for phase in ('install','session','accept','dispose'):
            self.assertEqual(self.cli.parser().parse_args([phase,'--checkpoint','/private/c']).phase,phase)
        with contextlib.redirect_stderr(io.StringIO()),self.assertRaises(SystemExit):
            self.cli.parser().parse_args(['accept','--ready'])


class AcquisitionAndFinalization(Guarded):
    rows = ConnectedContinuation.rows
    private = ConnectedContinuation.private
    def test_native_policy_refuses_c_store_and_foreign_project(self):
        e=self.h.source_module().evidence()
        owner=types.SimpleNamespace(root=Path('/owned'),commit='a'*40,project='native-private')
        read=self.patch(e,'read_file',return_value=b'actual-policy')
        self.patch(e,'decode',return_value={'release_commit':owner.commit,'compose_project':owner.project})
        self.patch(self.f.ops.daily.policy,'validate_policy')
        self.assertEqual(self.n.native_policy_bytes(owner,owner.root/self.n.SERVICE_STORE/'policy'),b'actual-policy')
        with self.assertRaises(self.n.NativeFixtureError):self.n.native_policy_bytes(owner,owner.root/'.genuine-source/policy')
        self.assertEqual(read.call_count,1)
        e.decode.return_value['compose_project']='foreign'
        with self.assertRaises(self.n.NativeFixtureError):self.n.native_policy_bytes(owner,owner.root/self.n.SERVICE_STORE/'policy')

    def config(self):
        return dict(code_commit='a'*40,owner='11111111-1111-4111-8111-111111111111',
            policy_sha256='sha256:'+'b'*64,first_auto_session='2026-11-11',raw_owner_uid=10001,
            entitlement_reference='fixture-only:retained',entitlement_sha256='sha256:'+'c'*64)

    def test_action_supply_derives_base_frontier_and_rights_from_retained_material(self):
        import json
        c=self.config();record={'range_end':'2026-08-28','candidate_content_sha256':'sha256:'+'d'*64}
        wire=self.n.action_supply_config(c['code_commit'],c,record)
        value=json.loads(wire)
        self.assertEqual(value['base_candidate_sha256'],record['candidate_content_sha256'])
        self.assertEqual(value['initial_civil_frontier'],record['range_end'])
        self.assertEqual(value['activation_date'],c['first_auto_session'])
        self.assertEqual(value['entitlement_reference_sha256'],self.h.digest(c['entitlement_reference'].encode()))
        self.assertEqual(value['schema'],'r2-action-supply-v1')
        with self.assertRaises(self.n.NativeFixtureError):
            self.n.action_supply_config('e'*40,c,record)
        with self.assertRaises(self.n.NativeFixtureError):
            self.n.action_supply_config(c['code_commit'],dict(c,first_auto_session='2026-08-28'),record)

    def test_env_supplies_required_compose_interpolation_and_real_action_wire(self):
        c=self.config();owner=types.SimpleNamespace(root=Path('/owned'),commit=c['code_commit'],project='private')
        action=self.n.action_supply_config(owner.commit,c,{'range_end':'2026-08-28','candidate_content_sha256':'sha256:'+'d'*64})
        env=dict(line.split('=',1) for line in self.n.source_environment_bytes(owner,c,Path('/inputs/state'),'generation',action).decode().splitlines())
        import re
        required=set(re.findall(r'\$\{([A-Z0-9_]+):\?',(HERE.parents[1]/'deploy/compose/compose.yml').read_text()))
        # The existing Source wrapper supplies its inactive range-profile sentinel.
        self.assertEqual(required-set(env),{'RANGE_RAW_BATCH_ID'})
        self.assertEqual(bytes.fromhex(env['RESEARCH_R2_ACTION_SUPPLY_CONFIG_HEX']),action)
        self.assertEqual(env['OWNER_BETA_PAPER_MODE'],'disabled')
        self.assertEqual(env['OWNER_INTRADAY_QUOTES_MODE'],'off')

    def test_prepublication_acquisition_holds_originals_and_never_retrofits_context(self):
        import json
        directory=Path(self.stack.enter_context(tempfile.TemporaryDirectory()))
        inputs=directory/'inputs';inputs.mkdir()
        (inputs/'state/kis-read-coordination').mkdir(parents=True)
        (inputs/'calendar').mkdir()
        (inputs/'credential-generation').write_bytes(b'generation\n')
        for leaf in ('kis_app_key','kis_app_secret'):(inputs/leaf).write_bytes(b'fixture-secret')
        for leaf in ('calendar.json','manifest.json','overrides.json'):
            (inputs/'calendar'/leaf).write_bytes(json.dumps({'range':{'start':'2026-01-01','end':'2027-01-01'}}).encode())
        root=directory/'owner';root.mkdir();(root/self.n.SERVICE_STORE).mkdir()
        for role in ('research-worker','research-range-raw'):(root/'secrets'/role).mkdir(parents=True)
        stage=root/'stage';(stage/'deploy/compose').mkdir(parents=True);(stage/'deploy/compose/.env').write_bytes(b'old')
        owner=types.SimpleNamespace(root=root,commit='a'*40,project='private',context=None)
        p=type('PreparationDouble',(),{})();p.owner=owner;p.used=True;p.published=False;p.stack=contextlib.ExitStack()
        self.addCleanup(p.stack.close);p.runtime=types.SimpleNamespace(config=self.config())
        p.material=types.SimpleNamespace(record={'range_end':'2026-08-28','candidate_content_sha256':'sha256:'+'d'*64})
        self.n._OWNERS.add(p);self.addCleanup(self.n._OWNERS.discard,p)
        e=self.h.source_module().evidence();opened=[]
        @contextlib.contextmanager
        def open_original(path,**kw):
            opened.append((Path(path),kw))
            fd=os.open(path,os.O_RDONLY if not kw.get('write') else os.O_RDWR)
            try:yield fd
            finally:os.close(fd)
        self.patch(e,'opened',side_effect=open_original)
        self.patch(self.f,'write_new',side_effect=lambda path,raw,*_:Path(path).write_bytes(raw))
        calendar=self.h.load('native_source_calendar','scripts/ops/xkrx-calendar-bootstrap.py')
        verify=self.patch(calendar,'load_existing')
        self.n.prepare_acquisition(p,stage,inputs)
        self.assertIsNone(owner.context);self.assertFalse(hasattr(owner,'release'))
        self.assertEqual(p.acquisition['coordination_root'],str(inputs/'state'))
        self.assertIn(b'RESEARCH_R2_ACTION_SUPPLY_CONFIG_HEX=',(stage/'deploy/compose/.env').read_bytes())
        verify.assert_called_once()
        self.n.revalidate_acquisition(p)
        (inputs/'kis_app_key').write_bytes(b'changed-secret')
        with self.assertRaises(self.n.NativeFixtureError):self.n.revalidate_acquisition(p)
        owner.context=object()
        with self.assertRaises(self.n.NativeFixtureError):self.n.prepare_acquisition(p,stage,inputs)

    def test_installer_default_dry_run_remains_official_and_private_callback_is_explicit(self):
        e=self.h.source_module().evidence()
        b=dict(release_root='/private/releases/'+'a'*40,production_env_file='/private/env',
            calendar_dir='/private/calendar',evidence_root='/private/data/evidence',
            cohort={'verifier_revision':'a'*40},state_dir='/private/state',lock_file='/private/lock')
        args=types.SimpleNamespace(binding='/private/binding',release_root=b['release_root'],commit='a'*40,
            env_file=b['production_env_file'],calendar_dir=b['calendar_dir'],state_dir=b['state_dir'],
            lock_file=b['lock_file'],systemd_dir='/etc/systemd/system',mode='dry-run')
        self.patch(e,'binding',return_value=(b,b'actual-binding'))
        out=io.StringIO()
        with contextlib.redirect_stdout(out):e.install_witness(args)
        self.assertIn('owner-beta-paper-evidence.py run --binding',out.getvalue())
        render=mock.Mock(return_value='private-original\n');ctl=mock.Mock()
        with contextlib.redirect_stdout(io.StringIO()):e.install_witness(args,_render_service=render,_systemctl=ctl)
        render.assert_called_once_with(b,args.binding,args.state_dir,args.lock_file);ctl.assert_not_called()

    def test_known_arm_is_not_rechecked_as_future_at_due(self):
        p=self.private();value=dict(state='INSTALLED',attempts=[],backend='b'*64)
        p.load=lambda:value
        p.state=mock.Mock(side_effect=[{'ActiveState':'inactive'},
            {'Job':'0','ActiveState':'active'},{'Job':'0','ActiveState':'active'},
            {'Job':'0','ActiveState':'inactive','Result':'success','ExecMainStatus':'0'}])
        actual={'identity':{'pid':99,'start_ticks':1}}
        def arm(attempt,state):attempt.update(process=actual,arm_sha256='sha256:'+'f'*64)
        p.observe_start=mock.Mock(side_effect=arm);p.process=mock.Mock(return_value=actual)
        p.e.decode.side_effect=lambda raw:dict(generation=value['attempts'][-1]['generation'],process=actual['identity'],
            boot_id='boot',cleanup='COMPLETE',session='2026-11-11',session_count=1,cases=[])
        with mock.patch.object(Path,'read_text',return_value='boot'),mock.patch('time.sleep'),mock.patch('fcntl.flock'):
            p.session()
        p.observe_start.assert_called_once();p.process.assert_called_once()

    def test_network_unknown_is_durable_before_create_and_never_installs_unit(self):
        p=self.private();p.unit=Path('/etc/systemd/system/unit');p.drop=Path('/etc/systemd/system/unit.d')
        p.ledger=p.root/'service.json';p.state=mock.Mock(return_value={'LoadState':'not-found'})
        events=[];p.save=mock.Mock(side_effect=lambda value:events.append(('saved',value['backend'])))
        def output(argv):
            if argv[:2]==['network','ls']:return ''
            events.append(('create',None));raise RuntimeError('ack-lost')
        p.owner.output=output
        self.patch(self.n,'copy_test_elf')
        with mock.patch.object(Path,'mkdir'):
            with self.assertRaises(RuntimeError):p.install('/actual-test')
        self.assertEqual(events,[('saved',None),('create',None)])
        self.assertTrue(p.owner.original_custody_uncertain);p.e.install_witness.assert_not_called()

    def test_final_cli_joins_real_accept_and_aggregation_after_disposal(self):
        events=[]
        phase=mock.MagicMock();phase.__enter__.return_value=phase
        phase.__exit__.side_effect=lambda *_:events.append('phase-close')
        phase.owner.observer.json.return_value={'state':'ENABLED'}
        phase.owner.cleanup={}
        private=mock.Mock(root=Path('/owned'))
        private.observations.return_value=(self.rows()[:2],{'committed':Path('/c'),'absent':Path('/a')})
        private.retire.side_effect=lambda:events.append('retire')
        caller=mock.MagicMock();caller.__enter__.return_value=caller
        caller.readback_case.side_effect=lambda *a,**kw:events.append('native-readback')
        caller.__exit__.side_effect=lambda *_:events.append('native-close')
        caller.report.side_effect=lambda:events.append('report') or self.rows()[2:]
        self.patch(self.n,'OwnedPhase',return_value=phase)
        self.patch(self.n,'PrivateService',return_value=private)
        self.patch(self.n,'NativeCaller',return_value=caller)
        self.patch(self.n,'later_off',side_effect=lambda _:events.append('off'))
        def dispose(value):
            events.append('dispose');value.owner.cleanup={'containers':'REMOVED','inputs':'REMOVED_AFTER_DISPOSITION'}
        self.patch(self.n,'dispose_native_phase',side_effect=dispose)
        self.patch(self.n,'NativePolicy',side_effect=lambda *a:contextlib.nullcontext())
        for name in ('termination_cleanup','bounded_run'):
            self.patch(self.f,name,side_effect=lambda *a:contextlib.nullcontext())
        result=self.n.continue_phase('/source','a'*40,b'manifest',object(),'/owned/checkpoint','accept')
        self.assertEqual(result['count'],4);self.assertEqual(result['fixture_disposition'],'REMOVED')
        self.assertEqual(events,['off','native-readback','native-readback','native-close','retire','dispose','phase-close','report'])
        phase.owner.cleanup={}
        self.n.dispose_native_phase.side_effect=RuntimeError('unknown-disposal')
        caller.report.reset_mock()
        with self.assertRaises(RuntimeError):
            self.n.continue_phase('/source','a'*40,b'manifest',object(),'/owned/checkpoint','accept')
        caller.report.assert_not_called()



class NativeNamespace(Guarded):
    def setup_policy(self):
        e=self.h.source_module().evidence()
        library=mock.Mock();library.unshare.return_value=0;library.setns.return_value=0
        state={'mounted':False}
        def mount(source,target,*args):
            if source is not None:state['mounted']=True
            return 0
        library.mount.side_effect=mount
        def unmount(*_):state['mounted']=False;return 0
        library.umount2.side_effect=unmount
        self.patch(self.f.ctypes,'CDLL',return_value=library)
        self.patch(self.n.os,'geteuid',return_value=0)
        self.patch(self.f.threading,'active_count',return_value=1)
        self.patch(self.n.os,'open',side_effect=lambda path,*a,**kw:20 if str(path).endswith('service-status') else 22 if str(path)=='/private/policy' else 21)
        self.patch(self.n.os,'fstat',side_effect=lambda fd:types.SimpleNamespace(st_dev=1,st_ino={10:1,11:2,20:3,22:4}.get(fd,5),st_uid=0,st_gid=0,st_mode=0o40700))
        closed=self.patch(self.n.os,'close')
        self.patch(e,'no_acl')
        @contextlib.contextmanager
        def opened(path,**kw):yield 10 if Path(path)==self.n.POLICY_TARGET.parent else 11
        self.patch(e,'opened',side_effect=opened)
        def identity(path):
            path=Path(path)
            if path==self.n.POLICY_TARGET:return (1,3) if state['mounted'] else (1,2)
            if path==self.n.POLICY_TARGET.parent:return (1,1)
            if path==Path('/private/policy'):return (1,4)
            return (1,3)
        self.patch(self.f.faults,'directory_identity',side_effect=identity)
        scope=self.n.NativePolicy(Path('/private/policy'))
        self.addCleanup(self.f._UNRESOLVED_POLICY_SCOPES.discard,scope)
        return scope,library,closed

    def test_exact_policy_directory_projection_preserves_original_data_namespace(self):
        policy,library,closed=self.setup_policy()
        with policy:
            self.assertTrue(policy.bound)
            self.assertEqual(self.f.faults.directory_identity('/var/lib/lagrange'),(1,1))
        calls=library.mount.call_args_list
        self.assertEqual(calls[1].args[0:2],(b'/private/policy/service-status',b'/var/lib/lagrange/service-status'))
        self.assertEqual(calls[2].args[3],4096|32|1)
        library.umount2.assert_called_once_with(b'/var/lib/lagrange/service-status',0)
        library.setns.assert_called_once_with(21,0x20000)
        self.assertEqual(closed.call_args_list,[mock.call(21),mock.call(20),mock.call(22)])
        actual_paths=('/var/lib/lagrange/data/artifacts/historical-price-beta-root',
            '/var/lib/lagrange/data/raw','/var/lib/lagrange/daily/config.json',
            '/var/lib/lagrange/runtime/kis-read-coordination')
        for path in actual_paths:self.assertFalse(Path(path).is_relative_to(self.n.POLICY_TARGET))

    def test_failed_bind_restores_actual_namespace_without_unmounting_host_policy(self):
        policy,library,closed=self.setup_policy()
        library.mount.side_effect=[0,-1]
        with self.assertRaisesRegex(self.n.NativeFixtureError,'NATIVE_POLICY_BIND'):policy.__enter__()
        library.umount2.assert_not_called();library.setns.assert_called_once_with(21,0x20000)
        self.assertEqual(closed.call_count,3)

    def test_failed_unmount_is_unknown_and_keeps_owned_descriptors(self):
        policy,library,closed=self.setup_policy();policy.__enter__()
        library.umount2.side_effect=None;library.umount2.return_value=-1
        with self.assertRaisesRegex(self.n.NativeFixtureError,'NATIVE_POLICY_UNMOUNT'):policy.__exit__(None,None,None)
        closed.assert_not_called();library.setns.assert_not_called()
        self.assertTrue(policy.bound)
        policy.stack.close()

    def test_native_cli_and_service_use_same_narrow_projection_only(self):
        owner=types.SimpleNamespace(root=Path('/owned'))
        policy=owner.root/self.n.SERVICE_STORE/'policy'
        body=self.n.native_drop_body(owner,policy)
        self.assertEqual(body,b'[Service]\nBindReadOnlyPaths=/owned/.native-source/policy/service-status:/var/lib/lagrange/service-status\n')
        ordinary=self.h.source_module().drop_body(policy)
        self.assertEqual(ordinary,body)
        cli=(HERE/'verify-owner-beta-paper-native-source.py').read_text()
        module=(HERE/'lib/owner-beta-paper-native-source.py').read_text()
        self.assertIn('native.NativePolicy(policy)',cli)
        self.assertNotIn('f.PrivatePolicy(',cli)
        self.assertIn('f.bounded_run(deadline), NativePolicy(policy)',module)
        self.assertIn('f.service_status_namespace(self.policy, pid)',module)
        self.assertTrue(issubclass(self.n.NativePolicy,self.f.ServiceStatusScope))
        self.assertNotIn('self.g.drop_body(',module)

    def test_exact_four_rejects_mixed_libtest_generations(self):
        rows=ConnectedContinuation.rows(self)
        rows[3]['elf_sha256']='sha256:'+'a'*64
        with self.assertRaisesRegex(self.n.NativeFixtureError,'NATIVE_TEST_ELF_CHANGED'):
            self.n.aggregate_cases(rows)

    def test_stopped_daily_containers_close_before_source_link_and_native_call(self):
        tree=ast.parse((HERE/'lib/owner-beta-paper-native-source.py').read_text())
        phase=next(n for n in tree.body if isinstance(n,ast.ClassDef) and n.name=='OwnedPhase')
        session=next(n for n in phase.body if isinstance(n,ast.FunctionDef) and n.name=='session')
        scheduler=next(n for n in ast.walk(session) if isinstance(n,ast.ClassDef) and n.name=='NativeScheduler')
        finish=next(n for n in scheduler.body if isinstance(n,ast.FunctionDef) and n.name=='source_finished')
        attempt=finish.body[0]
        self.assertIsInstance(attempt,ast.Try)
        self.assertEqual(ast.unparse(attempt.body[0]),'h.daily_produce(runtime)')
        self.assertEqual(ast.unparse(attempt.finalbody[0]),'runtime.close()')
        self.assertEqual(ast.unparse(finish.body[1]),'inner.guard()')



class InitialConfigureAndDisposal(Guarded):
    def configure_fixture(self):
        events=[]
        phase=object.__new__(self.n.OwnedPhase);phase.revalidate=mock.Mock()
        phase.deadline=types.SimpleNamespace(value=1000)
        phase.facts={'strategy_config_id':'actual-trend','generic_dataset':{'id':'actual-generic'}}
        phase.owner=types.SimpleNamespace(observer=mock.Mock(),root=Path('/owned'),release=Path('/owned/release'),
            lock_fd=77,contexts=[],context_modules={})
        phase.owner.observer.json.return_value=[]
        scheduler=mock.Mock();scheduler.b={'cohort':{'strategy_config_id':'actual-trend'}}
        scheduler.j.events=[{'arm':{'parent':{'pid':42,'start_ticks':7}}}]
        self.n._SCHEDULERS[scheduler]=phase;self.addCleanup(self.n._SCHEDULERS.pop,scheduler,None)
        runtime=types.SimpleNamespace(owner=phase.owner,start_api=lambda _:events.append('api'),close=lambda:events.append('runtime-close'))
        e=self.h.source_module().evidence();self.patch(e,'process',return_value={'pid':42,'start_ticks':7})
        self.patch(e,'verify_chain',side_effect=lambda *a:events.append('actual-armed-chain'))
        l=self.h.lifecycle_module();controller=mock.MagicMock();controller.__enter__.return_value=controller
        def configured(*args):
            events.append('configure')
            return l.LifecycleResult('OK',None,self.h.canonical({'status':'COMMITTED','current_state':'OFF','current_epoch':0}),None,'COMPLETE')
        controller.configure.side_effect=configured
        self.patch(l,'open_lifecycle',return_value=controller)
        self.patch(self.h,'lifecycle_context',return_value=object())
        self.patch(self.f,'existing_json_macro',return_value={'actual':'contract'})
        self.patch(self.h,'consent',side_effect=lambda *a,**kw:events.append('consent'))
        phase.source_barrier=mock.Mock(side_effect=lambda _:events.append('actual-native-barrier'))
        return phase,scheduler,runtime,controller,events

    def test_initial_arm_precedes_real_configure_consent_and_native_barrier(self):
        phase,scheduler,runtime,controller,events=self.configure_fixture()
        phase.configure_after_arm(scheduler,runtime)
        self.assertEqual(events,['actual-armed-chain','api','configure','consent','actual-native-barrier','runtime-close'])
        proposal=self.h.fixture_module().ops._json(controller.configure.call_args.args[1],65536)
        self.assertEqual(proposal['strategy_config_id'],'actual-trend')
        self.assertNotIn('account_id',proposal)

    def test_configured_session_cannot_reconfigure_or_change_source_cohort(self):
        phase,scheduler,runtime,controller,events=self.configure_fixture()
        phase.owner.observer.json.return_value=[{'strategy_config_id':'actual-trend'}]
        phase.configure_after_arm(scheduler,runtime)
        self.assertEqual(events,['actual-armed-chain']);controller.configure.assert_not_called()
        phase.owner.observer.json.return_value=[{'strategy_config_id':'foreign'}]
        with self.assertRaises(self.n.NativeFixtureError):phase.configure_after_arm(scheduler,runtime)
        controller.configure.assert_not_called()

    def test_unknown_configure_never_observes_reset_and_closes_runtime(self):
        phase,scheduler,runtime,controller,events=self.configure_fixture()
        controller.configure.side_effect=RuntimeError('unknown')
        with self.assertRaises(RuntimeError):phase.configure_after_arm(scheduler,runtime)
        self.assertEqual(events,['actual-armed-chain','api','runtime-close'])
        phase.source_barrier.assert_not_called();self.h.consent.assert_not_called()

    def disposal_fixture(self):
        import fcntl
        events=[]
        phase=type('DisposalDouble',(),{})();phase.stack=contextlib.ExitStack()
        self.addCleanup(phase.stack.close);phase.closed=False;phase.quiescent=mock.Mock();phase.revalidate=mock.Mock()
        phase.stack.callback(lambda:events.append('originals-closed'))
        phase.owner=types.SimpleNamespace(root=Path('/owned'),root_id=(1,2),commit='a'*40,
            lock_fd=77,deadline=mock.Mock(),cleanup={},original_custody_uncertain=False)
        self.n._PHASES.add(phase);self.addCleanup(self.n._PHASES.discard,phase)
        self.addCleanup(self.n._UNRESOLVED_PHASES.discard,phase)
        g=self.h.source_module();e=g.evidence()
        for name in ('absent','host_units','provider_idle'):self.patch(g,name)
        b={'lock_file':'/owned/collector.lock','evidence_root':'/owned/evidence'}
        self.patch(e,'binding',return_value=(b,b'actual-binding'))
        self.patch(e,'read_file',return_value=self.h.canonical({'state':'RETIRED','root':[1,2]}))
        self.patch(e,'validate_current')
        self.patch(self.n.os,'pread',return_value=self.h.canonical({'status':'INVALID','binding':b}))
        @contextlib.contextmanager
        def opened(*a,**kw):yield 70
        self.patch(e,'opened',side_effect=opened)
        self.stack.enter_context(mock.patch('fcntl.flock'))
        lock=self.stack.enter_context(mock.patch('fcntl.fcntl',return_value=bytes(e.OfdFlock(fcntl.F_UNLCK,0,0,0,0))))
        self.patch(self.n.os.path,'lexists',return_value=False)
        def dispose(close):
            events.append('backend-and-container-disposition')
            phase.owner.cleanup['containers']='REMOVED'
            close();events.append('tree-removal')
            phase.owner.cleanup['inputs']='REMOVED_AFTER_DISPOSITION'
        phase.owner.dispose=mock.Mock(side_effect=dispose)
        return phase,events,lock

    def test_actual_disposal_method_closes_originals_only_after_owned_backend_disposition(self):
        phase,events,_=self.disposal_fixture()
        self.n.dispose_native_phase(phase)
        self.assertEqual(events,['backend-and-container-disposition','originals-closed','tree-removal'])
        self.assertTrue(phase.closed);self.assertIsNone(phase.owner.lock_fd)
        self.assertNotIn(phase,self.n._PHASES)

    def test_unknown_disposal_retains_originals_and_never_marks_phase_closed(self):
        phase,events,_=self.disposal_fixture()
        phase.owner.dispose.side_effect=RuntimeError('unknown-backend')
        with self.assertRaises(RuntimeError):self.n.dispose_native_phase(phase)
        self.assertEqual(events,[]);self.assertFalse(phase.closed)
        self.assertTrue(phase.owner.original_custody_uncertain)
        self.assertIn(phase,self.n._UNRESOLVED_PHASES)

    def test_held_actual_publisher_fence_refuses_disposal(self):
        import fcntl
        phase,events,lock=self.disposal_fixture()
        e=self.h.source_module().evidence()
        lock.return_value=bytes(e.OfdFlock(fcntl.F_RDLCK,0,0,0,0))
        with self.assertRaisesRegex(self.n.NativeFixtureError,'NATIVE_SOURCE_FENCE_HELD'):
            self.n.dispose_native_phase(phase)
        phase.owner.dispose.assert_not_called();self.assertEqual(events,[])

if __name__ == '__main__':
    unittest.main()
