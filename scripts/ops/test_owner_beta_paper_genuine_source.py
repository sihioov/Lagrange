"""Effects-mocked caller tests, never genuine Source/runtime acceptance.

Real processes, ownership changes, sockets, pidfds and flock are forbidden unless
an individual test explicitly replaces the operation with its own mock.
"""
import contextlib
import copy
import importlib.util
import io
import json
import os
from pathlib import Path
import stat
import sys
import tempfile
import time
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
        for target in ('subprocess.Popen', 'subprocess.run', 'os.chown', 'os.fchown',
                       'socket.socket', 'os.pidfd_open', 'fcntl.flock', 'fcntl.fcntl'):
            cls.guards.enter_context(mock.patch(target, side_effect=AssertionError('UNPLANNED_EFFECT')))
        cls.s = load('genuine_service_subject', HERE / 'lib/owner-beta-paper-genuine-source.py')
        cls.h = cls.s.history()
        cls.f = cls.h.fixture_module()
        cls.e = cls.s.evidence()
        cls.cli = load('genuine_service_cli', HERE / 'verify-owner-beta-paper-genuine-archive.py')

    @classmethod
    def tearDownClass(cls):
        cls.guards.close()

    def setUp(self):
        self.stack = contextlib.ExitStack()
        self.addCleanup(self.stack.close)
        self.o = object.__new__(self.f.FixtureOwner)
        self.o.root = Path('/var/lib/lagrange-paper-operations-fixture-unit')
        self.o.root_id = (1, 2)
        self.o.commit = 'a' * 40
        self.o.project = 'private-fixture'
        self.o.release = self.o.root / 'releases' / self.o.commit
        self.o.deadline = types.SimpleNamespace(value=time.monotonic_ns() + 300 * 10**9,
            seconds=mock.Mock(return_value=10))
        self.o.lock_fd = 19
        self.o.original_custody_uncertain = False
        self.o.contexts = []
        self.o.context = types.SimpleNamespace(values={'POSTGRES_DB': 'lagrange'}, overlays=[])
        self.o.helpers = self.o.helper_bindings = {}
        self.o.threads = self.o.blockers = self.o.tracees = []
        self.o.stop_writer = None
        self.o.dispose = mock.Mock()
        self.b = {'state_dir': str(self.o.root / 'state'), 'lock_file': str(self.o.root / 'state/lock'),
                  'evidence_root': str(self.o.root / 'data/owner-beta-paper-evidence')}
        self.r = dict(schema='private-genuine-source-v1', commit=self.o.commit, root=list(self.o.root_id),
            phase='INSTALLED', assets={}, process=None, attempt=None,
            binding_original={'path': '/protected/binding'})

    def patch(self, name, **kwargs):
        return self.stack.enter_context(mock.patch.object(self.s, name, **kwargs))

    def orchestration(self, *, phase='INSTALLED', active=False):
        events = []
        self.r['phase'] = phase
        self.patch('owned')
        self.patch('owner_snapshot', side_effect=lambda _: events.append('snapshot') or {'lock': [1, 2]})
        self.patch('load_receipt', side_effect=lambda _: (copy.deepcopy(self.r), self.b))
        self.patch('host_units')
        self.patch('no_borrowers', side_effect=lambda _: events.append('borrowers-closed'))
        self.patch('provider_idle', side_effect=lambda _: events.append('provider-idle'))
        self.patch('save', side_effect=lambda _, r: (self.r.update(copy.deepcopy(r)), events.append('save:' + r['phase'])))
        self.patch('stopped', side_effect=lambda _, st: self.s.require(st['ActiveState'] in ('inactive', 'failed'), 'SOURCE_NOT_STOPPED'))
        self.stack.enter_context(mock.patch.object(self.s.fcntl, 'flock', side_effect=lambda fd, flag: events.append('unlock' if flag == self.s.fcntl.LOCK_UN else 'lock')))
        self.patch('ctl', side_effect=lambda _, *args: events.append('ctl:' + ' '.join(args)) or '')
        self.patch('reacquire', side_effect=lambda *_: events.append('reacquire'))
        self.patch('unit_state', return_value={'ActiveState': 'active' if active else 'inactive'})
        return events


class EntryAndOwnership(Guarded):
    def test_explicit_cli_source_phase_and_connected_owner(self):
        args = self.cli.parser().parse_args(['--source-root', '/source', '--commit', self.o.commit,
            '--manifest-file', '/manifest', '--deadline-monotonic-ns', '123', 'source-service',
            '--checkpoint', '/checkpoint', '--action', 'install', '--binding-file', '/protected/binding'])
        with mock.patch.object(self.h, 'prepared_release') as prepared, \
             mock.patch.object(self.h, 'source_module', return_value=self.s), \
             mock.patch.object(self.s, 'run_source_service', return_value={'status': 'INSTALLED_DISABLED'}) as service:
            result = self.h.retained_phase(self.o, {}, args, '/policy')
        prepared.assert_called_once_with(self.o)
        service.assert_called_once_with(self.o, args.binding_file, '/policy', 'install')
        self.assertEqual(result['status'], 'INSTALLED_DISABLED')

    def test_serialized_owner_is_refused(self):
        with self.assertRaisesRegex(self.s.SourceCustodyUnknown, 'SOURCE_OWNER'):
            self.s.owned(types.SimpleNamespace())

    def test_original_deadline_is_not_extended_or_serialized(self):
        for delta in (-1, self.s.MAX_NS + 10**9):
            self.o.deadline.value = time.monotonic_ns() + delta
            with self.subTest(delta=delta), self.assertRaisesRegex(Exception, 'SOURCE_DEADLINE'):
                self.s.tick(self.o)
        self.o.deadline.value = time.monotonic_ns() + 300 * 10**9
        self.s.tick(self.o, 0.1)
        self.o.deadline.seconds.assert_called_with(0.1, cleanup=True)

    def test_live_context_or_release_lock_duplicate_refuses_unlock(self):
        self.o.contexts = [types.SimpleNamespace(_closed=False)]
        with self.assertRaisesRegex(Exception, 'SOURCE_BORROWERS_OPEN'):
            self.s.no_borrowers(self.o)
        self.o.contexts = []
        fake = types.SimpleNamespace(st_dev=1, st_ino=2, st_uid=0, st_gid=0, st_mode=0o100600)
        with mock.patch.object(self.s.os, 'fstat', return_value=fake), \
             mock.patch.object(self.s.os, 'listdir', return_value=['19', '20']):
            with self.assertRaisesRegex(Exception, 'SOURCE_LOCK_DUPLICATE'):
                self.s.no_borrowers(self.o)

    def test_install_refuses_same_candidate_foreign_before_any_write(self):
        self.patch('owned')
        self.patch('owner_snapshot')
        self.patch('host_units')
        absent = self.patch('absent', side_effect=self.s.SourceCustodyUnknown('SOURCE_FOREIGN_UNIT'))
        write = self.patch('write_new')
        cmd = self.patch('command')
        with self.assertRaisesRegex(Exception, 'SOURCE_FOREIGN_UNIT'):
            self.s.run_source_service(self.o, '/protected/binding', '/policy', 'install')
        self.assertEqual(absent.call_args.args[0], self.s.paths(self.o)[1])
        write.assert_not_called()
        cmd.assert_not_called()

    def test_host_observer_foreign_guard_is_preserved(self):
        obs = mock.Mock()
        obs.units_now.side_effect = RuntimeError('PAPER_OPS_LAUNCH_AMBIGUOUS')
        with mock.patch.object(self.f.ops, '_HostStopObserver', return_value=obs):
            with self.assertRaisesRegex(Exception, 'PAPER_OPS_LAUNCH_AMBIGUOUS'):
                self.s.host_units(self.o)
        obs.close.assert_called_once()

    def test_foreign_unit_and_pending_job_refuse(self):
        for units, jobs in (({'lagrange-kis-witness-aaaaaaa.service': {}}, ''), ({}, '9 lagrange-kis-daily-aaaaaaa.service start waiting')):
            obs = mock.Mock()
            obs.units_now.return_value = units
            with mock.patch.object(self.f.ops, '_HostStopObserver', return_value=obs), \
                 mock.patch.object(self.s, 'ctl', return_value=jobs), self.assertRaises(self.s.SourceCustodyUnknown):
                self.s.host_units(self.o)
            obs.close.assert_called_once()

    def test_owner_snapshot_rereads_release_journal_account_context_and_db(self):
        self.patch('owned')
        self.patch('original', return_value={'identity': 'release-generation'})
        self.o.account = 'actual-account'
        self.o.manifest = b'actual-manifest'
        self.o.prepared_process_sha256 = 'actual-prepared-process'
        self.o.release_hashes, self.o.producer_hashes = {'release': 'original'}, {'producer': 'original'}
        self.o.observer = types.SimpleNamespace(generation=['actual-db'], target=mock.Mock(),
            json=mock.Mock(return_value='actual-account'))
        journal = types.SimpleNamespace(installation_id='original-installation', epoch_floor=3, revalidate=mock.Mock())
        checked = types.SimpleNamespace(process_sha256='actual-prepared-process', revalidate=mock.Mock())
        info = types.SimpleNamespace(st_dev=1, st_ino=2, st_uid=0, st_gid=0, st_mode=0o40700, st_nlink=1)
        with contextlib.ExitStack() as patches:
            patches.enter_context(mock.patch.object(self.f.ops.journal, 'open_journal', return_value=contextlib.nullcontext(journal)))
            patches.enter_context(mock.patch.object(self.f.ops.release, 'verify', return_value=contextlib.nullcontext(checked)))
            descriptors = patches.enter_context(mock.patch.object(self.h, 'release_descriptors',
                return_value=(self.o.release_hashes, self.o.producer_hashes)))
            current = patches.enter_context(mock.patch.object(self.f.ops.daily.policy, 'validate_current_release'))
            patches.enter_context(mock.patch.object(self.f.ops.daily.install, 'read_root_file', return_value=self.o.manifest))
            patches.enter_context(mock.patch.object(self.f.ops.daily, 'read_context', return_value=(self.o.context.values, [])))
            patches.enter_context(mock.patch.object(self.s.os, 'fstat', return_value=info))
            patches.enter_context(mock.patch.object(self.s.os, 'stat', return_value=info))
            actual = self.s.owner_snapshot(self.o)
            self.assertEqual(actual['installation'], 'original-installation')
            self.assertEqual(actual['floor'], 3)
            self.assertEqual(actual['db'], ['actual-db'])
            current.assert_called_once_with(str(self.o.root), str(self.o.release), self.o.commit)
            descriptors.assert_called_once_with(self.o, checked)
            self.o.observer.json.return_value = 'foreign-account'
            with self.assertRaisesRegex(Exception, 'SOURCE_ACCOUNT_CHANGED'):
                self.s.owner_snapshot(self.o)


class Installation(Guarded):
    def test_exact_installer_and_one_persistent_policy_dropin_disabled_only(self):
        directory = self.stack.enter_context(tempfile.TemporaryDirectory())
        self.o.root = Path(directory)
        self.o.release = self.o.root / 'releases' / self.o.commit
        systemd = self.o.root / 'systemd'
        systemd.mkdir()
        self.stack.enter_context(mock.patch.object(self.s, 'SYSTEMD', systemd))
        root, unit, drop, policy, binding = self.s.paths(self.o)
        self.patch('owned')
        self.patch('owner_snapshot')
        self.patch('host_units')
        self.patch('unit_state', return_value={'LoadState': 'not-found', 'ActiveState': 'inactive'})
        self.patch('provider_idle')
        self.patch('stopped')
        self.patch('binding_owner', return_value=(dict(self.b, production_env_file='/private/env', calendar_dir='/private/calendar'), b'binding'))
        self.patch('policy_bytes', return_value=b'private-policy')
        self.patch('original', return_value={'identity': [1, 2, 0, 0, 0o700]})
        self.patch('assets', return_value={'originals': 'actual-descriptors-in-runtime'})
        self.patch('load_receipt')
        self.patch('absent', side_effect=lambda p: self.assertFalse(os.path.lexists(p)))
        writes = []
        def write(path, raw, mode=0o600):
            self.assertFalse(path.exists())
            path.write_bytes(raw)
            path.chmod(mode)
            writes.append((path, raw, mode))
        self.patch('write_new', side_effect=write)
        receipts = []
        self.patch('save', side_effect=lambda _, value: receipts.append(copy.deepcopy(value)))
        rendered = b'[Service]\nExecStart=/unchanged\nProtectSystem=strict\n'
        self.stack.enter_context(mock.patch.object(self.e, 'render_service', return_value=rendered.decode()))
        self.stack.enter_context(mock.patch.object(self.e, 'read_file', return_value=rendered))
        commands = []
        self.patch('command', side_effect=lambda _, argv: commands.append(argv) or unit.write_bytes(rendered))
        self.patch('ctl', side_effect=lambda _, *args: commands.append(list(args)) or '')
        # Real parser -> CLI -> history.run -> retained FixtureOwner -> Source
        # install. Only filesystem/namespace/process/DB observations are mocked.
        phase_policy = self.o.root / 'invocation-policy'
        phase_policy.mkdir()
        self.o.extractions = mock.MagicMock()
        self.o.observer = types.SimpleNamespace(target=mock.Mock())
        argv_cli = ['--source-root', '/source', '--commit', self.o.commit, '--manifest-file', '/manifest',
            '--deadline-monotonic-ns', str(self.o.deadline.value), 'source-service', '--checkpoint', '/checkpoint',
            '--action', 'install', '--binding-file', str(self.o.root / 'original-binding')]
        real_close = self.h.os.close
        with contextlib.ExitStack() as patches:
            patches.enter_context(mock.patch.object(self.cli, 'history_module', return_value=self.h))
            patches.enter_context(mock.patch.object(self.h, 'source_module', return_value=self.s))
            patches.enter_context(mock.patch.object(self.h, 'prepared_release'))
            patches.enter_context(mock.patch.object(self.f, 'validated_inputs', return_value=(Path('/source'), b'manifest', self.o.deadline)))
            patches.enter_context(mock.patch.object(self.h, 'resume_owner', return_value=(self.o, {})))
            patches.enter_context(mock.patch.object(self.f, 'bounded_run', return_value=contextlib.nullcontext()))
            patches.enter_context(mock.patch.object(self.f, 'ServiceStatusScope', return_value=contextlib.nullcontext()))
            patches.enter_context(mock.patch.object(self.f, 'termination_cleanup', return_value=contextlib.nullcontext()))
            patches.enter_context(mock.patch.object(self.h.tempfile, 'mkdtemp', return_value=str(phase_policy)))
            patches.enter_context(mock.patch.object(self.h.os, 'geteuid', return_value=0))
            patches.enter_context(mock.patch.object(self.h.os, 'getegid', return_value=0))
            patches.enter_context(mock.patch.object(self.h.os, 'close', side_effect=lambda fd: None if fd == 19 else real_close(fd)))
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                self.assertEqual(self.cli.main(argv_cli), 0)
            result = json.loads(output.getvalue())
        argv = commands[0]
        self.assertEqual(argv[:3], ['/bin/bash', str(self.o.release / 'scripts/ops/install-kis-daily.sh'), '--install-disabled'])
        self.assertEqual(argv[3:7], ['--raw-source', '--action-supply', '--witness-binding', str(binding)])
        self.assertNotIn('--apply', argv)
        self.assertNotIn('enable', sum(commands, []))
        self.assertNotIn('start', sum(commands, []))
        self.assertEqual((drop / 'private-policy.conf').read_bytes(),
            ('[Service]\nBindReadOnlyPaths=' + str(policy / 'service-status') + ':/var/lib/lagrange/service-status\n').encode())
        self.assertEqual(unit.read_bytes(), rendered)
        self.assertEqual([r['phase'] for r in receipts], ['INSTALLING', 'INSTALLED'])
        self.assertNotIn('deadline', receipts[-1])
        self.assertEqual(result['status'], 'INSTALLED_DISABLED')
        self.assertTrue(policy.exists(), 'persistent policy must survive a command return')
        self.assertFalse(phase_policy.exists(), 'CLI policy namespace is restored and removed')
        self.assertIsNone(self.o.lock_fd)
        self.o.dispose.assert_not_called()

    def state(self):
        root, unit, drop, policy, binding = self.s.paths(self.o)
        st = {k: '' for k in self.s.PROPERTIES}
        st.update(Id=self.s.unit_name(self.o), LoadState='loaded', FragmentPath=str(unit),
            DropInPaths=str(drop / 'private-policy.conf'), MainPID='0', ActiveState='inactive',
            SubState='dead', UnitFileState='disabled', NeedDaemonReload='no', Job='0',
            ExecStart='{ path=/usr/bin/python3 ; argv[]=/usr/bin/python3 ' +
                str(self.o.release / 'scripts/ops/lib/owner-beta-paper-evidence.py') + ' run --binding ' + str(binding) + ' ; }')
        return st

    def test_actual_systemctl_show_contract_and_mutations(self):
        st = self.state()
        ctl = self.patch('ctl', side_effect=lambda *_: '\n'.join(k + '=' + v for k, v in st.items()))
        self.assertEqual(self.s.unit_state(self.o), st)
        self.assertIn('ExecStart', ctl.call_args.args[3])
        for key, value in (('DropInPaths', st['DropInPaths'] + ' /foreign/override.conf'),
            ('ExecStart', st['ExecStart'].replace('/usr/bin/python3 ', '/usr/bin/foreign ', 1)),
            ('FragmentPath', '/run/systemd/transient/foreign.service'), ('Job', '123'),
            ('UnitFileState', 'enabled'), ('NeedDaemonReload', 'yes'), ('TriggeredBy', 'foreign.timer')):
            old = st[key]
            st[key] = value
            with self.subTest(key=key), self.assertRaises(self.s.SourceCustodyUnknown):
                self.s.unit_state(self.o)
            st[key] = old

    def test_file_metadata_content_and_membership_are_part_of_original(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'unit'
            path.write_bytes(b'one')
            @contextlib.contextmanager
            def opened(p, **kwargs):
                fd = os.open(p, os.O_RDONLY | (os.O_DIRECTORY if kwargs.get('directory') else 0))
                try:
                    yield fd
                finally:
                    os.close(fd)
            with mock.patch.object(self.e, 'opened', side_effect=opened):
                first = self.s.original(path, 0o600)
                path.write_bytes(b'two')
                self.assertNotEqual(self.s.original(path, 0o600), first)
                changed = self.s.original(path, 0o600)
                path.chmod(0o400)
                self.assertNotEqual(self.s.original(path, 0o400), changed)
                self.s.original(Path(directory), 0o700, directory=True, members=['unit'])
                with self.assertRaisesRegex(Exception, 'SOURCE_MEMBERS_CHANGED'):
                    self.s.original(Path(directory), 0o700, directory=True, members=[])

    def test_receipt_assets_mutation_fails_before_binding_consumption(self):
        for field in ('identity', 'sha256', 'mtime', 'members'):
            receipt = dict(self.r, assets={'file': {field: 'old'}}, binding_original={})
            with mock.patch.object(self.e, 'read_file', return_value=self.e.canonical(receipt)), \
                 mock.patch.object(self.s, 'assets', return_value={'file': {field: 'new'}}), \
                 mock.patch.object(self.s, 'binding_owner') as binding:
                with self.subTest(field=field), self.assertRaisesRegex(Exception, 'SOURCE_ASSETS_CHANGED'):
                    self.s.load_receipt(self.o)
                binding.assert_not_called()

    def test_policy_copy_uses_actual_private_mount_and_refuses_host_alias(self):
        self.o.project = 'fixture'
        raw = self.e.canonical(dict(release_commit=self.o.commit, compose_project='fixture'))
        info = types.SimpleNamespace(st_dev=1, st_ino=2, st_uid=0, st_gid=0, st_mode=0o40700, st_nlink=1)
        different = types.SimpleNamespace(st_dev=1, st_ino=3, st_uid=0, st_gid=0, st_mode=0o40700, st_nlink=1)
        with mock.patch.object(self.s.os, 'stat', return_value=info) as stat_call, \
             mock.patch.object(Path, 'read_text', return_value='9 1 0:9 /owned /var/lib/lagrange/service-status ro - ext4 /dev/private rw\n'), \
             mock.patch.object(self.e, 'read_file', return_value=raw) as read, \
             mock.patch.object(self.f.ops.daily.policy, 'validate_policy') as validate:
            self.assertEqual(self.s.policy_bytes(self.o, '/var/tmp/lagrange-paper-genuine-policy-unit'), raw)
            read.assert_called_once_with(Path('/var/lib/lagrange/service-status/policy.json'), self.s.LIMIT, exact=(0, 0, 0o600))
            validate.assert_called_once()
            stat_call.side_effect = lambda path, **_: different if str(path).startswith('/var/tmp/') else info
            with self.assertRaisesRegex(Exception, 'SOURCE_POLICY_MOUNT'):
                self.s.policy_bytes(self.o, '/var/tmp/lagrange-paper-genuine-policy-unit')


class Controls(Guarded):
    def test_start_releases_waits_for_real_arm_then_reacquires_and_retains(self):
        events = self.orchestration()
        self.s.unit_state.side_effect = [{'ActiveState': 'inactive'}, {'ActiveState': 'active'},
                                        {'ActiveState': 'active'}, {'ActiveState': 'active'}]
        self.patch('process_state', return_value={'identity': {'pid': 2, 'start_ticks': 3}})
        self.patch('arm_observation', side_effect=lambda *_: events.append('real-arm') or 'sha256:actual')
        result = self.s.run_source_service(self.o, '/protected/binding', '/policy', 'start')
        self.assertLess(events.index('borrowers-closed'), events.index('unlock'))
        self.assertLess(events.index('unlock'), next(i for i, v in enumerate(events) if v.startswith('ctl:')))
        self.assertLess(events.index('real-arm'), events.index('reacquire'))
        self.assertLess(events.index('reacquire'), events.index('save:ARMED'))
        self.assertEqual(result['status'], 'ARMED')
        self.o.dispose.assert_not_called()
        self.assertEqual(self.o.lock_fd, 19)

    def test_start_waits_through_initial_current_before_any_reacquisition(self):
        events = self.orchestration()
        states = iter([{'ActiveState': 'inactive'}])
        self.s.unit_state.side_effect = lambda *_: next(states, {'ActiveState': 'active'})
        self.patch('process_state', return_value={'identity': {'pid': 2, 'start_ticks': 3}})
        observations = iter([None, 'sha256:real-arm'])
        def arm(*_args):
            value = next(observations)
            events.append('wait' if value is None else 'arm')
            self.assertNotIn('reacquire', events)
            return value
        self.patch('arm_observation', side_effect=arm)
        with mock.patch.object(self.s.time, 'sleep'):
            self.s.run_source_service(self.o, '/protected/binding', '/policy', 'start')
        self.assertLess(events.index('wait'), events.index('arm'))
        self.assertLess(events.index('arm'), events.index('reacquire'))

    def test_start_stopped_failure_reacquires_but_never_returns_success(self):
        events = self.orchestration()
        with self.assertRaisesRegex(Exception, 'SOURCE_START_FAILED'):
            self.s.run_source_service(self.o, '/protected/binding', '/policy', 'start')
        self.assertIn('reacquire', events)
        self.assertEqual(self.r['phase'], 'STOPPED')
        self.o.dispose.assert_not_called()

    def test_unknown_control_preserves_roots_and_does_not_race_relock(self):
        events = self.orchestration()
        self.s.ctl.side_effect = self.s.SourceCustodyUnknown('SOURCE_COMMAND_UNKNOWN')
        with self.assertRaisesRegex(Exception, 'SOURCE_COMMAND_UNKNOWN'):
            self.s.run_source_service(self.o, '/protected/binding', '/policy', 'start')
        self.assertNotIn('reacquire', events)
        self.assertTrue(self.o.original_custody_uncertain)
        self.assertEqual(self.r['phase'], 'STARTING')
        self.o.dispose.assert_not_called()

    def test_startup_observation_uncertainty_keeps_custody(self):
        self.orchestration()
        self.s.unit_state.side_effect = [{'ActiveState': 'inactive'}, {'ActiveState': 'active'}]
        self.patch('process_state', return_value={'identity': {'pid': 2, 'start_ticks': 3}})
        self.patch('arm_observation', side_effect=self.s.SourceCustodyUnknown('SOURCE_CURRENT_UNKNOWN'))
        with self.assertRaisesRegex(Exception, 'SOURCE_CURRENT_UNKNOWN'):
            self.s.run_source_service(self.o, '/protected/binding', '/policy', 'start')
        self.s.reacquire.assert_not_called()
        self.o.dispose.assert_not_called()

    def test_stop_releases_before_command_and_proves_disposition_before_relock(self):
        events = self.orchestration(phase='ARMED', active=True)
        self.r['process'] = {'identity': {'pid': 2, 'start_ticks': 3}}
        self.patch('attempt_process', return_value=self.r['process'])
        self.s.unit_state.side_effect = [{'ActiveState': 'active'}, {'ActiveState': 'inactive'}, {'ActiveState': 'inactive'}]
        result = self.s.run_source_service(self.o, '/protected/binding', '/policy', 'stop')
        self.assertEqual(result['status'], 'STOPPED')
        self.assertLess(events.index('unlock'), events.index('ctl:--no-block stop ' + self.s.unit_name(self.o)))
        self.assertLess(events.index('reacquire'), events.index('save:STOPPED'))
        self.o.dispose.assert_not_called()

    def test_stop_foreign_process_generation_is_never_signalled(self):
        self.orchestration(phase='ARMED', active=True)
        self.patch('attempt_process', side_effect=self.s.SourceCustodyUnknown('SOURCE_PROCESS_CHANGED'))
        with self.assertRaisesRegex(Exception, 'SOURCE_PROCESS_CHANGED'):
            self.s.run_source_service(self.o, '/protected/binding', '/policy', 'stop')
        self.s.ctl.assert_not_called()
        self.s.fcntl.flock.assert_not_called()

    def test_status_is_descriptive_and_uses_no_control_or_relock(self):
        self.orchestration(phase='ARMED', active=True)
        self.patch('attempt_process', return_value={'actual': 'process'})
        self.patch('current_observation', return_value=('current', 'clock'))
        result = self.s.run_source_service(self.o, '/protected/binding', '/policy', 'status')
        self.assertEqual(result['status'], 'RUNNING')
        self.assertFalse(result['archive_acceptance'])
        self.s.ctl.assert_not_called()
        self.s.fcntl.flock.assert_not_called()

    def test_original_lock_inode_and_nondecreasing_floor_revalidated(self):
        before = {'lock': [1, 2, 0, 0, 0o600], 'floor': 3, 'db': ['original']}
        info = types.SimpleNamespace(st_dev=1, st_ino=2, st_uid=0, st_gid=0, st_mode=0o100600)
        with mock.patch.object(self.s.os, 'fstat', return_value=info), \
             mock.patch.object(self.s.fcntl, 'flock') as flock, \
             mock.patch.object(self.s, 'owner_snapshot', return_value=dict(before, floor=4)) as snapshot:
            self.s.reacquire(self.o, before)
            flock.assert_called_once_with(19, self.s.fcntl.LOCK_EX | self.s.fcntl.LOCK_NB)
            snapshot.return_value = dict(before, floor=2)
            with self.assertRaisesRegex(Exception, 'SOURCE_OWNER_CHANGED'):
                self.s.reacquire(self.o, before)
            snapshot.return_value = dict(before, db=['restarted'])
            with self.assertRaisesRegex(Exception, 'SOURCE_OWNER_CHANGED'):
                self.s.reacquire(self.o, before)


class OriginalObservations(Guarded):
    def test_actual_unbound_starting_envelope_waits_only_for_owned_publisher_and_boot(self):
        fixture = self.h.ROOT / 'crates/job-queue/tests/fixtures/owner_beta_paper_source_evidence/valid_chain.json'
        event = self.e.decode(fixture.read_bytes(), self.e.SESSION_LIMIT)['events'][0]
        current = dict(schema='owner-beta-paper-current-binding-v1', status='INVALID', reason='STARTING',
            generation=event['arm']['generation'], boot_id=event['arm']['boot_id'],
            publisher=event['arm']['parent'], sequence=1, observed=event['observed'],
            binding=None, binding_sha256=None)
        process = {'identity': event['arm']['parent']}
        with mock.patch.object(self.e, 'read_file', side_effect=lambda *_args, **_: self.e.canonical(current)), \
             mock.patch.object(Path, 'read_text', return_value=current['boot_id'] + '\n'), \
             mock.patch.object(self.e, 'sample', return_value=event['observed']):
            # Keep the actual frozen-envelope validator in the consuming path.
            self.assertIsNone(self.s.current_observation(self.o, self.b, process))
            for field, value in (('publisher', {'pid': 101, 'start_ticks': 1000}),
                                 ('boot_id', '00000000-0000-4000-8000-000000000099'),
                                 ('reason', 'STOPPED')):
                with self.subTest(field=field):
                    original = current[field]
                    current[field] = value
                    with self.assertRaisesRegex(Exception, 'SOURCE_CURRENT_UNKNOWN'):
                        self.s.current_observation(self.o, self.b, process)
                    current[field] = original
            self.o.dispose.assert_not_called()

    def test_arm_endpoint_is_sampled_after_actual_chain_and_claim_reads(self):
        fixture = self.h.ROOT / 'crates/job-queue/tests/fixtures/owner_beta_paper_source_evidence/valid_chain.json'
        events = self.e.decode(fixture.read_bytes(), self.e.SESSION_LIMIT)['events'][:3]
        b = dict(self.b, cohort=events[0]['cohort'])
        parent = events[0]['arm']['parent']
        before = dict(events[0]['observed'])
        endpoint = {k: v + 10**9 for k, v in events[-1]['observed'].items()}
        current = dict(status='CURRENT', binding=b, publisher=parent,
            boot_id=events[0]['arm']['boot_id'], observed=before)
        common = ('schema', 'slot_id', 'session', 'due_realtime_ns', 'cohort', 'arm', 'reservation_id', 'invocation_id')
        claim = {k: events[0][k] for k in common}
        read_claim = False
        sample_count = 0

        def read(path, *_args, **_kwargs):
            nonlocal read_claim
            if Path(path).name == self.e.CURRENT_LEAF:
                return self.e.canonical(current)
            if Path(path).parent.name == '.control':
                read_claim = True
                return self.e.canonical(claim)
            return self.e.canonical(events[int(Path(path).stem)])

        def sample():
            nonlocal sample_count
            sample_count += 1
            if sample_count == 1:
                return before
            self.assertTrue(read_claim, 'endpoint sampled before the complete claim')
            return endpoint

        with contextlib.ExitStack() as patches:
            patches.enter_context(mock.patch.object(self.e, 'read_file', side_effect=read))
            patches.enter_context(mock.patch.object(self.e, 'validate_current'))
            patches.enter_context(mock.patch.object(self.e, 'sample', side_effect=sample))
            @contextlib.contextmanager
            def dirs(*_args, **_kwargs):
                yield 20 if Path(_args[0]).name == 'events' else 21
            patches.enter_context(mock.patch.object(self.e, 'opened', side_effect=dirs))
            patches.enter_context(mock.patch.object(self.s.os, 'listdir', side_effect=lambda fd:
                [events[0]['slot_id'][7:]] if fd == 20 else [f'{i:06}.json' for i in range(3)]))
            patches.enter_context(mock.patch.object(self.s.time, 'time_ns', return_value=before['realtime_ns']))
            self.assertEqual(self.s.arm_observation(self.o, b, {'identity': parent}),
                self.e.digest(self.e.canonical(events[-1])))
            self.assertEqual(sample_count, 2)
            for field in ('realtime_ns', 'boottime_ns', 'monotonic_ns'):
                with self.subTest(rollback=field):
                    original = endpoint[field]
                    endpoint[field] = events[-1]['observed'][field] - 1
                    read_claim, sample_count = False, 0
                    with self.assertRaisesRegex(Exception, 'CLOCK_INVALID'):
                        self.s.arm_observation(self.o, b, {'identity': parent})
                    endpoint[field] = original
            for seconds in (31, 61):
                with self.subTest(endpoint_seconds=seconds):
                    # A scan cannot preserve stale Current or an expired future-due window.
                    current['observed'] = {k: v + (50 if seconds == 61 else 0) * 10**9 for k, v in before.items()}
                    before.update(current['observed'])
                    endpoint.update({k: v + seconds * 10**9 for k, v in events[-1]['observed'].items()})
                    read_claim, sample_count = False, 0
                    with self.assertRaises(self.s.SourceCustodyUnknown):
                        self.s.arm_observation(self.o, b, {'identity': parent})
            self.o.dispose.assert_not_called()

    def test_initial_invalid_current_waits_and_stale_current_cannot_prove_arm(self):
        current = dict(status='INVALID', reason='STOPPED', binding=self.b)
        with mock.patch.object(self.e, 'read_file', side_effect=lambda *_args, **_: self.e.canonical(current)), \
             mock.patch.object(self.e, 'validate_current'), \
             mock.patch.object(self.e, 'sample', return_value={k: 32 * 10**9 for k in ('realtime_ns', 'boottime_ns', 'monotonic_ns')}):
            self.assertIsNone(self.s.arm_observation(self.o, self.b, {'identity': {'pid': 2}}))
            current.update(status='CURRENT', publisher={'pid': 2},
                observed={k: 10**9 for k in ('realtime_ns', 'boottime_ns', 'monotonic_ns')})
            with self.assertRaisesRegex(Exception, 'SOURCE_CURRENT_UNKNOWN'):
                self.s.arm_observation(self.o, self.b, {'identity': {'pid': 2}})

    def test_actual_chain_validator_and_claim_prove_only_arm_prefix(self):
        fixture = self.h.ROOT / 'crates/job-queue/tests/fixtures/owner_beta_paper_source_evidence/valid_chain.json'
        events = self.e.decode(fixture.read_bytes(), self.e.SESSION_LIMIT)['events'][:3]
        b = dict(self.b, cohort=events[0]['cohort'])
        parent = events[0]['arm']['parent']
        current = dict(status='CURRENT', binding=b, publisher=parent,
            boot_id=events[0]['arm']['boot_id'], observed=events[-1]['observed'])
        names = [events[0]['slot_id'][7:]]
        leaves = [f'{i:06}.json' for i in range(3)]
        common = ('schema', 'slot_id', 'session', 'due_realtime_ns', 'cohort', 'arm', 'reservation_id', 'invocation_id')
        claim = {k: events[0][k] for k in common}
        def read(path, *_args, **_kwargs):
            if Path(path).name == self.e.CURRENT_LEAF:
                return self.e.canonical(current)
            if Path(path).parent.name == '.control':
                return self.e.canonical(claim)
            return self.e.canonical(events[int(Path(path).stem)])
        with contextlib.ExitStack() as patches:
            patches.enter_context(mock.patch.object(self.e, 'read_file', side_effect=read))
            patches.enter_context(mock.patch.object(self.e, 'validate_current'))
            patches.enter_context(mock.patch.object(self.e, 'sample', side_effect=lambda: events[-1]['observed']))
            # Distinct directory handles avoid ambiguous traversal mocks.
            handles = iter([20, 21])
            @contextlib.contextmanager
            def dirs(*_args, **_kwargs):
                yield next(handles)
            patches.enter_context(mock.patch.object(self.e, 'opened', side_effect=dirs))
            patches.enter_context(mock.patch.object(self.s.os, 'listdir', side_effect=lambda fd: names if fd == 20 else leaves))
            patches.enter_context(mock.patch.object(self.s.time, 'time_ns', return_value=events[0]['due_realtime_ns'] - 120 * 10**9))
            self.assertEqual(self.s.arm_observation(self.o, b, {'identity': parent}), self.e.digest(self.e.canonical(events[-1])))
            # Real verify_chain rejects a fabricated ARMED transition/hash.
            events[-1]['previous_sha256'] = 'sha256:' + '0' * 64
            handles = iter([20, 21])
            with self.assertRaises(self.e.Rejected):
                self.s.arm_observation(self.o, b, {'identity': parent})
            self.o.dispose.assert_not_called()

    def test_actual_process_generation_mount_policy_and_argv_mutants(self):
        pid = 123
        root, unit, drop, policy, binding = self.s.paths(self.o)
        st = dict(MainPID=str(pid), ActiveState='active', SubState='running', InvocationID='b' * 32,
            ControlGroup='/system.slice/' + self.s.unit_name(self.o), ExecMainStartTimestampMonotonic='500')
        b = dict(self.b, release_root=str(self.o.release))
        texts = {f'/proc/{pid}/cgroup': '0::' + st['ControlGroup'] + '\n',
            f'/proc/{pid}/mountinfo': '9 1 0:9 /owned /var/lib/lagrange/service-status ro,nosuid - ext4 /dev/private rw\n'}
        argv = ['/usr/bin/python3', str(self.o.release / 'scripts/ops/lib/owner-beta-paper-evidence.py'), 'run', '--binding', str(binding)]
        blobs = {f'/proc/{pid}/cmdline': b'\0'.join(x.encode() for x in argv) + b'\0',
            f'/proc/{pid}/root/var/lib/lagrange/service-status/policy.json': b'policy'}
        info = types.SimpleNamespace(st_dev=1, st_ino=2, st_uid=0, st_gid=0, st_mode=0o40700, st_nlink=1)
        with contextlib.ExitStack() as patches:
            patches.enter_context(mock.patch.object(self.e, 'process', return_value={'pid': pid, 'start_ticks': 3}))
            patches.enter_context(mock.patch.object(self.s, 'binding_owner', return_value=(b, b'wire')))
            patches.enter_context(mock.patch.object(self.s, 'policy_bytes', return_value=b'policy'))
            patches.enter_context(mock.patch.object(Path, 'read_text', side_effect=lambda *a, **_: texts[str(a[0])], autospec=True))
            patches.enter_context(mock.patch.object(Path, 'read_bytes', side_effect=lambda *a, **_: blobs[str(a[0])], autospec=True))
            patches.enter_context(mock.patch.object(self.s.os, 'stat', return_value=info))
            actual = self.s.process_state(self.o, st)
            self.assertEqual(actual['identity'], {'pid': pid, 'start_ticks': 3})
            for path, bad in ((f'/proc/{pid}/mountinfo', texts[f'/proc/{pid}/mountinfo'].replace('ro,nosuid', 'rw,nosuid')),
                              (f'/proc/{pid}/cgroup', '0::/foreign\n')):
                old = texts[path]
                texts[path] = bad
                with self.subTest(path=path), self.assertRaises(self.s.SourceCustodyUnknown):
                    self.s.process_state(self.o, st)
                texts[path] = old
            blobs[f'/proc/{pid}/root/var/lib/lagrange/service-status/policy.json'] = b'changed-policy'
            with self.assertRaisesRegex(Exception, 'SOURCE_POLICY_MOUNT'):
                self.s.process_state(self.o, st)
            blobs[f'/proc/{pid}/cmdline'] = b'foreign\0'
            with self.assertRaisesRegex(Exception, 'SOURCE_PROCESS_CHANGED'):
                self.s.process_state(self.o, st)

    def test_binding_assets_are_reused_and_actual_network_name_is_bound(self):
        self.o.project, self.o.network, self.o.account = 'fixture', 'network-id', 'account'
        b = dict(self.b, cohort=dict(verifier_revision=self.o.commit, release_manifest_sha256='manifest-hash',
                cohort_sha256='cohort-hash', owner_user_id='owner', strategy_config_id='strategy'),
            release_root=str(self.o.release), project='fixture', network='actual-network-name', database='lagrange',
            production_env_file=str(self.o.release / 'deploy/compose/.env'), data_root=str(self.o.root / 'data'),
            secret_root=str(self.o.root / 'secrets'), calendar_dir=str(self.o.root / 'calendar'),
            calendar_file=str(self.o.root / 'calendar/calendar.json'))
        process = dict(account_id='account', daily_cohort_sha256='cohort-hash', owner_user_id='owner', strategy_config_id='strategy')
        raw = self.e.canonical(process)
        self.o.prepared_process_sha256 = self.e.digest(raw)
        self.o.manifest = b'manifest'
        b['cohort']['release_manifest_sha256'] = self.e.digest(self.o.manifest)
        self.o.docker = types.SimpleNamespace(object=mock.Mock(return_value=dict(Id='network-id', Internal=True,
            Labels={'com.docker.compose.project': 'fixture'}, Name='actual-network-name')))
        with mock.patch.object(self.e, '_binding_with_helper_projection', return_value=(b, b'wire', object())) as assets, \
             mock.patch.object(self.e, 'read_file', return_value=raw):
            self.assertEqual(self.s.binding_owner(self.o, '/original'), (b, b'wire'))
            assets.assert_called_once_with('/original')
            for key, bad in (('network', 'network-id'), ('project', 'foreign'), ('data_root', '/production')):
                old = b[key]
                b[key] = bad
                with self.subTest(key=key), self.assertRaisesRegex(Exception, 'SOURCE_BINDING_OWNER'):
                    self.s.binding_owner(self.o, '/original')
                b[key] = old

    def test_fixed_command_uses_capped_output_clean_env_and_original_deadline(self):
        def run(argv, **kwargs):
            self.assertEqual(argv, ['/usr/bin/systemctl', '--no-pager', 'daemon-reload'])
            self.assertEqual(kwargs['env'], self.s.ENV)
            self.assertTrue(kwargs['close_fds'])
            self.assertEqual(kwargs['timeout'], 10)
            self.assertNotIn('shell', kwargs)
            self.assertTrue(callable(kwargs['preexec_fn']))
            kwargs['stdout'].write(b'OK')
            return types.SimpleNamespace(returncode=0)
        with mock.patch.object(self.s.subprocess, 'run', side_effect=run):
            self.assertEqual(self.s.ctl(self.o, 'daemon-reload'), 'OK')
        with mock.patch.object(self.s.subprocess, 'run', side_effect=self.s.subprocess.TimeoutExpired(['fixed'], 1)):
            with self.assertRaisesRegex(Exception, 'SOURCE_COMMAND_UNKNOWN'):
                self.s.ctl(self.o, 'daemon-reload')

    def test_provider_and_backend_disposition_never_assumes_separate_projects(self):
        self.o.project, self.o.pg = 'fixture', '1' * 64
        self.o.docker = types.SimpleNamespace(object=mock.Mock(return_value={
            'Config': {'Labels': {'com.docker.compose.service': 'research-worker', 'com.docker.compose.project': 'foreign'}},
            'State': {'Running': True}, 'Mounts': [], 'Name': '/unrelated-research'}))
        with mock.patch.object(self.s, 'foreign_processes'), \
             mock.patch.object(self.s, 'command', return_value='2' * 64):
            with self.assertRaisesRegex(Exception, 'SOURCE_FOREIGN_WRITER'):
                self.s.provider_idle(self.o)
        self.o.observer = types.SimpleNamespace(target=mock.Mock(), json=mock.Mock(return_value={'clients': 1, 'prepared': 0}))
        with mock.patch.object(self.s, 'provider_idle'):
            with self.assertRaisesRegex(Exception, 'SOURCE_BACKEND_UNKNOWN'):
                self.s.quiescent(self.o)
            self.o.observer.json.return_value = {'clients': 0, 'prepared': 0}
            self.s.quiescent(self.o)

    def test_manual_source_writer_outside_systemd_is_refused_without_signal(self):
        process = mock.Mock(name='proc-entry')
        process.name = '123'
        stream = io.BytesIO(b'/usr/bin/python3\0/release/owner-beta-paper-evidence.py\0run\0--binding\0/private\0')
        process.__truediv__ = mock.Mock(return_value=types.SimpleNamespace(open=mock.Mock(return_value=contextlib.closing(stream))))
        with mock.patch.object(Path, 'iterdir', return_value=iter([process])):
            with self.assertRaisesRegex(Exception, 'SOURCE_FOREIGN_WRITER'):
                self.s.foreign_processes(self.o)


class TerminalCustody(Guarded):
    def setup_custody(self, *, service=False, acceptance=False):
        events = []
        self.patch('owned')
        self.patch('host_units', side_effect=lambda *_: events.append('units'))
        self.patch('absent', side_effect=lambda *_: events.append('absent'))
        self.patch('unit_state', return_value={'LoadState': 'not-found', 'ActiveState': 'inactive'})
        self.patch('stopped', side_effect=lambda *_: events.append('stopped'))
        self.patch('quiescent', side_effect=lambda *_: events.append('quiescent'))
        self.patch('load_receipt', side_effect=lambda _: (copy.deepcopy(self.r), self.b))
        self.patch('save', side_effect=lambda _, r: (self.r.update(copy.deepcopy(r)), events.append('save:' + r['phase'])))
        self.patch('ctl', side_effect=lambda *_: events.append('reload'))
        self.stack.enter_context(mock.patch.object(self.s.os.path, 'lexists', return_value=service))
        self.stack.enter_context(mock.patch.object(self.s.os, 'unlink', side_effect=lambda *_: events.append('unlink')))
        self.stack.enter_context(mock.patch.object(self.s.os, 'rmdir', side_effect=lambda *_: events.append('rmdir')))
        self.stack.enter_context(mock.patch.object(self.e, 'read_file', return_value=b'current'))
        self.stack.enter_context(mock.patch.object(self.e, 'decode', return_value={'status': 'INVALID', 'binding': self.b}))
        self.stack.enter_context(mock.patch.object(self.e, 'validate_current'))
        @contextlib.contextmanager
        def locks(guard, stack):
            events.append('lock')
            yield
            events.append('unlock')
        def acquire(guard, stack):
            stack.enter_context(locks(guard, stack))
        self.stack.enter_context(mock.patch.object(self.s.Custody, 'publisher_locks', acquire))
        self.o.dispose.side_effect = lambda callback: (events.append('DB-teardown'), callback(), events.append('root-teardown'))
        return events, self.s.Custody(self.o, acceptance=acceptance)

    def test_absent_service_disposal_has_a_consuming_positive_path(self):
        events, custody = self.setup_custody()
        with custody:
            self.s.dispose_with_source_custody(self.o, custody)
        self.o.dispose.assert_called_once()
        self.assertLess(events.index('absent'), events.index('DB-teardown'))

    def test_stopped_service_unit_and_locks_survive_c_then_terminal_seal(self):
        events, custody = self.setup_custody(service=True, acceptance=True)
        with custody:
            events.append('real-C-own-lock-lifetime')
            self.assertNotIn('unlink', events, 'C must discover original Source unit/binding')
            self.s.dispose_with_source_custody(self.o, custody)
        self.assertLess(events.index('real-C-own-lock-lifetime'), events.index('save:RETIRING'))
        self.assertLess(events.index('save:RETIRING'), events.index('unlink'))
        self.assertLess(events.index('reload'), events.index('DB-teardown'))
        # Initial probe lock is released before C; terminal lock lasts through DB.
        self.assertEqual(events.count('lock'), 2)
        self.assertGreater(events.index('DB-teardown'), max(i for i, x in enumerate(events) if x == 'lock'))
        self.assertLess(events.index('DB-teardown'), max(i for i, x in enumerate(events) if x == 'unlock'))
        self.assertLess(max(i for i, x in enumerate(events) if x == 'unlock'), events.index('root-teardown'))

    def test_active_or_unknown_service_never_invokes_disposal(self):
        for error in ('SOURCE_NOT_STOPPED', 'SOURCE_ASSETS_CHANGED', 'SOURCE_CURRENT_UNKNOWN'):
            with self.subTest(error=error), contextlib.ExitStack() as local:
                events, custody = self.setup_custody(service=True)
                local.enter_context(mock.patch.object(custody, 'check', side_effect=self.s.SourceCustodyUnknown(error)))
                with self.assertRaisesRegex(Exception, error):
                    with custody:
                        self.s.dispose_with_source_custody(self.o, custody)
                self.o.dispose.assert_not_called()
                self.assertNotIn('unlink', events)

    def test_late_stop_or_launch_uncertainty_retains_database(self):
        events, custody = self.setup_custody(service=True, acceptance=True)
        with self.assertRaisesRegex(Exception, 'SOURCE_NOT_STOPPED'):
            with custody:
                self.s.stopped.side_effect = self.s.SourceCustodyUnknown('SOURCE_NOT_STOPPED')
                self.s.dispose_with_source_custody(self.o, custody)
        self.o.dispose.assert_not_called()
        self.assertNotIn('unlink', events)

    def test_unentered_custody_cannot_authorize_disposal(self):
        with self.assertRaisesRegex(Exception, 'SOURCE_CUSTODY_UNKNOWN'):
            self.s.dispose_with_source_custody(self.o, self.s.Custody(self.o))
        self.o.dispose.assert_not_called()

    def test_lock_probe_checks_actual_ofd_owner_without_claiming_readiness(self):
        custody = self.s.Custody(self.o)
        custody.binding = self.b
        @contextlib.contextmanager
        def opened(*_, **__):
            yield 21
        with contextlib.ExitStack() as patches:
            patches.enter_context(mock.patch.object(self.e, 'opened', side_effect=opened))
            patches.enter_context(mock.patch.object(self.s.os, 'fstat', return_value=types.SimpleNamespace(
                st_dev=1, st_ino=2, st_uid=0, st_gid=0, st_mode=0o100600)))
            flock = patches.enter_context(mock.patch.object(self.s.fcntl, 'flock'))
            getlk = patches.enter_context(mock.patch.object(self.s.fcntl, 'fcntl',
                return_value=bytes(self.e.OfdFlock(self.s.fcntl.F_UNLCK, 0, 0, 0, 0))))
            with contextlib.ExitStack() as held:
                custody.publisher_locks(held)
            self.assertEqual(flock.call_count, 2)
            getlk.return_value = bytes(self.e.OfdFlock(self.s.fcntl.F_WRLCK, 0, 0, 0, 0))
            with contextlib.ExitStack() as held, self.assertRaisesRegex(Exception, 'SOURCE_CURRENT_UNKNOWN'):
                custody.publisher_locks(held)

    def test_bootstrap_failure_uses_actual_absent_custody_positive_and_unknown(self):
        events, _custody = self.setup_custody()
        self.o.extractions = mock.MagicMock()
        self.o.setup = mock.Mock(side_effect=self.h.HistoryError('SETUP_FAILED'))
        self.o.deadline.cleanup_phase = mock.Mock()
        with mock.patch.object(self.f, 'FixtureOwner', return_value=self.o), \
             mock.patch.object(self.h, 'source_module', return_value=self.s):
            with self.assertRaisesRegex(Exception, 'SETUP_FAILED'):
                self.h.bootstrap('/source', self.o.commit, b'manifest', self.o.deadline,
                    approved_base='/base', raw_root='/raw', config_source='/config', generic_bundle='/bundle',
                    generic_session='2026-11-10', policy_root='/policy')
            self.o.dispose.assert_called_once()
            self.o.dispose.reset_mock()
            self.s.host_units.side_effect = self.s.SourceCustodyUnknown('SOURCE_FOREIGN_UNIT')
            with self.assertRaisesRegex(Exception, 'SOURCE_FOREIGN_UNIT'):
                self.h.bootstrap('/source', self.o.commit, b'manifest', self.o.deadline,
                    approved_base='/base', raw_root='/raw', config_source='/config', generic_bundle='/bundle',
                    generic_session='2026-11-10', policy_root='/policy')
            self.o.dispose.assert_not_called()

    def test_real_history_accept_consumer_uses_held_guard_before_c_and_disposal(self):
        events, _ = self.setup_custody(service=True, acceptance=True)
        values = {key: str(self.o.root / key.lower()) for key in ('BACKUP_ROOT', 'KEY_FILE', 'LOCK_FILE', 'METRICS_FILE')}
        selection = types.SimpleNamespace(_backup_module=lambda: types.SimpleNamespace(config_file=lambda *_: values))
        original_load = self.h.load
        archive = mock.Mock(side_effect=lambda *_args, **_kwargs: events.append('archive') or [{'case': 'MOCK_C', 'status': 'PASS'}])
        args = types.SimpleNamespace(phase='accept', storage_config='/owned/config')
        facts = {'generic_preview_id': 'preview', 'generic_dataset': {'id': 'generic'}}
        with mock.patch.object(self.h, 'prepared_release'), \
             mock.patch.object(self.h, 'verify_generic_preview'), \
             mock.patch.object(self.h, 'archive_acceptance', archive), \
             mock.patch.object(self.h, 'load', side_effect=lambda name, path: selection if name == 'paper_genuine_history_selection' else original_load(name, path)):
            with self.s.require_source_custody_for_acceptance(self.o) as custody:
                result = self.h.retained_phase(self.o, facts, args, '/policy', source_custody=custody)
                self.assertTrue(result['archive_acceptance'])
                self.assertNotIn('unlink', events)
                self.s.dispose_with_source_custody(self.o, custody)
            self.assertLess(events.index('archive'), events.index('DB-teardown'))
            self.assertLess(events.index('quiescent'), events.index('archive'))
            self.o.dispose.reset_mock()
            archive.reset_mock()
            self.s.stopped.side_effect = self.s.SourceCustodyUnknown('SOURCE_NOT_STOPPED')
            self.r['phase'] = 'STOPPED'  # Independent retained-owner scenario.
            with self.assertRaisesRegex(Exception, 'SOURCE_NOT_STOPPED'):
                with self.s.require_source_custody_for_acceptance(self.o) as custody:
                    self.h.retained_phase(self.o, facts, args, '/policy', source_custody=custody)
            archive.assert_not_called()
            self.o.dispose.assert_not_called()

    def test_late_held_writer_lock_path_replacement_refuses_teardown(self):
        events, custody = self.setup_custody(service=True, acceptance=True)
        info = types.SimpleNamespace(st_dev=1, st_ino=3, st_uid=0, st_gid=0, st_mode=0o100600)
        with mock.patch.object(self.s.os, 'fstat', return_value=info), \
             self.assertRaisesRegex(Exception, 'SOURCE_LOCK_CHANGED'):
            with custody:
                custody.lock_pins = [(Path('/owned/lock'), 21, [1, 2, 0, 0, 0o600])]
                self.s.dispose_with_source_custody(self.o, custody)
        self.o.dispose.assert_not_called()


class PolicyProjectionContracts(Guarded):
    def test_ordinary_source_drop_preserves_actual_material_paths(self):
        policy=self.s.paths(self.o)[3]
        self.assertEqual(self.s.drop_body(policy),
            ('[Service]\nBindReadOnlyPaths='+str(policy / 'service-status')+':/var/lib/lagrange/service-status\n').encode())

    def test_actual_material_cli_selects_shared_scope_not_legacy_g2(self):
        import inspect
        source=inspect.getsource(self.h.run)
        self.assertIn('fixture.ServiceStatusScope(policy_root)',source)
        self.assertNotIn('fixture.PrivatePolicy(',source)

    def test_shared_scope_exists_without_changing_legacy_g2_bytes(self):
        import ast,hashlib
        self.assertTrue(hasattr(self.f,'ServiceStatusScope'))
        path=HERE / 'owner-beta-paper-genuine-fixture.py'
        raw=path.read_text();tree=ast.parse(raw)
        legacy=next(node for node in tree.body if isinstance(node,ast.ClassDef) and node.name=='PrivatePolicy')
        self.assertEqual(hashlib.sha256(ast.get_source_segment(raw,legacy).encode()).hexdigest(),
            '0149d2711f43671797ab448399b929b6d201c6cd2749d35483858bbf83bc5245')
        run=next(node for node in tree.body if isinstance(node,ast.FunctionDef) and node.name=='run')
        main=next(node for node in tree.body if isinstance(node,ast.FunctionDef) and node.name=='main')
        self.assertIn('with PrivatePolicy(policy_root):',ast.get_source_segment(raw,run))
        self.assertIn('run(args.source_root, args.commit, args.manifest_file, args.deadline_monotonic_ns)',ast.get_source_segment(raw,main))


class ServiceStatusScopeChecks(Guarded):
    def scope_effects(self):
        policy = Path('/private/policy')
        state = {'mounted': False, 'restored': False, 'wrong_target': False, 'wrong_fd': False}
        library = mock.Mock()
        library.unshare.return_value = 0
        def mount(source, target, *_args):
            if source is not None:
                self.assertEqual((source, target), (b'/private/policy/service-status',
                    b'/var/lib/lagrange/service-status'))
                state['mounted'] = True
            return 0
        def unmount(*_):
            state['mounted'] = False
            return 0
        def restore(*_):
            state['restored'] = True
            return 0
        library.mount.side_effect = mount
        library.umount2.side_effect = unmount
        library.setns.side_effect = restore
        self.stack.enter_context(mock.patch.object(self.f.ctypes, 'CDLL', return_value=library))
        self.stack.enter_context(mock.patch.object(self.f.os, 'geteuid', return_value=0))
        self.stack.enter_context(mock.patch.object(self.f.threading, 'active_count', return_value=1))
        self.stack.enter_context(mock.patch.object(Path, 'is_symlink', return_value=False))
        self.stack.enter_context(mock.patch.object(self.f.os, 'open', side_effect=lambda path, *_a, **_k:
            20 if str(path) == 'service-status' else 22 if Path(path) == policy else 21))
        def fstat(fd):
            return types.SimpleNamespace(st_dev=1, st_ino=99 if state['wrong_fd'] and fd == 20
                else {10:1,11:2,20:3,22:4,21:5}[fd], st_uid=0, st_gid=0, st_mode=0o40700)
        self.stack.enter_context(mock.patch.object(self.f.os, 'fstat', side_effect=fstat))
        closed = self.stack.enter_context(mock.patch.object(self.f.os, 'close'))
        self.stack.enter_context(mock.patch.object(self.f, '_service_status_evidence', return_value=self.e))
        acl = self.stack.enter_context(mock.patch.object(self.e, 'no_acl'))
        @contextlib.contextmanager
        def opened(path, **kw):
            if Path(path) == self.f.SERVICE_STATUS_TARGET:
                self.assertEqual(kw['exact'], (0,0,0o700))
                yield 11
            else:
                self.assertEqual(Path(path), self.f.SERVICE_STATUS_TARGET.parent)
                yield 10
        opened_mock = self.stack.enter_context(mock.patch.object(self.e, 'opened', side_effect=opened))
        def identity(path):
            path = Path(path)
            if path == self.f.SERVICE_STATUS_TARGET:
                return (1,99) if state['wrong_target'] else (1,3 if state['mounted'] else 2)
            if path == self.f.SERVICE_STATUS_TARGET.parent:
                return (1,1)
            return (1,4 if path == policy else 3)
        self.stack.enter_context(mock.patch.object(self.f.faults, 'directory_identity', side_effect=identity))
        scope = self.f.ServiceStatusScope(policy)
        self.addCleanup(self.f._UNRESOLVED_POLICY_SCOPES.discard, scope)
        return scope, library, closed, state, opened_mock, acl

    def test_setns_failure_retains_source_and_fds_even_after_acknowledged_unmount(self):
        scope, libc, closed, state, _, _ = self.scope_effects()
        scope.__enter__()
        libc.setns.side_effect = None
        libc.setns.return_value = -1
        with self.assertRaisesRegex(self.f.FixtureError, 'FIXTURE_NAMESPACE_RESTORE'):
            scope.__exit__(None, None, None)
        self.assertFalse(scope.bound)
        self.assertTrue(scope.entered)
        self.assertIn(scope, self.f._UNRESOLVED_POLICY_SCOPES)
        closed.assert_not_called()
        libc.setns.return_value = 0
        scope.__exit__(None, None, None)
        self.assertNotIn(scope, self.f._UNRESOLVED_POLICY_SCOPES)
        self.assertEqual(closed.call_args_list, [mock.call(21),mock.call(20),mock.call(22)])

    def test_restore_identity_failure_stays_unresolved_after_both_flags_clear(self):
        scope, libc, closed, state, _, _ = self.scope_effects()
        scope.__enter__()
        def restore(*_):
            state['wrong_target'] = True
            return 0
        libc.setns.side_effect = restore
        with self.assertRaisesRegex(self.f.FixtureError, 'FIXTURE_POLICY_TARGET_CHANGED'):
            scope.__exit__(None,None,None)
        self.assertFalse(scope.bound or scope.entered)
        self.assertIn(scope,self.f._UNRESOLVED_POLICY_SCOPES)
        closed.assert_not_called()
        state['wrong_target'] = False
        scope.__exit__(None,None,None)
        self.assertNotIn(scope,self.f._UNRESOLVED_POLICY_SCOPES)

    def test_readonly_bind_failure_unwinds_and_does_not_register_unknown(self):
        scope, libc, closed, state, _, _ = self.scope_effects()
        original = libc.mount.side_effect
        def mount(source,target,*args):
            if args[1] == 4096 | 32 | 1:
                return -1
            return original(source,target,*args)
        libc.mount.side_effect = mount
        with self.assertRaisesRegex(self.f.FixtureError,'FIXTURE_POLICY_BIND'):
            scope.__enter__()
        libc.umount2.assert_called_once()
        libc.setns.assert_called_once()
        self.assertEqual(closed.call_count,3)
        self.assertNotIn(scope,self.f._UNRESOLVED_POLICY_SCOPES)

    def test_missing_protected_target_has_no_host_creation_or_mount(self):
        scope, libc, closed, state, opened, _ = self.scope_effects()
        opened.side_effect = FileNotFoundError('absent target')
        with mock.patch.object(Path,'mkdir',side_effect=AssertionError('HOST_TARGET_CREATION')), \
             self.assertRaises(FileNotFoundError):
            scope.__enter__()
        libc.unshare.assert_not_called()
        libc.mount.assert_not_called()
        self.assertNotIn(scope,self.f._UNRESOLVED_POLICY_SCOPES)

    def test_source_acl_failure_is_before_namespace_and_releases_parent(self):
        scope, libc, closed, state, _, acl = self.scope_effects()
        acl.side_effect = self.e.Rejected('ORIGIN_UNPROVEN')
        with self.assertRaises(self.e.Rejected):
            scope.__enter__()
        libc.unshare.assert_not_called()
        closed.assert_called_once_with(22)

    def test_changed_held_source_rejects_cleanup_without_unmounting_foreign_object(self):
        scope, libc, closed, state, _, _ = self.scope_effects()
        scope.__enter__()
        state['wrong_fd'] = True
        with self.assertRaisesRegex(self.f.FixtureError,'FIXTURE_POLICY_TARGET_CHANGED'):
            scope.__exit__(None,None,None)
        self.assertIn(scope,self.f._UNRESOLVED_POLICY_SCOPES)
        libc.umount2.assert_not_called()
        libc.setns.assert_not_called()
        closed.assert_not_called()
        state['wrong_fd'] = False
        scope.__exit__(None,None,None)

    def test_replaced_source_name_is_unknown_even_with_original_descriptor_still_open(self):
        scope,libc,closed,state,_,_=self.scope_effects()
        scope.__enter__()
        original=self.f.faults.directory_identity.side_effect
        for path in (scope.policy,scope.policy/'service-status'):
            with self.subTest(replaced=path):
                self.f.faults.directory_identity.side_effect=lambda candidate: (1,99) if Path(candidate)==path else original(candidate)
                with self.assertRaisesRegex(self.f.FixtureError,'FIXTURE_POLICY_TARGET_CHANGED'):
                    scope.__exit__(None,None,None)
                self.assertIn(scope,self.f._UNRESOLVED_POLICY_SCOPES)
                libc.umount2.assert_not_called();closed.assert_not_called()
        self.f.faults.directory_identity.side_effect=original
        scope.__exit__(None,None,None)

    def test_actual_proc_leaf_proof_rejects_mount_and_three_identity_replacements(self):
        policy=Path('/owned/policy');pid=123
        mounted=Path('/proc/123/root/var/lib/lagrange/service-status')
        pairs=((mounted.parent,self.f.SERVICE_STATUS_TARGET.parent),
            (mounted,policy/'service-status'),(mounted/'policy.json',policy/'service-status/policy.json'))
        identities={str(path):number for number,pair in enumerate(pairs,1) for path in pair}
        line='9 1 0:9 /private /var/lib/lagrange/service-status ro,nosuid - ext4 /dev/private rw\n'
        def info(path):
            return types.SimpleNamespace(st_dev=1,st_ino=identities[str(path)],st_uid=0,st_gid=0,
                st_mode=0o100600 if str(path).endswith('policy.json') else 0o40700,st_nlink=1)
        with mock.patch.object(Path,'read_text',return_value=line) as mountinfo, \
             mock.patch.object(self.f.os,'stat',side_effect=info):
            self.assertEqual(self.f.service_status_namespace(policy,pid),mounted)
            for invalid in (line.replace('ro,nosuid','rw,nosuid'),line+line,
                    line.replace('/var/lib/lagrange/service-status','/var/lib/lagrange')):
                mountinfo.return_value=invalid
                with self.subTest(mount=invalid),self.assertRaises(self.f.FixtureError):
                    self.f.service_status_namespace(policy,pid)
            mountinfo.return_value=line
            for actual,_ in pairs:
                old=identities[str(actual)];identities[str(actual)]=99
                with self.subTest(replaced=str(actual)),self.assertRaisesRegex(self.f.FixtureError,'FIXTURE_POLICY_TARGET_CHANGED'):
                    self.f.service_status_namespace(policy,pid)
                identities[str(actual)]=old

    def test_actual_c_cli_material_routes_preserve_original_paths_inside_leaf_namespace(self):
        # Execute CLI -> history.run -> real bootstrap / retained_phase -> produce.
        # Stop at the independently owned material reader, before any DB effect.
        for phase in ('bootstrap','produce'):
            with self.subTest(phase=phase), contextlib.ExitStack() as patches:
                scope,libc,closed,state,_,_=self.scope_effects()
                policy=scope.policy
                paths=('/var/lib/lagrange/data/artifacts/historical-price-beta-root',
                    '/var/lib/lagrange/data/raw','/var/lib/lagrange/daily/config.json',
                    '/var/lib/lagrange/runtime/kis-read-coordination')
                originals={p:(7,i+1) for i,p in enumerate(paths)}
                def resolve(path):
                    path=Path(path)
                    if state['mounted'] and path.is_relative_to(self.f.SERVICE_STATUS_TARGET):
                        return policy/'service-status'/path.relative_to(self.f.SERVICE_STATUS_TARGET)
                    return path
                observed=[]
                def material(source,base,raw,deadline):
                    self.assertTrue(state['mounted'])
                    self.assertEqual((base,raw),paths[:2])
                    for path in paths:
                        self.assertEqual(originals[str(resolve(path))],originals[path])
                    self.assertEqual(resolve('/var/lib/lagrange/service-status/policy.json'),policy/'service-status/policy.json')
                    observed.append('original-material-visible')
                    raise self.h.HistoryError('MATERIAL_EFFECT_BOUNDARY')
                self.o.source=Path('/source');self.o.lock_fd=None
                self.o.extractions=contextlib.nullcontext()
                self.o.setup=mock.Mock()
                self.o.deadline.cleanup_phase=mock.Mock()
                patches.enter_context(mock.patch.object(self.h,'HistoricalMaterial',side_effect=material))
                patches.enter_context(mock.patch.object(self.f,'FixtureOwner',return_value=self.o))
                patches.enter_context(mock.patch.object(self.f,'ServiceStatusScope',return_value=scope))
                patches.enter_context(mock.patch.object(self.f,'validated_inputs',return_value=(Path('/source'),b'manifest',self.o.deadline)))
                patches.enter_context(mock.patch.object(self.f,'bounded_run',return_value=contextlib.nullcontext()))
                patches.enter_context(mock.patch.object(self.f,'termination_cleanup',return_value=contextlib.nullcontext()))
                patches.enter_context(mock.patch.object(self.h,'phase_deadline'))
                patches.enter_context(mock.patch.object(self.h,'prepared_release'))
                patches.enter_context(mock.patch.object(self.h,'resume_owner',return_value=(self.o,{})))
                patches.enter_context(mock.patch.object(self.s,'dispose_with_source_custody'))
                patches.enter_context(mock.patch.object(self.h,'source_module',return_value=self.s))
                patches.enter_context(mock.patch.object(self.cli,'history_module',return_value=self.h))
                patches.enter_context(mock.patch.object(self.h.os,'getegid',return_value=0))
                patches.enter_context(mock.patch.object(self.h.tempfile,'mkdtemp',return_value=str(policy)))
                patches.enter_context(mock.patch.object(Path,'mkdir'))
                remove=patches.enter_context(mock.patch.object(self.h.shutil,'rmtree'))
                args=['--source-root','/source','--commit',self.o.commit,'--manifest-file','/manifest',
                    '--deadline-monotonic-ns',str(self.o.deadline.value),phase,
                    '--approved-base',paths[0],'--raw-root',paths[1],'--daily-config',paths[2]]
                args += ['--generic-bundle','/bundle','--generic-session','2026-11-10'] if phase=='bootstrap' else ['--checkpoint','/checkpoint','--producer','daily']
                with contextlib.redirect_stdout(io.StringIO()) as output:
                    self.assertEqual(self.cli.main(args),1)
                self.assertEqual(observed,['original-material-visible'])
                self.assertNotIn('PASS',output.getvalue())
                self.assertFalse(scope.bound or scope.entered)
                self.assertTrue(state['restored'])
                remove.assert_called_once_with(policy)

    def test_actual_c_accept_output_follows_disposal_namespace_restore_and_policy_removal(self):
        scope,libc,closed,state,_,_=self.scope_effects()
        events=[]
        self.o.lock_fd=None;self.o.cleanup={}
        self.o.extractions=contextlib.nullcontext()
        self.o.deadline.cleanup_phase=lambda:events.append('cleanup-phase')
        facts={'generic_preview_id':'preview','generic_dataset':{'id':'dataset'}}
        values={key:str(self.o.root/key.lower()) for key in ('BACKUP_ROOT','KEY_FILE','LOCK_FILE','METRICS_FILE')}
        selection=types.SimpleNamespace(_backup_module=lambda:types.SimpleNamespace(config_file=lambda *_:values))
        original_load=self.h.load
        self.stack.enter_context(mock.patch.object(self.h,'load',side_effect=lambda name,path:
            selection if name=='paper_genuine_history_selection' else original_load(name,path)))
        self.stack.enter_context(mock.patch.object(self.h,'prepared_release'))
        self.stack.enter_context(mock.patch.object(self.h,'verify_generic_preview'))
        def archive(*_a,**_k):
            self.assertTrue(scope.bound)
            events.append('archive-consumed')
            return [{'case':'MOCK_EFFECT','status':'PASS'}]
        self.stack.enter_context(mock.patch.object(self.h,'archive_acceptance',side_effect=archive))
        custody=types.SimpleNamespace(owner=self.o,check=lambda:events.append('custody-check'))
        @contextlib.contextmanager
        def held(owner):
            self.assertIs(owner,self.o)
            events.append('custody-open')
            yield custody
            events.append('custody-close')
        def dispose(owner,held_custody):
            self.assertIs(held_custody,custody)
            self.assertTrue(scope.bound)
            events.append('dispose');owner.cleanup['containers']='REMOVED'
        self.stack.enter_context(mock.patch.object(self.s,'require_source_custody_for_acceptance',side_effect=held))
        self.stack.enter_context(mock.patch.object(self.s,'dispose_with_source_custody',side_effect=dispose))
        self.stack.enter_context(mock.patch.object(self.h,'source_module',return_value=self.s))
        self.stack.enter_context(mock.patch.object(self.f,'ServiceStatusScope',return_value=scope))
        self.stack.enter_context(mock.patch.object(self.f,'validated_inputs',return_value=(Path('/source'),b'manifest',self.o.deadline)))
        self.stack.enter_context(mock.patch.object(self.f,'bounded_run',return_value=contextlib.nullcontext()))
        self.stack.enter_context(mock.patch.object(self.f,'termination_cleanup',return_value=contextlib.nullcontext()))
        self.stack.enter_context(mock.patch.object(self.h,'phase_deadline'))
        self.stack.enter_context(mock.patch.object(self.h,'resume_owner',return_value=(self.o,facts)))
        self.stack.enter_context(mock.patch.object(self.cli,'history_module',return_value=self.h))
        self.stack.enter_context(mock.patch.object(self.h.os,'getegid',return_value=0))
        self.stack.enter_context(mock.patch.object(self.h.tempfile,'mkdtemp',return_value=str(scope.policy)))
        self.stack.enter_context(mock.patch.object(Path,'mkdir'))
        def remove(path):
            self.assertEqual(path,scope.policy)
            self.assertFalse(scope.bound or scope.entered)
            self.assertTrue(state['restored'])
            events.append('policy-removed')
        self.stack.enter_context(mock.patch.object(self.h.shutil,'rmtree',side_effect=remove))
        output=[]
        def emit(text):
            self.assertEqual(events[-1],'policy-removed')
            self.assertEqual(closed.call_count,3)
            self.assertNotIn(scope,self.f._UNRESOLVED_POLICY_SCOPES)
            output.append(json.loads(text))
        self.stack.enter_context(mock.patch('builtins.print',side_effect=emit))
        args=['--source-root','/source','--commit',self.o.commit,'--manifest-file','/manifest',
            '--deadline-monotonic-ns',str(self.o.deadline.value),'accept','--checkpoint','/checkpoint',
            '--storage-config',str(self.o.root/'config')]
        self.assertEqual(self.cli.main(args),0)
        self.assertEqual(output[0]['status'],'PASS')
        self.assertEqual(events,['custody-open','custody-check','archive-consumed','custody-check',
            'cleanup-phase','dispose','custody-close','policy-removed'])

    def test_c_caller_retains_policy_on_actual_cleanup_failure_after_work(self):
        scope,libc,closed,state,_,_=self.scope_effects()
        self.stack.enter_context(mock.patch.object(self.f,'ServiceStatusScope',return_value=scope))
        self.stack.enter_context(mock.patch.object(self.f,'validated_inputs',return_value=(Path('/source'),b'manifest',self.o.deadline)))
        self.stack.enter_context(mock.patch.object(self.f,'bounded_run',return_value=contextlib.nullcontext()))
        self.stack.enter_context(mock.patch.object(self.h,'phase_deadline'))
        self.stack.enter_context(mock.patch.object(self.h.os,'getegid',return_value=0))
        self.stack.enter_context(mock.patch.object(self.h.tempfile,'mkdtemp',return_value=str(scope.policy)))
        self.stack.enter_context(mock.patch.object(Path,'mkdir'))
        remove=self.stack.enter_context(mock.patch.object(self.h.shutil,'rmtree'))
        def bootstrap(*_a,**_k):
            self.assertTrue(scope.bound)
            # Failure is from the real shared close, after the work result exists.
            libc.umount2.side_effect=None;libc.umount2.return_value=-1
            return Path('/checkpoint')
        self.stack.enter_context(mock.patch.object(self.h,'bootstrap',side_effect=bootstrap))
        args=types.SimpleNamespace(phase='bootstrap',source_root='/source',commit=self.o.commit,
            manifest_file='/manifest',deadline_monotonic_ns=self.o.deadline.value,
            approved_base='/base',raw_root='/raw',daily_config='/config',generic_bundle='/bundle',generic_session='2026-11-10')
        with self.assertRaisesRegex(self.f.FixtureError,'FIXTURE_POLICY_UNMOUNT'):
            self.h.run(args)
        self.assertIn(scope,self.f._UNRESOLVED_POLICY_SCOPES)
        remove.assert_not_called();closed.assert_not_called()
        libc.umount2.side_effect=lambda *_:state.update(mounted=False) or 0
        scope.__exit__(None,None,None)


if __name__ == '__main__':
    unittest.main()
