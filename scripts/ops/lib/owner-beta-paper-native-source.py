"""Private native4 fixture preparation. Never official archive provenance.

A single registered private owner prepares the unpublished OFF-reader, installs
its own exact service, acquires one real future session per bounded invocation,
and consumes the four native cases before actual service/DB/root disposal.
All observations are implementation evidence and never archive authority.
"""
from __future__ import annotations

import contextlib
import importlib.util
import os
from pathlib import Path
import re
import sys
import time

OWNER_MARKER = 'native-source-owner.json'
CHECKPOINT = 'native-source-checkpoint.json'
SCHEMA = 'private-native-source-fixture-v1'
PREFIX = 'owner_beta::paper::operations::protected::tests::'
CASES = (
    ('actual_native_source_join_requires_post_anchor_arm', b'post-anchor\n'),
    ('actual_native_source_join_rejects_pre_reset_arm_despite_later_db_writes', b'pre-reset-arm-post-reset-db\n'),
    ('actual_native_readback_invalid_current_daily_off_expired_without_worker', b'readback-offline-committed\n'),
    ('actual_native_absent_or_unproven_readback_is_commit_unknown', b'readback-unresolved\n'),
)
_OWNERS = set()
_UNRESOLVED_BOOTSTRAPS = []
_UNRESOLVED_PREPARATIONS = set()


class NativeFixtureError(Exception):
    """Closed payload-free diagnostic only."""


def require(value, code):
    if not value:
        raise NativeFixtureError(code)


def history():
    name = 'owner_beta_paper_genuine_history'
    path = Path(__file__).with_name('owner-beta-paper-genuine-history.py')
    if name not in sys.modules:
        spec = importlib.util.spec_from_file_location(name, path)
        module = importlib.util.module_from_spec(spec)
        sys.modules[name] = module
        spec.loader.exec_module(module)
    module = sys.modules[name]
    require(Path(module.__file__).resolve() == path.resolve(), 'NATIVE_MODULE')
    return module


def test_argv(name):
    require(name in {item[0] for item in CASES}, 'NATIVE_TEST_SELECTION')
    return [PREFIX + name, '--exact', '--include-ignored', '--test-threads=1', '--show-output']


def named_success(output, name, exit_code):
    """Only an actual named libtest result and exactly one nonignored pass.

    This parser is diagnostic, not fixture/Source authority. A caller must also
    prove input and compiled identities plus backend/child cleanup separately.
    """
    test_argv(name)
    require(type(output) is bytes and len(output) <= 65536 and exit_code == 0,
            'NATIVE_TEST_FAILED')
    try:
        lines = output.decode('utf-8', 'strict').splitlines()
    except UnicodeError:
        raise NativeFixtureError('NATIVE_TEST_OUTPUT') from None
    expected = 'test ' + PREFIX + name + ' ... ok'
    require(lines.count(expected) == 1, 'NATIVE_NAMED_RESULT')
    results = [line for line in lines if line.startswith('test result:')]
    require(len(results) == 1 and re.fullmatch(
        r'test result: ok\. 1 passed; 0 failed; 0 ignored; 0 measured; [0-9]+ filtered out; finished in [0-9.]+s',
        results[0]) is not None, 'NATIVE_TEST_SUMMARY')
    require([line for line in lines if line.startswith('test ')
             and not line.startswith('test result:')] == [expected], 'NATIVE_TEST_SELECTION')


class Preparation:
    """Registered original owner for one pre-publication callback.

    Actual FixtureOwner.setup creates the unpublished stage and DB. The callback
    receives that stage; there is no caller stage selector or synthetic Context.
    This owner never exports an ENABLE authorization or checked session facts.
    """
    def __init__(self, owner, material, config_source, *, source_inputs=None):
        h = history()
        require(type(owner) is h.fixture_module().FixtureOwner, 'NATIVE_OWNER')
        require(owner.context is None and owner.observer is None and owner.lock_fd is None,
                'NATIVE_OWNER_ALREADY_SETUP')
        self.owner, self.material = owner, material
        self.source_inputs = source_inputs
        self.config_source = Path(config_source)
        self.runtime = None
        self.entered = self.used = self.published = self.closed = False
        self.config_fd = None
        self.config_pin = None
        self.marker = h.canonical({'schema': SCHEMA, 'root_identity': list(owner.root_id),
            'commit': owner.commit, 'fixture_parent_origin': 'PRIVATE_IMPLEMENTATION',
            'sql_origin': 'LIBTEST_IMPLEMENTATION', 'archive_acceptance': False})

    def __enter__(self):
        require(not self.entered and not self.closed, 'NATIVE_OWNER_REUSE')
        h = history()
        self.stack = contextlib.ExitStack()
        try:
            e = h.source_module().evidence()
            self.config_fd = self.stack.enter_context(e.opened(self.config_source, exact=(0, 0, 0o600)))
            self.config_pin = os.fstat(self.config_fd)
            require(self.config_pin.st_size <= 16384, 'NATIVE_CONFIG')
            self.config_raw = os.pread(self.config_fd, 16385, 0)
            h.fixture_module().ops.daily.install.validate_config(self.config_raw, self.owner.commit)
            h.fixture_module().write_new(self.owner.root / OWNER_MARKER, self.marker, 0o400)
            _OWNERS.add(self)
            self.entered = True
            self.revalidate()
            return self
        except BaseException:
            self.stack.close()
            _OWNERS.discard(self)
            self.closed = True
            raise

    def revalidate(self, *, cleanup=False):
        require(self in _OWNERS and self.entered and not self.closed, 'NATIVE_OWNER_CLOSED')
        h = history()
        f = h.fixture_module()
        o = self.owner
        o.deadline.seconds(cleanup=cleanup)
        require(f.faults.directory_identity(o.root) == o.root_id, 'NATIVE_ROOT_CHANGED')
        require(f.ops.daily.install.read_root_file(str(o.root / OWNER_MARKER), gid=0, mode=0o400)
                == self.marker, 'NATIVE_ORIGIN_CHANGED')
        identity = lambda s: (s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns, s.st_ctime_ns,
                              s.st_uid, s.st_gid, s.st_mode, s.st_nlink)
        require(identity(os.fstat(self.config_fd)) == identity(self.config_pin)
                == identity(os.stat(self.config_source, follow_symlinks=False))
                and os.pread(self.config_fd, 16385, 0) == self.config_raw, 'NATIVE_CONFIG_CHANGED')
        self.material.revalidate()
        revalidate_acquisition(self)
        if o.lock_fd is not None:
            f.ops._file(o.lock_fd, 0o600)
            require(f.ops._identity(os.fstat(o.lock_fd)) == f.ops._identity(
                os.stat(o.root / f.ops.LOCK_NAME, follow_symlinks=False)), 'NATIVE_LOCK_CHANGED')

    def prepare(self, owner, stage):
        self.revalidate()
        require(owner is self.owner and not self.used and not self.published, 'NATIVE_CALLBACK_REUSE')
        require(owner.context is None and not hasattr(owner, 'release')
                and owner.observer is not None and owner.network is not None and owner.pg is not None,
                'NATIVE_PREPUBLICATION_ORDER')
        require(stage.parent == owner.root / 'releases'
                and stage.name.startswith('.staging.' + owner.commit + '.'), 'NATIVE_STAGE_OWNER')
        self.used = True  # A failed callback cannot be reused after partial writes.
        owner.observer.target()
        h = history()
        install = h.fixture_module().ops.daily.install
        policy = stage / 'configs/evidence/owner-beta-daily-price-policy-v1.json'
        install.stage_config(str(stage), str(self.config_source), str(policy), owner.commit)
        require(install.verify_config(str(stage), owner.commit)[1] == h.digest(self.config_raw),
                'NATIVE_STAGE_CONFIG')
        # Uses actual DB/password originals for role views, but no installed
        # Context and no producer start. All mutations are in this owned fixture.
        self.runtime = h._initialize_runtime_material(owner, self.material, self.config_source, policy)
        if self.source_inputs is not None:
            prepare_acquisition(self, stage, self.source_inputs)
        require(owner.context is None and not hasattr(owner, 'release'), 'NATIVE_CONTEXT_BEFORE_PUBLICATION')
        self.revalidate()

    def after_publish(self):
        self.revalidate()
        require(self.used and not self.published and self.runtime is not None,
                'NATIVE_PUBLICATION_ORDER')
        o = self.owner
        h = history()
        require(o.release == o.root / 'releases' / o.commit and o.context is not None,
                'NATIVE_PUBLICATION_REQUIRED')
        with h.fixture_module().ops.release.verify(str(o.root), str(o.release), o.commit) as verified:
            require(verified.process_sha256 is None, 'NATIVE_OFF_READER_REQUIRED')
            verified.revalidate()
        install = h.fixture_module().ops.daily.install
        require(install.verify_config(str(o.release), o.commit)[1] == h.digest(self.config_raw),
                'NATIVE_PUBLISHED_CONFIG_CHANGED')
        self.runtime.config_root = o.release / install.CONFIG_DIRECTORY
        self.published = True
        return self.runtime

    def close_after_disposal(self):
        # Only FixtureOwner's actual disposal removes this exact private root.
        require(self.owner.cleanup.get('containers') == 'REMOVED'
                and self.owner.cleanup.get('inputs') == 'REMOVED_AFTER_DISPOSITION'
                and not os.path.lexists(self.owner.root), 'NATIVE_DISPOSAL_UNPROVEN')
        self.stack.close()
        self.closed = True
        _OWNERS.discard(self)
        _UNRESOLVED_PREPARATIONS.discard(self)

    def __exit__(self, exc_type, exc, tb):
        try:
            if self.runtime is not None:
                self.runtime.close()
            self.revalidate(cleanup=True)
        except BaseException:
            _UNRESOLVED_PREPARATIONS.add(self)
            raise
        self.stack.close()
        self.closed = True
        _OWNERS.discard(self)



def bootstrap(source, commit, manifest, deadline, *, approved_base, raw_root, config_source,
              generic_bundle, generic_session, policy_root, source_inputs=None):
    """Concrete unconfigured Daily-capable OFF reader, never native4 readiness."""
    h = history()
    f = h.fixture_module()
    owner = f.FixtureOwner(source, commit, manifest, deadline)
    scope = contextlib.ExitStack()
    preparation = None
    try:
        scope.enter_context(owner.extractions)
        material = scope.enter_context(h.HistoricalMaterial(source, approved_base, raw_root, deadline))
        require(source_inputs is not None, 'NATIVE_ACQUISITION_REQUIRED')
        store = owner.root / SERVICE_STORE
        store.mkdir(mode=0o700)
        (store / 'collector').mkdir(mode=0o700)
        f.write_new(store / 'collector.lock', b'lock\n', 0o600)
        preparation = Preparation(owner, material, config_source, source_inputs=source_inputs).__enter__()
        owner.setup(policy_root, _prepare_release=preparation.prepare)
        runtime = preparation.after_publish()
        # Prepare actual Daily DB control only after publication.
        h.seed_user_and_control(runtime)
        pin = h.collect_generic(runtime, generic_session, generic_bundle)
        runtime.start_api(pin)
        beta_strategy = h.create_beta_config(runtime)
        runtime.close()
        activate_daily(owner)
        create_binding(owner, runtime.config, beta_strategy)
        persisted_policy = store / 'policy'
        persisted_policy.mkdir(mode=0o700)
        (persisted_policy / 'service-status').mkdir(mode=0o700)
        f.write_new(persisted_policy / 'service-status/policy.json',
            (Path(policy_root) / 'service-status/policy.json').read_bytes(), 0o600)
        f.write_new(store / 'inputs.json', h.canonical(dict(approved_base=str(Path(approved_base)),
            raw_root=str(Path(raw_root)), config_source=str(Path(config_source)),
            acquisition=preparation.acquisition)), 0o400)
        owner.observer.target()
        with f.ops.open_context(owner.context, owner.lock_fd) as reader:
            owner.contexts.append(reader)
            with reader.probe_history() as inventory:
                require(not inventory._configured and not inventory._history,
                        'NATIVE_EXPECTED_UNCONFIGURED')
        with f.ops.journal.open_journal(str(owner.root), owner.lock_fd, bootstrap=True) as journal:
            require(journal.epoch_floor == 0, 'NATIVE_INITIAL_JOURNAL')
            journal.revalidate()
        # This locator deliberately has no account, process proposal,
        # Source facts or readiness field. It cannot resume as C.
        checkpoint = {'schema': SCHEMA, 'commit': commit,
            'root_identity': list(owner.root_id), 'project': owner.project,
            'network': owner.network, 'postgres': owner.pg, 'postgres_image': owner.pg_image,
            'postgres_generation': list(owner.observer.generation),
            'manifest_sha256': h.digest(manifest), 'config_sha256': h.digest(preparation.config_raw),
            'release_hashes': owner.release_hashes, 'producer_hashes': owner.producer_hashes,
            'facts': {'strategy_config_id': beta_strategy, 'owner_user_id': runtime.config['owner'],
                      'generic_dataset': pin}}
        f.write_new(owner.root / CHECKPOINT, h.canonical(checkpoint), 0o400)
        preparation.revalidate()
        owner.extractions.reconcile()
        preparation.__exit__(None, None, None)
        scope.close()
        os.close(owner.lock_fd)
        owner.lock_fd = None
        return {'status': 'BOOTSTRAPPED_UNCONFIGURED', 'checkpoint': str(owner.root / CHECKPOINT),
                'archive_acceptance': False, 'native_cases': 'NOT_RUN', 'source_acquisition': 'NOT_STARTED'}
    except BaseException:
        deadline.cleanup_phase()
        try:
            # Dispose actual owned helpers before closing borrowed material.
            # If any child/backend remains unknown, preserve the whole scope.
            if preparation is not None and preparation.runtime is not None:
                preparation.runtime.close()
            owner.dispose(lambda: None)
        except BaseException:
            _UNRESOLVED_BOOTSTRAPS.append((owner, scope, preparation))
            raise NativeFixtureError('NATIVE_BOOTSTRAP_CLEANUP_UNKNOWN') from None
        # The root is gone only after its actual children/backends were proved
        # closed. Preparation's normal revalidation cannot run against a removed
        # root; close its held input scope explicitly, then the external material.
        if preparation is not None:
            preparation.close_after_disposal()
        scope.close()
        raise


_PHASES = set()
_SCHEDULERS = {}
_UNRESOLVED_PHASES = set()


class OwnedPhase:
    """Reopen one native fixture's actual originals under its original lock.

    The checkpoint is a locator. It neither creates a Source parent nor proves
    native eligibility. Each bounded invocation must reopen these resources;
    an expired invocation cannot extend itself across trading days.
    """
    def __init__(self, source, commit, manifest, deadline, checkpoint, policy_root):
        self.source, self.commit, self.manifest, self.deadline = Path(source), commit, manifest, deadline
        self.path, self.policy_root = Path(checkpoint), Path(policy_root)
        self.stack = contextlib.ExitStack()
        self.owner = None
        self.closed = False
        self.originals = []

    def __enter__(self):
        import fcntl
        h = history()
        f = h.fixture_module()
        e = h.source_module().evidence()
        require(self.path.is_absolute() and self.path.name == CHECKPOINT
                and self.path.parent.parent == Path('/var/lib')
                and self.path.parent.name.startswith('lagrange-paper-operations-fixture-'), 'NATIVE_CHECKPOINT_PATH')
        h.phase_deadline(self.deadline.value)
        try:
            def retain(path):
                fd = self.stack.enter_context(e.opened(path, exact=(0, 0, 0o400)))
                info = os.fstat(fd)
                require(info.st_size <= 65536, 'NATIVE_CHECKPOINT')
                raw = os.pread(fd, 65537, 0)
                self.originals.append((path, fd, info, raw))
                return e.decode(raw)
            value = retain(self.path)
            marker = retain(self.path.parent / OWNER_MARKER)
            require(type(value) is dict and set(value) == {'schema', 'commit', 'root_identity', 'project',
                'network', 'postgres', 'postgres_image', 'postgres_generation', 'manifest_sha256',
                'config_sha256', 'release_hashes', 'producer_hashes', 'facts'} and value['schema'] == SCHEMA,
                'NATIVE_CHECKPOINT')
            require(value['commit'] == self.commit and value['manifest_sha256'] == h.digest(self.manifest)
                    and tuple(value['root_identity']) == f.faults.directory_identity(self.path.parent),
                    'NATIVE_CHECKPOINT_CHANGED')
            require(marker == {'schema': SCHEMA, 'root_identity': value['root_identity'], 'commit': self.commit,
                'fixture_parent_origin': 'PRIVATE_IMPLEMENTATION', 'sql_origin': 'LIBTEST_IMPLEMENTATION',
                'archive_acceptance': False}, 'NATIVE_ORIGIN_CHANGED')
            h.image_manifest(self.manifest, self.commit)

            class RetainedNativeOwner(f.FixtureOwner):
                def __init__(inner):
                    # Same explicit reopen discipline as genuine-history, with
                    # a distinct native-only schema and no configured assertion.
                    inner.source, inner.commit, inner.manifest, inner.deadline = self.source, self.commit, self.manifest, self.deadline
                    inner.root, inner.root_id, inner.project = self.path.parent, tuple(value['root_identity']), value['project']
                    inner.docker = f.FixtureDocker(self.deadline)
                    inner.pg_image, inner.network, inner.pg = value['postgres_image'], value['network'], value['postgres']
                    inner.network_sent = inner.pg_sent = True
                    inner.context = inner.lock_fd = inner.observer = None
                    inner.contexts, inner.tracees, inner.threads, inner.blockers = [], [], [], []
                    inner.context_modules, inner.helpers, inner.helper_bindings = {}, {}, {}
                    inner.stop_writer, inner.stop_writer_sent = None, False
                    inner.cancel_event = f.threading.Event()
                    inner.original_custody_uncertain = False
                    inner.cleanup = {'containers': 'PENDING', 'backends': 'PENDING', 'inputs': 'RETAINED', 'extractions': 'NOT_STARTED'}
                    inner.extractions = f.P3ExtractionCustody(inner)
                    inner.release = inner.root / 'releases' / inner.commit
                    inner.release_hashes, inner.producer_hashes = value['release_hashes'], value['producer_hashes']

            o = self.owner = RetainedNativeOwner()
            o.lock_fd = os.open(o.root / f.ops.LOCK_NAME, os.O_RDWR | os.O_CLOEXEC | os.O_NOFOLLOW)
            self.stack.callback(os.close, o.lock_fd)
            f.ops._file(o.lock_fd, 0o600)
            fcntl.flock(o.lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            if self.policy_root == o.root / SERVICE_STORE / 'policy':
                native_policy_bytes(o, self.policy_root)
            else:
                h.private_policy(o, self.policy_root)
            require(f.ops.daily.install.read_root_file(str(o.release / f.ops.release.MANIFEST_NAME),
                gid=0, mode=0o600, maximum=65536) == self.manifest, 'NATIVE_RELEASE_CHANGED')
            selected = self.release_guard = self.stack.enter_context(
                f.ops.release.verify(str(o.root), str(o.release), o.commit))
            require(selected.process_sha256 is None and h.release_descriptors(o, selected)
                    == (o.release_hashes, o.producer_hashes), 'NATIVE_RELEASE_CHANGED')
            selected.revalidate()
            identity_path = o.root / f.ops.journal.PAPER_DIR / 'installation-id'
            identity_fd = self.stack.enter_context(e.opened(identity_path, exact=(0, 0, 0o600)))
            identity_raw = os.pread(identity_fd, 65, 0)
            self.originals.append((identity_path, identity_fd, os.fstat(identity_fd), identity_raw))
            with f.ops.journal.open_journal(str(o.root), o.lock_fd) as actual_journal:
                require(identity_raw == (actual_journal.installation_id + '\n').encode('ascii'), 'NATIVE_JOURNAL_IDENTITY')
                self.installation = actual_journal.installation_id
                self.floor = actual_journal.epoch_floor
            config, config_sha = f.ops.daily.install.verify_config(str(o.release), o.commit)
            require(config_sha == value['config_sha256'], 'NATIVE_CONFIG_CHANGED')
            values, overlays = f.ops.daily.read_context(str(o.release), o.commit, 'disable')
            require(values['LAGRANGE_DATA_DIR'] == str(o.root / 'data')
                    and values['LAGRANGE_ARTIFACTS_DIR'] == str(o.root / 'artifacts')
                    and values['LAGRANGE_RUNTIME_SECRET_DIR'] == str(o.root / 'secrets'), 'NATIVE_RELEASE_ROOTS')
            o.context = f.ops.daily.Context(str(o.release), o.commit,
                f.ops.daily.read_manifest(str(o.release), o.commit), values, overlays)
            o.observer = f.PgObserver(o)
            o.observer.generation = tuple(value['postgres_generation'])
            o.observer.target()
            self.config, self.config_sha, self.facts = config, config_sha, value['facts']
            require(type(self.facts) is dict and set(self.facts) == {'strategy_config_id', 'owner_user_id', 'generic_dataset'}
                    and self.facts['owner_user_id'] == config['owner'], 'NATIVE_CHECKPOINT_FACTS')
            _PHASES.add(self)
            self.revalidate()
            self.quiescent()
            return self
        except BaseException:
            _PHASES.discard(self)
            self.stack.close()
            raise

    def revalidate(self, *, cleanup=False):
        h = history()
        f = h.fixture_module()
        o = self.owner
        require(self in _PHASES and not self.closed and o is not None and o.lock_fd is not None,
                'NATIVE_PHASE_CLOSED')
        o.deadline.seconds(cleanup=cleanup)
        require(f.faults.directory_identity(o.root) == o.root_id and not o.original_custody_uncertain,
                'NATIVE_OWNER_CHANGED')
        identity = lambda s: (s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns, s.st_ctime_ns,
                              s.st_mode, s.st_uid, s.st_gid, s.st_nlink)
        for path, fd, before, raw in self.originals:
            require(identity(before) == identity(os.fstat(fd)) == identity(os.stat(path, follow_symlinks=False))
                    and raw == os.pread(fd, 65537, 0), 'NATIVE_ORIGINAL_CHANGED')
        require(f.ops._identity(os.fstat(o.lock_fd)) == f.ops._identity(
            os.stat(o.root / f.ops.LOCK_NAME, follow_symlinks=False)), 'NATIVE_LOCK_CHANGED')
        if hasattr(self, 'release_guard'):
            self.release_guard.revalidate()
            with f.ops.journal.open_journal(str(o.root), o.lock_fd) as journal:
                require(journal.installation_id == self.installation and journal.epoch_floor >= self.floor,
                        'NATIVE_JOURNAL_CHANGED')
                self.floor = journal.epoch_floor
                journal.revalidate()
        o.observer.target(cleanup=cleanup)

    def quiescent(self, *, cleanup=False):
        self.revalidate(cleanup=cleanup)
        o = self.owner
        network = o.docker.object('network', o.network)
        require(set((network.get('Containers') or {}).keys()) == {o.pg}, 'NATIVE_CONTAINERS_UNSETTLED')
        state = o.observer.json("SELECT json_build_object('clients',(SELECT count(*) FROM pg_stat_activity "
            "WHERE datname=current_database() AND backend_type='client backend' AND pid<>pg_backend_pid()),"
            "'prepared',(SELECT count(*) FROM pg_prepared_xacts WHERE database=current_database()));", cleanup=cleanup)
        require(type(state) is dict and set(state) == {'clients', 'prepared'}
                and all(type(v) is int and v == 0 for v in state.values()), 'NATIVE_BACKENDS_UNSETTLED')

    def process_proposal(self, binding):
        """Untrusted proposal from actual control/consent and held release.

        This is not an installed process.json and never enters its location.
        The official native observe-reset still verifies its own ELF/capability
        and every Current/selection/original independently.
        """
        import uuid
        self.revalidate()
        h = history()
        o = self.owner
        c = o.observer.json('SELECT to_jsonb(c) FROM public.owner_beta_paper_control c;', role='migration_owner')
        require(type(c) is dict and c['state'] in ('OFF', 'RESTORED_OFF', 'ENABLED')
                and c['owner_user_id'] == binding['cohort']['owner_user_id']
                and c['strategy_config_id'] == binding['cohort']['strategy_config_id'], 'NATIVE_CONTROL_CHANGED')
        account = str(uuid.UUID(c['account_id']))
        consent = str(uuid.UUID(c['consent_event_id']))
        proposal = dict(schema='owner-beta-paper-process-v1', owner_user_id=c['owner_user_id'], account_id=account,
            strategy_config_id=c['strategy_config_id'], consent_event_id=consent, consent_version=c['contract_id'],
            control_epoch=c['control_epoch'] + 1, daily_cohort_sha256=binding['cohort']['cohort_sha256'],
            financial_contract_sha256=c['contract_sha256'], code_commit=o.commit,
            release_manifest_sha256=h.digest(o.manifest), api_binary_sha256=o.release_hashes['api_binary_sha256'],
            worker_binary_sha256=o.release_hashes['worker_binary_sha256'])
        h.fixture_module().ops.release._validate_process(proposal, o.commit, h.digest(o.manifest))
        self.revalidate()
        return proposal, h.canonical(proposal)

    def source_barrier(self, scheduler):
        """Constructor-owned private barrier; production never selects this.

        Inventory and reset/anchor originals come from real native invocations.
        There is no ready flag and no replacement for independent native proof.
        """
        self.revalidate()
        require(_SCHEDULERS.get(scheduler) is self, 'NATIVE_SOURCE_OWNER')
        h = history()
        l = h.lifecycle_module()
        o = self.owner
        require(scheduler.authority is not None and scheduler.b['release_root'] == str(o.release)
                and scheduler.b['data_root'] == str(o.root / 'data')
                and scheduler.b['cohort']['verifier_revision'] == o.commit
                and scheduler.b['cohort']['release_manifest_sha256'] == h.digest(o.manifest), 'NATIVE_SOURCE_OWNER')
        scheduler.authority.poll(check_assets=True)
        context = h.lifecycle_context(o, l)
        with l.open_lifecycle(context, o.lock_fd, deadline=o.deadline.value) as controller:
            o.contexts.append(controller.context)
            o.context_modules[controller.context] = l.ops
            controller.deadline.tick = lambda: scheduler.authority.poll(check_assets=True)
            with l.ops._inspect_generation_null(controller.context) as call:
                _index, _pages, _history, configured = l.ops._read_inventory(call)
                call._imported = True
            if not configured:
                # Actual unconfigured inventory is the sole no-anchor outcome.
                result = None
            else:
                proposal, raw = self.process_proposal(scheduler.b)
                journal = controller._journal()
                epoch = proposal['control_epoch'] - 1
                require(journal.epoch_floor == epoch, 'NATIVE_JOURNAL_EPOCH')
                intent = l._intent(journal.installation_id, h.digest(o.manifest), 'INSPECT', proposal['account_id'], epoch)
                query = h.canonical(dict(schema='owner-beta-paper-exact-request-v1', kind='RESET',
                    account_id=proposal['account_id'], args={'expected_epoch': epoch}))
                read = controller._run(l._Call(controller, 'readback', intent=h.canonical(intent), sql=query))
                require(read.outcome == 'OK' and read.cleanup == 'COMPLETE' and read.readback is not None,
                        'NATIVE_RESET_UNPROVEN')
                reset = l._exact_readback(read.readback, query)
                require(reset['status'] == 'RESET' and reset['current_state'] in ('OFF', 'RESTORED_OFF'),
                        'NATIVE_RESET_STATE')
                selection = dict(schema='owner-beta-paper-reset-selection-v1', installation_id=journal.installation_id,
                    reset=reset['reset'], release_manifest_sha256=h.digest(o.manifest),
                    cohort_sha256=proposal['daily_cohort_sha256'])
                controller.store.publish('selections', h.canonical(selection))
                boot = Path('/proc/sys/kernel/random/boot_id').read_text().strip()
                anchors = controller._anchors(selection)
                same_boot = [item for item in anchors if l._anchor(item, selection)['boot_id'] == boot]
                require(len(same_boot) <= 1, 'NATIVE_ANCHOR_AMBIGUOUS')
                if same_boot:
                    result = l.LifecycleResult('REPLAY', None, None, same_boot[0], 'COMPLETE')
                else:
                    intent = l._intent(journal.installation_id, h.digest(o.manifest), 'INSPECT',
                        proposal['account_id'], epoch, raw)
                    result = controller._run(l._Call(controller, 'observe-reset', intent=h.canonical(intent),
                        evidence=raw, selection=selection))
                    require(result.outcome == 'OK' and result.cleanup == 'COMPLETE' and result.artifact is not None,
                            'NATIVE_ANCHOR_UNPROVEN')
                    require(l._anchor(result.artifact, selection)['boot_id'] == boot, 'NATIVE_ANCHOR_BOOT')
                    controller.pending_anchors.append(result.artifact)
        self.revalidate()
        scheduler.authority.poll(check_assets=True)
        return result

    def configure_after_arm(self, scheduler, runtime):
        """Actual native CONFIGURE/HTTP consent between retained ARM and due."""
        self.revalidate()
        require(_SCHEDULERS.get(scheduler) is self and runtime.owner is self.owner, 'NATIVE_SOURCE_OWNER')
        h = history()
        e = h.source_module().evidence()
        e.verify_chain(scheduler.j.events, 'ARMED')
        require(scheduler.j.events[-1]['arm']['parent'] == e.process(os.getpid()), 'NATIVE_PARENT_CHANGED')
        rows = self.owner.observer.json("SELECT coalesce(jsonb_agg(c), '[]'::jsonb) FROM public.owner_beta_paper_control c;",
                                       role='migration_owner')
        require(type(rows) is list and len(rows) <= 1, 'NATIVE_CONTROL_CHANGED')
        if rows:
            require(rows[0]['strategy_config_id'] == self.facts['strategy_config_id'], 'NATIVE_CONTROL_CHANGED')
            return
        require(scheduler.b['cohort']['strategy_config_id'] == self.facts['strategy_config_id'], 'NATIVE_SOURCE_OWNER')
        runtime.start_api(self.facts['generic_dataset'])
        try:
            l = h.lifecycle_module()
            context = h.lifecycle_context(self.owner, l)
            contract = h.fixture_module().existing_json_macro(
                self.owner.release / 'tests/integration/migration-contract/tests/paper_0067.rs', 'financial_contract')
            proposal = h.canonical(dict(schema='owner-beta-paper-configure-operation-v1',
                strategy_config_id=self.facts['strategy_config_id'], account_name='Owner-beta Paper', contract=contract))
            with l.open_lifecycle(context, self.owner.lock_fd, deadline=self.deadline.value) as controller:
                self.owner.contexts.append(controller.context)
                self.owner.context_modules[controller.context] = l.ops
                controller.deadline.tick = lambda: scheduler.authority.poll(check_assets=True)
                configured = controller.configure('native-source-configure-after-arm', proposal)
            require(configured.cleanup == 'COMPLETE' and configured.outcome == 'OK'
                    and configured.readback is not None, 'NATIVE_CONFIGURE_UNPROVEN')
            actual = h.fixture_module().ops.journal._json(configured.readback)
            require(actual['status'] == 'COMMITTED' and actual['current_state'] == 'OFF'
                    and actual['current_epoch'] == 0, 'NATIVE_CONFIGURE_UNPROVEN')
            scheduler.authority.poll(check_assets=True)
            h.consent(runtime, 0, opt_in=True)
            scheduler.authority.poll(check_assets=True)
            # Observe the actual CONFIGURED reset now. The later negative test
            # independently proves ARM < RESET and all five writes > RESET.
            self.source_barrier(scheduler)
        finally:
            runtime.close()
        self.revalidate()

    @contextlib.contextmanager
    def session(self, binding_path, runtime):
        """One real future Source session within this invocation's deadline.

        The context yields only after genuine RUN_LINKED/LINKED. It retains the
        real publisher/Source lock while an independent native consumer runs.
        No background lease, time change or multi-day waiting is introduced.
        Activation by the owned private service adapter is still required.
        """
        import datetime
        import fcntl
        import signal
        self.revalidate()
        require(runtime.owner is self.owner and runtime.config == self.config, 'NATIVE_RUNTIME_OWNER')
        h = history()
        e = h.source_module().evidence()
        phase = self

        class NativeScheduler(e.Scheduler):
            def source_lifecycle_barrier(inner):
                return phase.source_barrier(inner)
            def armed(inner):
                phase.configure_after_arm(inner, runtime)
            def source_finished(inner):
                # Source itself publishes admission/run links after this hook.
                # Waiting for those links here would deadlock the same parent.
                try:
                    h.daily_produce(runtime)
                finally:
                    runtime.close()
                inner.guard()
            def command(inner, argv, timeout):
                return super().command(argv, phase.deadline.seconds(timeout, cleanup=inner.in_cleanup))
            def guard(inner):
                phase.deadline.seconds()
                return super().guard()
            def run(inner):
                # An accidental call cannot enter the normal unbounded loop.
                raise NativeFixtureError('NATIVE_USE_OWNED_SESSION_SCOPE')

        scheduler = NativeScheduler(str(binding_path))
        b = scheduler.b
        require(b['release_root'] == str(self.owner.release) and b['project'] == self.owner.project
                and b['data_root'] == str(self.owner.root / 'data')
                and b['secret_root'] == str(self.owner.root / 'secrets')
                and b['cohort']['strategy_config_id'] == self.facts['strategy_config_id']
                and b['cohort']['owner_user_id'] == self.facts['owner_user_id'], 'NATIVE_SOURCE_OWNER')
        network = self.owner.docker.object('network', self.owner.network)
        require(network['Name'] == b['network'] and network['Id'] == self.owner.network
                and network['Internal'] is True, 'NATIVE_SOURCE_NETWORK')
        _SCHEDULERS[scheduler] = self
        original_command = self.owner.command
        old_docker = scheduler.docker.call
        handlers = {}
        def command(args, payload=None, cap=30, cleanup=False, allow_failure=False):
            import tempfile
            stream = None
            try:
                if payload is not None:
                    require(type(payload) is bytes and len(payload) <= 2 * 1024 * 1024, 'NATIVE_COMMAND_INPUT')
                    stream = tempfile.TemporaryFile(dir=self.owner.root)
                    self.stack.callback(stream.close)
                    stream.write(payload)
                    stream.flush()
                    stream.seek(0)
                def tick():
                    self.deadline.seconds(cleanup=cleanup)
                    if not cleanup:
                        scheduler.authority.poll(check_assets=True)
                result = e.guarded_command(self.owner.argv(args), self.deadline.seconds(cap, cleanup=cleanup),
                    tick, scheduler.invalidate, stdin=stream)
                require(allow_failure or result.returncode == 0, 'NATIVE_COMMAND_FAILED')
                return result
            except BaseException as error:
                child = getattr(error, 'child_custody', None)
                if child is not None and not child.reaped:
                    scheduler.command_custodies.append(child)
                    self.owner.original_custody_uncertain = True
                raise
            finally:
                if stream is not None and not self.owner.original_custody_uncertain:
                    stream.close()
        try:
            root = Path(b['evidence_root'])
            with e.opened(root / '.control/lock', write=True, exact=(0, 0, 0o600)) as lock:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                scheduler.authority = e.CurrentAuthority(scheduler, lock)
                scheduler.docker.call = scheduler.docker_call
                self.owner.command = command
                def interrupted(_sig, _frame):
                    raise e.Rejected('INTERRUPTED')
                try:
                    for sig in (signal.SIGTERM, signal.SIGINT):
                        handlers[sig] = signal.signal(sig, interrupted)
                    scheduler.authority.start()
                    today = datetime.datetime.now(e.KST).date()
                    selected = None
                    for offset in range(370):
                        self.deadline.seconds()
                        day = (today + datetime.timedelta(days=offset)).isoformat()
                        due = e.due_ns(day)
                        if due - time.time_ns() < 60 * 10**9:
                            continue
                        # Preserve the inherited 120s cleanup reserve. A future
                        # trading day outside this bounded phase is not awaited.
                        require(due - time.time_ns() < int(self.deadline.seconds(5400) * 10**9),
                                'NATIVE_SESSION_OUTSIDE_PHASE')
                        result = scheduler.command(['/usr/bin/python3', str(self.owner.release / 'scripts/ops/xkrx-calendar-bootstrap.py'),
                            '--emit-sessions', '--start', day, '--end', day, '--output-dir', b['calendar_dir']], 30)
                        require(result.returncode == 0 and len(result.stdout) < 32, 'NATIVE_CALENDAR')
                        meta = e.decode(result.stderr.strip(), 8192, False)
                        require(meta.get('artifact_sha256') == b['cohort']['calendar_sha256'], 'NATIVE_CALENDAR')
                        if result.stdout.decode().splitlines() == [day]:
                            selected = day
                            break
                    require(selected is not None, 'NATIVE_CALENDAR')
                    scheduler.run_session(selected)
                    e.verify_chain(scheduler.j.events, 'LINKED')
                    self.revalidate()
                    yield scheduler
                    # run_session closes its cancellation timer before LINKED.
                    # Current's publisher remains live through this scope.
                    scheduler.authority.poll(check_assets=True)
                    self.revalidate()
                except BaseException as error:
                    scheduler.invalidate(error)
                    raise
                finally:
                    try:
                        scheduler.authority.revoke('STOPPED')
                        runtime.close()
                        require(not scheduler.cleanup_errors and scheduler.current is None
                                and scheduler.current_container is None
                                and (scheduler.owned_child is None or scheduler.owned_child.reaped)
                                and not scheduler.command_custodies, 'NATIVE_SOURCE_CLEANUP_UNKNOWN')
                    except BaseException:
                        self.owner.original_custody_uncertain = True
                        raise
        finally:
            self.owner.command = original_command
            scheduler.docker.call = old_docker
            for sig, handler in handlers.items():
                signal.signal(sig, handler)
            _SCHEDULERS.pop(scheduler, None)

    def __exit__(self, exc_type, exc, tb):
        if self.closed:
            return
        try:
            self.revalidate(cleanup=True)
            o = self.owner
            require(all(c._closed for c in o.contexts) and not o.helpers and not o.helper_bindings
                    and not o.tracees and not o.threads and not o.blockers, 'NATIVE_BORROWERS_OPEN')
            o.observer.absent(cleanup=True)
            self.quiescent(cleanup=True)
        except BaseException:
            # Keep actual descriptors and owned child/input objects on UNKNOWN.
            # A diagnostic is not a permit to dispose a live/uncertain fixture.
            _UNRESOLVED_PHASES.add(self)
            raise
        self.stack.close()
        self.closed = True
        _PHASES.discard(self)
        self.owner.lock_fd = None


_PRODUCERS = ('actual_native_fixture_compiled_identity',
              'produce_actual_native_enable_for_offline_readback',
              'produce_actual_native_absent_enable_original')
_CALLERS = set()
_UNRESOLVED_CALLERS = set()


def fixture_argv(name):
    if name in {item[0] for item in CASES}:
        return test_argv(name)
    require(name in _PRODUCERS, 'NATIVE_TEST_SELECTION')
    return [PREFIX + name, '--exact', '--include-ignored', '--test-threads=1', '--show-output']


def fixture_success(output, name, code):
    # Keep the same exact-result grammar for helper tests, without widening the
    # public four-case selector. Helpers never count as one of the four cases.
    fixture_argv(name)
    if name in {item[0] for item in CASES}:
        return named_success(output, name, code)
    require(type(output) is bytes and len(output) <= 65536 and code == 0, 'NATIVE_TEST_FAILED')
    try:
        lines = output.decode('utf-8', 'strict').splitlines()
    except UnicodeError:
        raise NativeFixtureError('NATIVE_TEST_OUTPUT') from None
    expected = 'test ' + PREFIX + name + ' ... ok'
    require([line for line in lines if line.startswith('test ') and not line.startswith('test result:')]
            == [expected], 'NATIVE_NAMED_RESULT')
    summaries = [line for line in lines if line.startswith('test result:')]
    require(len(summaries) == 1 and re.fullmatch(
        r'test result: ok\. 1 passed; 0 failed; 0 ignored; 0 measured; [0-9]+ filtered out; finished in [0-9.]+s',
        summaries[0]) is not None, 'NATIVE_TEST_SUMMARY')


def database_image(phase):
    """Full real row sets, read in one snapshot. No caller table selector."""
    phase.revalidate()
    tables = ('owner_beta_paper_control', 'owner_beta_paper_events', 'owner_beta_paper_operations',
        'owner_beta_paper_operation_observations', 'owner_beta_paper_recovery_grants', 'accounts',
        'account_strategy_bindings', 'cash_ledger', 'positions', 'orders', 'fills', 'daily_equity',
        'pending_targets', 'paper_rebalance_previews', 'owner_beta_paper_lineage', 'owner_beta_paper_sessions',
        'owner_beta_paper_session_events', 'paper_settlement_outbox', 'paper_settlement_outbox_archive',
        'notifications', 'notification_deliveries', 'jobs', 'job_attempts', 'owner_beta_paper_source_slots',
        'owner_beta_paper_source_admissions', 'owner_beta_paper_source_runs', 'owner_beta_daily_reservations')
    fields = ["'" + table + "',(SELECT COALESCE(jsonb_agg(to_jsonb(t) ORDER BY to_jsonb(t)::text COLLATE \"C\"),"
              "'[]'::jsonb) FROM public." + table + " t)" for table in tables]
    # The owned PostgreSQL observer uses its existing administrative read role,
    # so RLS cannot turn absent actor visibility into a false no-write result.
    value = phase.owner.observer.json('BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY; '
        "SET LOCAL statement_timeout='15000'; SELECT jsonb_build_object(" + ','.join(fields) + '); ROLLBACK;')
    require(type(value) is dict and set(value) == set(tables) and all(type(v) is list for v in value.values()),
            'NATIVE_DATABASE_IMAGE')
    phase.revalidate()
    return value


class NativeCaller:
    """Held untrusted libtest and actual fixture resources for sequential calls.

    This caller is deliberately separate from official _NativeInvocation. It
    neither changes official ELF provenance nor gives a test result authority
    over Source, process installation or archive acceptance.
    """
    def __init__(self, phase, elf_path):
        require(phase in _PHASES and not phase.closed, 'NATIVE_PHASE_CLOSED')
        self.phase, self.path = phase, Path(elf_path)
        self.stack = contextlib.ExitStack()
        self.stages, self.inputs, self.controllers = [], [], []
        self.closed = False
        self.identity_checked = False
        self.runtime = None
        self.results = []
        self.command_custodies = []

    def __enter__(self):
        import hashlib
        h = history()
        e = h.source_module().evidence()
        self.phase.revalidate()
        try:
            self.fd = self.stack.enter_context(e.opened(self.path, exact=(0, 0, 0o500)))
            self.info = os.fstat(self.fd)
            require(0 < self.info.st_size <= 512 * 1024 * 1024 and os.pread(self.fd, 4, 0) == b'\x7fELF',
                    'NATIVE_TEST_ELF')
            hasher = hashlib.sha256()
            offset = 0
            while chunk := os.pread(self.fd, 1024 * 1024, offset):
                self.phase.deadline.seconds()
                hasher.update(chunk)
                offset += len(chunk)
            require(offset == self.info.st_size, 'NATIVE_TEST_ELF')
            self.elf_sha = 'sha256:' + hasher.hexdigest()
            self.runtime = h.producer_runtime(self.phase.owner)
            def command(args, payload=None, *, limit=60, output=None, cleanup=False,
                        include_stderr=False, failure_service=None):
                # Only this owned libtest container receives supplementary read
                # groups; the official launcher and runtime defaults are intact.
                require(payload is None and output is None and failure_service is None,
                        'NATIVE_COMMAND_SELECTION')
                if args[:1] == ['create']:
                    args = [args[0], '--group-add', '10001', *args[1:]]
                def tick():
                    self.phase.deadline.seconds(cleanup=cleanup)
                    if not cleanup:
                        for scheduler, owner in tuple(_SCHEDULERS.items()):
                            if owner is self.phase:
                                scheduler.authority.poll(check_assets=True)
                def failed(_error):
                    for scheduler, owner in tuple(_SCHEDULERS.items()):
                        if owner is self.phase:
                            scheduler.invalidate(_error)
                try:
                    result = e.guarded_command(self.phase.owner.argv(args),
                        self.phase.deadline.seconds(limit, cleanup=cleanup), tick, failed)
                except BaseException as error:
                    child = getattr(error, 'child_custody', None)
                    if child is not None and not child.reaped:
                        self.command_custodies.append(child)
                        self.phase.owner.original_custody_uncertain = True
                        _UNRESOLVED_CALLERS.add(self)
                    raise
                require(result.returncode == 0 and len(result.stdout) + len(result.stderr) <= h.MAX_DOCUMENT,
                        'NATIVE_COMMAND_FAILED')
                return result.stdout + (result.stderr if include_stderr else b'')
            self.runtime.command = command
            _CALLERS.add(self)
            self.revalidate()
            return self
        except BaseException:
            _CALLERS.discard(self)
            self.stack.close()
            raise

    def revalidate(self, *, cleanup=False):
        require(self in _CALLERS and not self.closed, 'NATIVE_CALLER_CLOSED')
        self.phase.revalidate(cleanup=cleanup)
        identity = lambda s: (s.st_dev, s.st_ino, s.st_uid, s.st_gid, s.st_mode, s.st_nlink,
                              s.st_size, s.st_mtime_ns, s.st_ctime_ns)
        require(identity(os.fstat(self.fd)) == identity(self.info)
                == identity(os.stat(self.path, follow_symlinks=False)), 'NATIVE_TEST_ELF_CHANGED')
        for stage in self.stages:
            if not stage.closed:
                stage.revalidate()
        for owner in self.inputs:
            if not owner.closed:
                owner.revalidate()

    def stage(self):
        self.revalidate()
        stage = history().fixture_module().ops._Stage('native4-', root=str(self.phase.owner.root))
        self.stages.append(stage)
        self.stack.callback(stage.close, remove=True)
        return stage

    def execute(self, name, mounts):
        self.revalidate()
        fixture_argv(name)
        require(name == _PRODUCERS[0] or self.identity_checked, 'NATIVE_COMPILED_IDENTITY_REQUIRED')
        runtime = self.runtime
        require(not runtime.created and not runtime.uncertain_creations, 'NATIVE_PREVIOUS_CHILD_OPEN')
        output = None
        try:
            cid = runtime.create('native4-libtest', runtime.images['paper-scheduler'], 0,
                self.phase.owner.network,
                [runtime.mount(self.path, '/run/native4-libtest'), *mounts],
                {'LANG': 'C.UTF-8', 'LC_ALL': 'C.UTF-8', 'TZ': 'UTC'},
                '/run/native4-libtest', fixture_argv(name))
            observed = runtime.inspect('container', cid)
            require(observed['HostConfig'].get('GroupAdd') == ['10001'], 'NATIVE_TEST_CONTAINER')
            # Same inherited deadline; no timeout extension for native work.
            output = runtime.command(['start', '--attach', '--interactive', cid], limit=15)
            state = runtime.inspect('container', cid)['State']
            require(state['Running'] is False and state['OOMKilled'] is False, 'NATIVE_TEST_CHILD_UNKNOWN')
            fixture_success(output, name, state['ExitCode'])
            self.revalidate()
        finally:
            try:
                runtime.close()
                self.phase.owner.observer.absent(cleanup=True)
                self.phase.quiescent(cleanup=True)
                self.revalidate(cleanup=True)
            except BaseException:
                self.phase.owner.original_custody_uncertain = True
                _UNRESOLVED_CALLERS.add(self)
                raise
        if not hasattr(self, 'outputs'):
            self.outputs = {}
        self.outputs[name] = output
        return output

    def check_identity(self):
        self.revalidate()
        require(not self.identity_checked, 'NATIVE_IDENTITY_REUSE')
        output = self.execute(_PRODUCERS[0], [])
        source = self.phase.owner.release / 'crates/job-queue/src/owner_beta/paper/operations/protected.rs'
        e = history().source_module().evidence()
        expected = ('NATIVE_FIXTURE_CODE ' + self.phase.commit + ' ' +
                    history().digest(e.read_file(source, 2 * 1024 * 1024))).encode()
        require([line for line in output.splitlines() if line.startswith(b'NATIVE_FIXTURE_CODE ')] == [expected],
                'NATIVE_COMPILED_IDENTITY')
        self.identity_checked = True
        self.revalidate()

    @contextlib.contextmanager
    def source_inputs(self, scheduler, name):
        """Stage actual selection/anchors and held originals for one Source call."""
        require(_SCHEDULERS.get(scheduler) is self.phase and scheduler.authority is not None,
                'NATIVE_SOURCE_OWNER')
        require(name in {CASES[0][0], CASES[1][0], *_PRODUCERS[1:]}, 'NATIVE_TEST_SELECTION')
        h = history()
        l = h.lifecycle_module()
        o = self.phase.owner
        context = h.lifecycle_context(o, l)
        controller = self.stack.enter_context(l.open_lifecycle(context, o.lock_fd, deadline=o.deadline.value))
        self.controllers.append(controller)
        o.contexts.append(controller.context)
        o.context_modules[controller.context] = l.ops
        controller.deadline.tick = lambda: scheduler.authority.poll(check_assets=True)
        proposal, raw = self.phase.process_proposal(scheduler.b)
        journal = controller._journal()
        epoch = proposal['control_epoch'] - 1
        require(epoch == journal.epoch_floor, 'NATIVE_JOURNAL_EPOCH')
        query = h.canonical(dict(schema='owner-beta-paper-exact-request-v1', kind='RESET',
            account_id=proposal['account_id'], args={'expected_epoch': epoch}))
        inspect = l._intent(journal.installation_id, h.digest(o.manifest), 'INSPECT', proposal['account_id'], epoch)
        result = controller._run(l._Call(controller, 'readback', intent=h.canonical(inspect), sql=query))
        require(result.outcome == 'OK' and result.cleanup == 'COMPLETE', 'NATIVE_RESET_UNPROVEN')
        reset = l._exact_readback(result.readback, query)
        require(reset['status'] == 'RESET', 'NATIVE_RESET_UNPROVEN')
        selection = dict(schema='owner-beta-paper-reset-selection-v1', installation_id=journal.installation_id,
            reset=reset['reset'], release_manifest_sha256=h.digest(o.manifest),
            cohort_sha256=proposal['daily_cohort_sha256'])
        anchors = controller._anchors(selection)
        # Never run positive-only anchor pruning for the pre-reset case.
        require(1 <= len(anchors) <= 3, 'NATIVE_ANCHORS_REQUIRED')
        intent = l._intent(journal.installation_id, h.digest(o.manifest), 'ENABLE', proposal['account_id'], epoch, raw)
        stage = self.stage()
        ops = l.ops
        for directory in ('secrets', 'marker', 'reset-anchors', 'output'):
            os.mkdir(directory, 0o700, dir_fd=stage.fd)
        for leaf, value in (('request.json', h.canonical(intent)), ('evidence.json', raw),
                ('database.json', h.canonical(controller.context.database)),
                ('reset-selection.json', h.canonical(selection))):
            ops._write_file(stage.fd, leaf, value)
        marker = dict(CASES).get(name, b'private-enable-producer\n')
        for folder, values in (('marker', [('native-reset-case', marker),
                (OWNER_MARKER, h.canonical({'schema': SCHEMA, 'root_identity': list(o.root_id),
                    'commit': o.commit, 'fixture_parent_origin': 'PRIVATE_IMPLEMENTATION',
                    'sql_origin': 'LIBTEST_IMPLEMENTATION', 'archive_acceptance': False}))]),
                ('reset-anchors', [(h.digest(value)[7:] + '.json', value) for value in anchors])):
            fd = os.open(folder, ops.journal.DIR_FLAGS, dir_fd=stage.fd)
            try:
                for leaf, value in values:
                    ops._write_file(fd, leaf, value)
            finally:
                os.close(fd)
        inputs = l._Inputs(controller, financial=False)
        self.inputs.append(inputs)
        inputs.open(None)
        require(inputs.binding == scheduler.b, 'NATIVE_SOURCE_CHANGED')
        config, pin = ops.daily.install.verify_config(str(o.release), o.commit)
        require(pin == scheduler.b['daily_config_sha256'], 'NATIVE_CONFIG_CHANGED')
        for host, alias, uid, gid in ((str(o.root / 'data/raw'), '/data/raw', config['raw_owner_uid'], 10001),
                (str(o.root / 'data/owner-beta-daily/admitted'), '/data/owner-beta-daily/admitted', 10003, 10001),
                (str(o.root / 'data/owner-beta-paper-evidence/events'), '/data/owner-beta-paper-evidence/events', 0, 10001)):
            inputs.root(host, uid, gid)
            inputs.mounts.append((host, alias, True))
        config_root = str(o.release / '.owner-beta-daily')
        config_fd = inputs.root(config_root, gid=10001)
        for leaf in ('config.json', 'policy.json'):
            original = l._Original(config_fd, leaf, mode=0o440, gid=10001)
            inputs.originals.append(original)
            inputs.mounts.append((config_root + '/' + leaf, '/run/owner-beta-daily/' + leaf, True))
        for role, service, uid in (('migration_owner', 'db-migrate', 999), ('worker', 'owner-beta-runner', 10001)):
            secret_path = str(o.root / 'secrets' / service)
            if uid == 999:
                # Migration credentials are not a Source/Raw root. Retain their
                # real service directory under its existing 999:999 ownership.
                e = h.source_module().evidence()
                parent = inputs.stack.enter_context(e.opened(secret_path, directory=True,
                    owners=(0, 999), exact=(999, 999, 0o750)))
                pin = ops._identity(os.fstat(parent))
                inputs.checks.append(lambda parent=parent, pin=pin, secret_path=secret_path: require(
                    ops._identity(os.fstat(parent)) == pin == ops._identity(
                        os.stat(secret_path, follow_symlinks=False)), 'NATIVE_SECRET_DIRECTORY_CHANGED'))
            else:
                parent = inputs.root(secret_path, uid, uid)
            original = l._Original(parent, 'db_' + role + '_password', uid=uid, gid=uid)
            inputs.originals.append(original)
            secret = os.open('secrets', ops.journal.DIR_FLAGS, dir_fd=stage.fd)
            try:
                ops._write_file(secret, 'db_' + role + '_password', original.raw)
            finally:
                os.close(secret)
        base = h.base_module()
        view = self.stack.enter_context(base.native_root_view(o.root / 'artifacts/historical-price-beta-root', stage.fd))
        inputs.view = view
        stage.snapshot(mutable_children=('historical', 'output'))
        inputs.revalidate()
        mounts = [(stage.path, '/run/owner-beta-paper-ops', True),
            (stage.path + '/output', '/run/owner-beta-paper-ops/output', False),
            (stage.path + '/secrets', '/run/secrets', True),
            (stage.path + '/marker', '/run/r5-paper-checker-integration', True),
            (stage.path + '/historical', '/data/owner-beta-artifacts', True), *inputs.mounts]
        yield stage, [self.runtime.mount(*item) for item in mounts]
        self.revalidate()

    def preserve_enable_original(self, scheduler, *, commit):
        """Consume the genuine checker; preserve its exact dispatch originals.

        ABSENT uses a different newly generated intent/key and never dispatches
        SQL. These copies retain original canonical bytes, including evaluation
        times; neither branch reconstructs ENABLE evidence from caller JSON.
        """
        require(type(commit) is bool, 'NATIVE_PRODUCER_SELECTION')
        name = _PRODUCERS[1 if commit else 2]
        before = database_image(self.phase)
        h = history()
        l = h.lifecycle_module()
        with self.source_inputs(scheduler, name) as (stage, mounts):
            self.execute(name, mounts)
            if not commit:
                require(database_image(self.phase) == before, 'NATIVE_ABSENT_PRODUCER_MUTATED')
            output_fd = os.open('output', l.journal.DIR_FLAGS, dir_fd=stage.fd)
            self.stack.callback(os.close, output_fd)
            names = sorted(os.listdir(output_fd))
            require(1 <= len(names) <= 8, 'NATIVE_DISPATCH_OUTPUT')
            dispatches = []
            originals = {}
            for leaf in names:
                require(re.fullmatch('[0-9a-f]{64}\\.json', leaf) is not None, 'NATIVE_DISPATCH_OUTPUT')
                original = l._Original(output_fd, leaf, maximum=1048576)
                self.stack.callback(original.close)
                require(h.digest(original.raw)[7:] + '.json' == leaf, 'NATIVE_DISPATCH_HASH')
                value = l.ops._json(original.raw, 1048576)
                originals[h.digest(original.raw)] = original
                if value.get('schema') == 'owner-beta-paper-native-dispatch-v1':
                    dispatches.append((original, value))
            require(len(dispatches) == 1, 'NATIVE_DISPATCH_ORIGINAL')
            descriptor, dispatch = dispatches[0]
            require(set(dispatch) == {'schema', 'intent_canonical', 'intent_sha256', 'artifact_canonical',
                'artifact_sha256', 'sql_request_canonical', 'sql_request_sha256'}, 'NATIVE_DISPATCH_ORIGINAL')
            values = {}
            for key, leaf in (('intent', 'request.json'), ('artifact', 'evidence.json'), ('sql_request', 'sql-request.json')):
                raw = dispatch[key + '_canonical'].encode('ascii')
                pin = dispatch[key + '_sha256']
                require(h.digest(raw) == pin and pin in originals and originals[pin].raw == raw,
                        'NATIVE_DISPATCH_ORIGINAL')
                originals[pin].revalidate()
                values[leaf] = raw
            original_intent = l.ops._json(values['request.json'], 65536)
            original_sql = l.ops._json(values['sql-request.json'], 1048576)
            require(original_intent['purpose'] == 'ENABLE' and original_sql['kind'] == 'TRANSITION'
                    and original_sql['args']['transition'] == 'ENABLE'
                    and original_sql['args']['request_key'] == original_intent['request_key']
                    and original_sql['account_id'] == original_intent['account_id'], 'NATIVE_DISPATCH_CORRELATION')
            root = self.phase.owner.root / 'native4-originals'
            if not os.path.lexists(root):
                root.mkdir(mode=0o700)
            with h.source_module().evidence().opened(root, directory=True, exact=(0, 0, 0o700)):
                destination = root / h.digest(descriptor.raw)[7:]
                destination.mkdir(mode=0o700)
                for leaf, raw in values.items():
                    h.fixture_module().write_new(destination / leaf, raw, 0o400)
                h.fixture_module().write_new(destination / 'dispatch.json', descriptor.raw, 0o400)
                fd = os.open(destination, l.journal.DIR_FLAGS)
                try:
                    os.fsync(fd)
                finally:
                    os.close(fd)
            descriptor.revalidate()
            self.revalidate()
        # A locator only. Later readback reopens all four actual originals and
        # independently correlates them; it does not trust this return value.
        return destination

    def source_case(self, scheduler, name):
        require(name in {CASES[0][0], CASES[1][0]}, 'NATIVE_TEST_SELECTION')
        before = database_image(self.phase)
        with self.source_inputs(scheduler, name) as (stage, mounts):
            output = self.execute(name, mounts)
            require(database_image(self.phase) == before, 'NATIVE_READ_MUTATED_DATABASE')
            self.revalidate()
        result = {'name': PREFIX + name, 'status': 'OBSERVED_TEST_OK_PENDING_CLOSE', 'elf_sha256': self.elf_sha,
                  'output_sha256': history().digest(output), 'database_changed': False,
                  'cleanup': 'PENDING', 'archive_acceptance': False}
        self.results.append(result)

    def readback_case(self, original_dir, *, committed):
        """Reopen actual retained dispatch; read only, never redispatch ENABLE."""
        import datetime
        require(type(committed) is bool, 'NATIVE_TEST_SELECTION')
        name, marker = CASES[2 if committed else 3]
        h = history()
        l = h.lifecycle_module()
        e = h.source_module().evidence()
        o = self.phase.owner
        self.revalidate()
        original_dir = Path(original_dir)
        require(original_dir.parent == o.root / 'native4-originals'
                and re.fullmatch('[0-9a-f]{64}', original_dir.name) is not None, 'NATIVE_DISPATCH_OWNER')
        parent = self.stack.enter_context(e.opened(original_dir, directory=True, exact=(0, 0, 0o700)))
        require(set(os.listdir(parent)) == {'request.json', 'evidence.json', 'sql-request.json', 'dispatch.json'},
                'NATIVE_DISPATCH_ORIGINAL')
        originals = {}
        for leaf in ('request.json', 'evidence.json', 'sql-request.json', 'dispatch.json'):
            original = l._Original(parent, leaf, maximum=1048576)
            self.stack.callback(original.close)
            originals[leaf] = original
        descriptor = originals['dispatch.json'].raw
        require(h.digest(descriptor)[7:] == original_dir.name, 'NATIVE_DISPATCH_HASH')
        dispatch = l.ops._json(descriptor, 1048576)
        require(type(dispatch) is dict and set(dispatch) == {'schema', 'intent_canonical', 'intent_sha256',
            'artifact_canonical', 'artifact_sha256', 'sql_request_canonical', 'sql_request_sha256'}
            and dispatch['schema'] == 'owner-beta-paper-native-dispatch-v1', 'NATIVE_DISPATCH_ORIGINAL')
        for key, leaf in (('intent', 'request.json'), ('artifact', 'evidence.json'), ('sql_request', 'sql-request.json')):
            require(dispatch[key + '_canonical'].encode('ascii') == originals[leaf].raw
                    and dispatch[key + '_sha256'] == h.digest(originals[leaf].raw), 'NATIVE_DISPATCH_ORIGINAL')
        request = l.ops._json(originals['sql-request.json'].raw, 1048576)
        require(request['kind'] == 'TRANSITION' and request['args']['transition'] == 'ENABLE',
                'NATIVE_ENABLE_ORIGINAL_REQUIRED')
        evidence = l.ops._json(request['args']['evidence_canonical'].encode('ascii'), 65536)
        if committed:
            expiry = datetime.datetime.fromisoformat(evidence['expires_at'].replace('Z', '+00:00'))
            require(expiry.tzinfo is not None, 'NATIVE_EXPIRY')
            # Real elapsed time under the same absolute phase. No clock change,
            # forged date or renewed deadline can make an original expire.
            while datetime.datetime.now(datetime.timezone.utc) <= expiry:
                time.sleep(self.phase.deadline.seconds(0.25))
        stage = self.stage()
        for directory in ('output', 'secrets', 'marker'):
            os.mkdir(directory, 0o700, dir_fd=stage.fd)
        for leaf in ('request.json', 'evidence.json', 'sql-request.json'):
            l.ops._write_file(stage.fd, leaf, originals[leaf].raw)
        l.ops._write_file(stage.fd, 'database.json', h.canonical(dict(schema='owner-beta-paper-ops-database-v1',
            host='postgres', port=5432, database='lagrange')))
        for folder, leaf, raw in (('marker', 'native-reset-case', marker),):
            fd = os.open(folder, l.journal.DIR_FLAGS, dir_fd=stage.fd)
            try:
                l.ops._write_file(fd, leaf, raw)
            finally:
                os.close(fd)
        secret_path = o.root / 'secrets/db-migrate'
        secret_parent = self.stack.enter_context(e.opened(secret_path, directory=True, owners=(0, 999), exact=(999, 999, 0o750)))
        secret_original = l._Original(secret_parent, 'db_migration_owner_password', uid=999, gid=999)
        self.stack.callback(secret_original.close)
        secret_fd = os.open('secrets', l.journal.DIR_FLAGS, dir_fd=stage.fd)
        try:
            l.ops._write_file(secret_fd, 'db_migration_owner_password', secret_original.raw)
            require(set(os.listdir(secret_fd)) == {'db_migration_owner_password'}, 'NATIVE_WORKER_SECRET_PRESENT')
        finally:
            os.close(secret_fd)
        mounts = [(stage.path, '/run/owner-beta-paper-ops', True),
            (stage.path + '/output', '/run/owner-beta-paper-ops/output', False),
            (stage.path + '/secrets', '/run/secrets', True),
            (stage.path + '/marker', '/run/r5-paper-checker-integration', True)]
        if committed:
            current_dir = o.root / 'data/owner-beta-paper-evidence/current'
            current_parent = self.stack.enter_context(e.opened(current_dir, directory=True, exact=(0, 10001, 0o750)))
            current = l._Original(current_parent, e.CURRENT_LEAF, mode=0o440, gid=10001)
            self.stack.callback(current.close)
            value = e.decode(current.raw)
            e.validate_current(value)
            require(value['status'] == 'INVALID', 'NATIVE_SOURCE_STILL_CURRENT')
            daily_dir = o.release / '.owner-beta-daily'
            daily_parent = self.stack.enter_context(e.opened(daily_dir, directory=True, exact=(0, 10001, 0o750)))
            for leaf in ('config.json', 'policy.json'):
                original = l._Original(daily_parent, leaf, mode=0o440, gid=10001)
                self.stack.callback(original.close)
                mounts.append((str(daily_dir / leaf), '/run/owner-beta-daily/' + leaf, True))
            mounts.append((str(current_dir), '/data/owner-beta-paper-evidence/current', True))
        stage.snapshot(mutable_children=('output',))
        before = database_image(self.phase)
        output = self.execute(name, [self.runtime.mount(*item) for item in mounts])
        require(database_image(self.phase) == before, 'NATIVE_READ_MUTATED_DATABASE')
        for item in originals.values():
            item.revalidate()
        self.revalidate()
        self.results.append({'name': PREFIX + name, 'status': 'OBSERVED_TEST_OK_PENDING_CLOSE',
            'elf_sha256': self.elf_sha, 'output_sha256': h.digest(output), 'database_changed': False,
            'cleanup': 'PENDING', 'archive_acceptance': False,
            'receipt_case': 'COMMITTED' if committed else 'ABSENT_ONLY'})

    def report(self):
        require(self.closed and self.phase.closed and not self.phase.owner.original_custody_uncertain,
                'NATIVE_REPORT_BEFORE_CLEANUP')
        return [dict(item, status='PASS_TEST_ORIGIN', cleanup='COMPLETE') for item in self.results]

    def __exit__(self, exc_type, exc, tb):
        try:
            if self.runtime is not None:
                self.runtime.close()
            self.revalidate(cleanup=True)
            self.phase.quiescent(cleanup=True)
            require(not self.command_custodies, 'NATIVE_CHILD_CUSTODY_UNKNOWN')
            for item in reversed(self.inputs):
                item.close()
            self.stack.close()
            require(all(item.closed for item in self.inputs) and all(c.closed for c in self.controllers),
                    'NATIVE_INPUT_BORROWERS_OPEN')
            require(all(stage.closed for stage in self.stages), 'NATIVE_STAGE_CLOSE_UNKNOWN')
        except BaseException:
            _UNRESOLVED_CALLERS.add(self)
            self.phase.owner.original_custody_uncertain = True
            raise
        self.closed = True
        _CALLERS.discard(self)


def later_off(phase):
    """Actual BEGIN/RECONCILE/OFF on this isolated libtest-origin branch.

    Source must already have revoked Current and returned its actual lock. This
    is SQL implementation evidence, not an official DrainedHandle or archive
    operation. The native-origin root remains excluded from C acceptance.
    """
    import fcntl
    import uuid
    h = history()
    f = h.fixture_module()
    e = h.source_module().evidence()
    o = phase.owner
    phase.revalidate()
    require(phase in _PHASES and not any(value is phase for value in _SCHEDULERS.values())
            and not any(caller.phase is phase for caller in _CALLERS)
            and not o.helpers and not o.helper_bindings and not o.tracees and not o.threads and not o.blockers,
            'NATIVE_WRITERS_UNSETTLED')
    phase.quiescent()
    root = o.root / 'data/owner-beta-paper-evidence'
    with e.opened(root / '.control/lock', write=True, exact=(0, 0, 0o600)) as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        with e.opened(root / 'current' / e.CURRENT_LEAF, exact=(0, 10001, 0o440)) as current:
            current_raw = os.pread(current, e.SOURCE_LIMIT + 1, 0)
            value = e.decode(current_raw)
            e.validate_current(value)
            require(value['status'] == 'INVALID', 'NATIVE_SOURCE_STILL_CURRENT')
            control = o.observer.json('SELECT to_jsonb(c) FROM public.owner_beta_paper_control c;', role='migration_owner')
            require(type(control) is dict and control['state'] == 'ENABLED'
                    and control['owner_user_id'] == phase.facts['owner_user_id']
                    and control['strategy_config_id'] == phase.facts['strategy_config_id'], 'NATIVE_ENABLE_REQUIRED')
            account = str(uuid.UUID(control['account_id']))
            epoch = control['control_epoch']
            require(type(epoch) is int and 0 < epoch < 2**63 - 2, 'NATIVE_CONTROL_EPOCH')
            with f.ops.journal.open_journal(str(o.root), o.lock_fd) as journal:
                installation = journal.installation_id
                journal.revalidate()
            generation = str(uuid.uuid4())
            intent = dict(schema='owner-beta-paper-operation-intent-v1', installation_id=installation,
                generation=generation, account_id=account, predecessor_generation=None,
                request_key=generation + ':begin', purpose='STOP', scope='PAPER_STOP',
                expected_epoch=epoch, local_epoch_floor=epoch, release_manifest_sha256=h.digest(o.manifest),
                artifact_sha256=None, resume_target_id=None)
            # Actual observed identities/rows form the private inventory. There
            # is no caller assertion of stopped state, fake PID or clock.
            clients = o.observer.json("SELECT coalesce(jsonb_agg(jsonb_build_object('pid',pid,'start',backend_start)),"
                "'[]'::jsonb) FROM pg_stat_activity WHERE datname=current_database() "
                "AND backend_type='client backend' AND pid<>pg_backend_pid();")
            require(clients == [], 'NATIVE_BACKENDS_UNSETTLED')
            network = o.docker.object('network', o.network)
            require(set((network.get('Containers') or {}).keys()) == {o.pg}, 'NATIVE_CONTAINERS_UNSETTLED')
            inventory = h.canonical(dict(schema='private-native4-stop-inventory-v1',
                root_identity=list(o.root_id), postgres=o.pg, database_generation=list(o.observer.generation),
                network=o.network, container_ids=sorted(network['Containers']), clients=clients,
                current_sha256=h.digest(current_raw), boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip(),
                observed=e.sample()))
            stage_bytes = h.canonical(dict(schema='private-native4-stop-stage-v1', generation=generation,
                intent_sha256=h.digest(h.canonical(intent)), inventory_sha256=h.digest(inventory)))
            def json_sql(raw):
                return "convert_from(decode('" + raw.hex() + "','hex'),'UTF8')::jsonb"
            try:
                begin = o.observer.json('SELECT public.begin_owner_beta_paper_operation_v1(' + str(epoch) + ','
                    + json_sql(h.canonical(intent)) + ",'" + generation + ":begin');", role='migration_owner')
                require(begin['outcome'] == 'OK' and begin['control_epoch'] == epoch + 1,
                        'NATIVE_STOP_BEGIN_UNPROVEN')
                fence = begin['control_epoch']
                phase.quiescent()
                reconciled = o.observer.json("SELECT public.reconcile_owner_beta_paper_operation_v1('" + generation
                    + "'::uuid," + str(fence) + ",'" + h.digest(inventory) + "','" + generation + ":reconcile:1');",
                    role='migration_owner')
                require(reconciled['outcome'] == 'OK' and reconciled['data']['summary']['unresolved_commits'] == 0
                        and reconciled['data']['summary']['live_claims'] == 0, 'NATIVE_STOP_RECONCILE_UNPROVEN')
                phase.quiescent()
                require(os.pread(current, e.SOURCE_LIMIT + 1, 0) == current_raw, 'NATIVE_SOURCE_CHANGED')
                receipt = dict(schema='owner-beta-paper-drain-receipt-v1', installation_id=installation,
                    generation=generation, account_id=account, request_key=generation + ':off', scope='PAPER_STOP',
                    expected_epoch=epoch, resulting_epoch=fence, intent_sha256=begin['data']['intent_sha256'],
                    stage_sha256=h.digest(stage_bytes), inventory_sha256=h.digest(inventory),
                    reconciliation_sha256=reconciled['data']['summary_sha256'],
                    observation_id=reconciled['data']['observation_id'], release_manifest_sha256=h.digest(o.manifest),
                    boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip(), observed=e.sample())
                off = o.observer.json('SELECT public.transition_owner_beta_paper_v1(' + str(fence) + ",'OFF',"
                    + json_sql(h.canonical(receipt)) + ",'" + generation + ":off');", role='migration_owner')
                require(off['outcome'] == 'OK' and off['control_epoch'] == fence + 1 and off['data']['state'] == 'OFF',
                        'NATIVE_STOP_OFF_UNPROVEN')
                config_path = o.release / '.owner-beta-daily/config.json'
                config_raw = e.read_file(config_path, 16384, exact=(0, 10001, 0o440))
                policy_raw = e.read_file(o.release / '.owner-beta-daily/policy.json', 65536, exact=(0, 10001, 0o440))
                config = f.ops.daily.install.validate_config(config_raw, o.commit)
                template = (o.release / 'deploy/db/owner-beta-daily-control.sql').read_text()
                for old, new in {'__ACTION__': 'disable', '__CONFIG_HEX__': config_raw.hex(),
                        '__POLICY_HEX__': policy_raw.hex(), '__COMMIT__': o.commit,
                        '__CAPACITY__': 'NULL', '__LEASE__': 'NULL', '__BACKOFF__': 'NULL'}.items():
                    template = template.replace(old, new)
                require(o.observer.sql(template, role='migration_owner') == b'DAILY_CONTROL: DISABLED',
                        'NATIVE_DAILY_OFF_UNPROVEN')
                disabled = o.observer.json('SELECT to_jsonb(c) FROM public.owner_beta_daily_control c;', role='migration_owner')
                require(disabled['enabled'] is False and disabled['owner_user_id'] == config['owner'],
                        'NATIVE_DAILY_OFF_UNPROVEN')
                # The immutable normal config remains unchanged. Activation
                # belongs to the actual DB control row, never a config field.
                f.write_new(o.root / 'native4-stop-originals.json', h.canonical(dict(
                    schema='private-native4-stop-originals-v1', intent=intent, inventory=e.decode(inventory),
                    stage=e.decode(stage_bytes), begin=begin, reconciliation=reconciled, receipt=receipt, off=off,
                    fixture_parent_origin='PRIVATE_IMPLEMENTATION', sql_origin='LIBTEST_IMPLEMENTATION',
                    archive_acceptance=False)), 0o400)
                phase.quiescent()
                require(os.pread(current, e.SOURCE_LIMIT + 1, 0) == current_raw, 'NATIVE_SOURCE_CHANGED')
            except BaseException:
                # An acknowledgement/cleanup failure cannot be retried as a new
                # operation or silently advertised as a known OFF result.
                o.original_custody_uncertain = True
                raise
    return {'status': 'OFF_TEST_ORIGIN', 'archive_acceptance': False}


SERVICE_STORE = '.native-source'
_PRIVATE_INSTALLERS = set()


def action_supply_config(commit, config, record):
    """Normal closed R2 wire built from the retained approved V3 material."""
    import datetime
    import uuid
    require(config['code_commit'] == commit and str(uuid.UUID(config['owner'])) == config['owner']
        and datetime.date.fromisoformat(config['first_auto_session'])
            > datetime.date.fromisoformat(record['range_end']), 'NATIVE_ACTION_CONFIG')
    value = dict(schema='r2-action-supply-v1', enabled=True, release_commit=commit,
        owner=config['owner'], policy_sha256=config['policy_sha256'],
        activation_date=config['first_auto_session'], base_candidate_sha256=record['candidate_content_sha256'],
        initial_civil_frontier=record['range_end'],
        entitlement_reference_sha256=history().digest(config['entitlement_reference'].encode('utf-8')),
        raw_owner_uid=config['raw_owner_uid'], timeout_seconds=300)
    wire = history().canonical(value)
    require(len(wire) <= 4096, 'NATIVE_ACTION_CONFIG')
    return wire


def source_environment_bytes(owner, config, coordination_root, generation, action_config):
    """Closed nonsecret environment derived before immutable publication."""
    require(re.fullmatch('[A-Za-z0-9._-]{1,128}', generation) is not None, 'NATIVE_CREDENTIAL_GENERATION')
    values = dict(LAGRANGE_CODE_COMMIT=owner.commit, COMPOSE_PROJECT_NAME=owner.project,
        LAGRANGE_DATA_DIR=str(owner.root / 'data'), LAGRANGE_ARTIFACTS_DIR=str(owner.root / 'artifacts'),
        LAGRANGE_RUNTIME_SECRET_DIR=str(owner.root / 'secrets'),
        LAGRANGE_RUNTIME_STATE_DIR=str(coordination_root), POSTGRES_DB='lagrange',
        OWNER_BETA_DAILY_MODE='owner_only', OWNER_BETA_PAPER_MODE='disabled',
        OWNER_BETA_ACCESS_MODE='owner_only', OWNER_BETA_PRICE_INPUT_MODE='sealed_v1',
        MAX_JOBS_PER_OWNER='10', PRICE_BETA_LEASE_SECS='60', PRICE_BETA_BACKOFF_SECS='30',
        RESEARCH_APP_ENV='production', RESEARCH_FETCH_MODE='credentialed',
        RESEARCH_ENTITLEMENT_REFERENCE=config['entitlement_reference'],
        RESEARCH_ENTITLEMENT_SHA256=config['entitlement_sha256'],
        RESEARCH_R2_ACTION_SUPPLY_CONFIG_HEX=action_config.hex(),
        BACKTEST_MIN_FREE_BYTES='0', BACKTEST_MAX_QUEUED_BACKTESTS='0',
        BACKTEST_RECONCILE_GRACE_SECS='0', BACKTEST_RECONCILE_INTERVAL_SECS='0',
        RESEARCH_CANDIDATE_ENABLED='false', OWNER_INTRADAY_QUOTES_MODE='off',
        KIS_READ_COORDINATION_MODE='shared_required', KIS_READ_CREDENTIAL_GENERATION=generation,
        OWNER_INTRADAY_SESSION_WINDOWS_SOURCE='release_v1', OWNER_INTRADAY_QUOTE_TRANSPORT='market_ws')
    require(all(isinstance(v, str) and re.fullmatch('[A-Za-z0-9/._:@+-]+', v) for v in values.values()),
            'NATIVE_ENV_VALUE')
    return ''.join(k + '=' + v + '\n' for k, v in sorted(values.items())).encode('ascii')


def prepare_acquisition(preparation, stage, inputs):
    """Borrow real credentials/calendar/shared rate state; never reset that state.

    Inputs are protected originals, not readiness assertions. Only their owned
    copies and deterministic environment are placed in the unpublished release.
    The existing shared coordination directory is retained, never cloned.
    """
    import datetime
    h = history()
    f = h.fixture_module()
    e = h.source_module().evidence()
    o = preparation.owner
    require(preparation in _OWNERS and preparation.used and not preparation.published
            and o.context is None and not hasattr(o, 'release'), 'NATIVE_PREPUBLICATION_ORDER')
    inputs = Path(inputs)
    preparation.stack.enter_context(e.opened(inputs, directory=True, exact=(0, 0, 0o700)))
    def read(path, maximum=65536, exact=(0, 0, 0o400)):
        fd = preparation.stack.enter_context(e.opened(path, exact=exact))
        before = os.fstat(fd)
        raw = os.pread(fd, maximum + 1, 0)
        require(0 < len(raw) <= maximum and len(raw) == before.st_size, 'NATIVE_ACQUISITION_INPUT')
        preparation.acquisition_originals.append((Path(path), fd, before, raw))
        return raw
    preparation.acquisition_originals = []
    generation = read(inputs / 'credential-generation', 128).decode('ascii').strip()
    # This root owns the actual shared token/rate locks. No new generation,
    # token cache, clock, or private copy can hide another credential user.
    state = inputs / 'state'
    preparation.stack.enter_context(e.opened(state, directory=True, exact=(0, 10001, 0o750)))
    preparation.stack.enter_context(e.opened(state / 'kis-read-coordination', directory=True,
                                            exact=(10001, 10001, 0o700), owners=(0, 10001)))
    calendar = o.root / SERVICE_STORE / 'calendar'
    calendar.mkdir(mode=0o700)
    for leaf in ('calendar.json', 'manifest.json', 'overrides.json'):
        f.write_new(calendar / leaf, read(inputs / 'calendar' / leaf, 16 * 1024 * 1024), 0o400)
    cal = h.load('native_source_calendar', 'scripts/ops/xkrx-calendar-bootstrap.py')
    value = e.decode((calendar / 'calendar.json').read_bytes(), 16 * 1024 * 1024, False)
    cal.load_existing(calendar, datetime.date.fromisoformat(value['range']['start']),
                      datetime.date.fromisoformat(value['range']['end']))
    for leaf in ('kis_app_key', 'kis_app_secret'):
        raw = read(inputs / leaf, 4096)
        require(b'\x00' not in raw and b'\n' not in raw.rstrip(b'\n'), 'NATIVE_CREDENTIAL_ORIGINAL')
        for service in ('research-worker', 'research-range-raw'):
            parent = o.root / 'secrets' / service
            if not parent.exists():
                parent.mkdir(mode=0o750)
                os.chown(parent, 10001, 10001)
                os.chmod(parent, 0o750)
            f.write_new(parent / leaf, raw, 0o400, 10001, 10001)
    action = action_supply_config(o.commit, preparation.runtime.config, preparation.material.record)
    env = source_environment_bytes(o, preparation.runtime.config, state, generation, action)
    # This is the actual owned unpublished env inode, not installed release surgery.
    path = stage / 'deploy/compose/.env'
    with e.opened(path, write=True, exact=(0, 0, 0o600)) as fd:
        os.ftruncate(fd, 0)
        require(os.write(fd, env) == len(env), 'NATIVE_ENV_WRITE')
        os.fsync(fd)
    preparation.acquisition = dict(input_root=str(inputs), coordination_root=str(state),
        coordination_identity=list(h.source_module().inode(os.stat(state))),
        calendar_sha256=h.digest((calendar / 'calendar.json').read_bytes()),
        environment_sha256=h.digest(env))


def revalidate_acquisition(preparation):
    for path, fd, before, raw in getattr(preparation, 'acquisition_originals', ()):
        key = lambda s: (s.st_dev, s.st_ino, s.st_size, s.st_mtime_ns, s.st_ctime_ns,
                         s.st_uid, s.st_gid, s.st_mode, s.st_nlink)
        require(key(before) == key(os.fstat(fd)) == key(os.stat(path, follow_symlinks=False))
                and os.pread(fd, len(raw) + 1, 0) == raw, 'NATIVE_ACQUISITION_CHANGED')


def activate_daily(owner):
    """Use the existing normal config/control SQL builder and actual DB result."""
    h = history()
    daily = h.fixture_module().ops.daily
    values, overlays = daily.read_context(str(owner.release), owner.commit, 'enable')
    daily.reject_ambient(values)
    sql = daily.install.render_control(str(owner.release), owner.commit, 'enable',
        values['MAX_JOBS_PER_OWNER'], values['PRICE_BETA_LEASE_SECS'], values['PRICE_BETA_BACKOFF_SECS'])
    require(owner.observer.sql(sql, role='migration_owner').strip() == b'DAILY_CONTROL: ENABLED',
            'NATIVE_DAILY_ACTIVATION')
    require(overlays == ['-f', str(owner.release / 'deploy/compose/compose.intraday.yml')],
            'NATIVE_SOURCE_OVERLAYS')


def create_binding(owner, config, beta_strategy):
    """Untrusted binding built only from actual returned config/control rows."""
    import uuid
    h = history()
    e = h.source_module().evidence()
    root = owner.root / SERVICE_STORE
    actor = str(uuid.UUID(config['owner']))
    identifier = str(uuid.UUID(beta_strategy))
    row = owner.observer.json("BEGIN READ ONLY; DO $$ BEGIN PERFORM set_config('app.actor_user_id','" + actor + "',true); END $$;"
        "SELECT jsonb_build_object('control',to_jsonb(d),'strategy',to_jsonb(s)) "
        "FROM public.owner_beta_daily_control d JOIN public.user_strategy_configs s ON s.owner_user_id=d.owner_user_id "
        "WHERE s.id='" + identifier + "'::uuid; ROLLBACK;", role='migration_owner')
    d, s = row['control'], row['strategy']
    require(d['enabled'] is True and d['owner_user_id'] == actor and d['policy_sha256'] == config['policy_sha256']
        and s['id'] == identifier and s['strategy_id'] == 'trend_following' and s['strategy_version'] == '1.0.0'
        and s['is_active'] is True and s['daily_enabled'] is True
        and s['config_json'] == {'benchmark_instrument': '069500.KRX', 'fast_ma': 50, 'slow_ma': 200},
        'NATIVE_BINDING_DATABASE')
    images = h.image_manifest(owner.manifest, owner.commit)
    # Same public canonical snapshot material as OwnerBetaStrategySnapshot.
    strategy_hash = h.digest(h.canonical(dict(schema='owner-beta-strategy-config-v1',
        strategy_id=s['strategy_id'], strategy_version=s['strategy_version'], config_json=s['config_json'])))
    cohort = dict(owner_user_id=actor, policy_sha256=d['policy_sha256'], activation_epoch=d['activation_epoch'],
        strategy_config_id=identifier, config_epoch=s['daily_epoch'], strategy_config_sha256=strategy_hash,
        entitlement_reference=d['entitlement_reference'], entitlement_sha256=d['entitlement_sha256'],
        release_manifest_sha256=h.digest(owner.manifest), research_image_id=images['research-worker'],
        helper_image_id=images['owner-beta-runner'], verifier_revision=owner.commit,
        calendar_sha256=h.digest((root / 'calendar/calendar.json').read_bytes()))
    network = owner.docker.object('network', owner.network)
    binding = dict(schema='owner-beta-paper-witness-binding-v1', cohort=cohort,
        evidence_root=str(owner.root / 'data/owner-beta-paper-evidence'), release_root=str(owner.release),
        production_env_file=str(owner.release / 'deploy/compose/.env'),
        production_env_sha256=h.digest((owner.release / 'deploy/compose/.env').read_bytes()),
        calendar_dir=str(root / 'calendar'), calendar_file=str(root / 'calendar/calendar.json'),
        release_manifest_file=str(owner.release / '.lagrange-release-manifest'), data_root=str(owner.root / 'data'),
        daily_config_file=str(owner.release / '.owner-beta-daily/config.json'),
        daily_config_sha256=h.digest((owner.release / '.owner-beta-daily/config.json').read_bytes()),
        secret_root=str(owner.root / 'secrets'), database='lagrange', project=owner.project, network=network['Name'],
        raw_owner_uid=config['raw_owner_uid'], max_source_seconds=3600,
        state_dir=str(root / 'collector'), lock_file=str(root / 'collector.lock'))
    # Recursive fields are excluded by the existing semantic-hash constructor.
    cohort.update(semantic_config_sha256='', cohort_sha256='')
    cohort['semantic_config_sha256'] = e.semantic_hash(binding)
    cohort['cohort_sha256'] = h.digest(h.canonical({k: v for k, v in cohort.items() if k != 'cohort_sha256'}))
    e.binding_structure(binding)
    h.fixture_module().write_new(root / 'binding.json', e.canonical(binding), 0o600)
    actual, raw, _ = e._binding_with_helper_projection(str(root / 'binding.json'))
    require(actual == binding and raw == e.canonical(binding), 'NATIVE_BINDING_CHANGED')
    return binding


POLICY_TARGET = history().fixture_module().SERVICE_STATUS_TARGET


def native_drop_body(owner, policy):
    require(Path(policy) == owner.root / SERVICE_STORE / 'policy', 'NATIVE_POLICY_OWNER')
    require(re.fullmatch('/[A-Za-z0-9/._-]+', str(policy)) is not None, 'NATIVE_POLICY_OWNER')
    return history().fixture_module().service_status_drop_body(policy)


class NativePolicy(history().fixture_module().ServiceStatusScope):
    """Native caller of the shared fixed-target private policy scope."""
    def _check(self, value, operation):
        require(value, 'NATIVE_POLICY_' + operation)


def native_policy_bytes(owner, policy):
    """Validate the distinct owned native policy without adopting C custody."""
    h = history()
    e = h.source_module().evidence()
    require(Path(policy) == owner.root / SERVICE_STORE / 'policy', 'NATIVE_POLICY_OWNER')
    raw = e.read_file(Path(policy) / 'service-status/policy.json', 65536, exact=(0, 0, 0o600))
    value = e.decode(raw, canonical_required=False)
    h.fixture_module().ops.daily.policy.validate_policy(value)
    require(value['release_commit'] == owner.commit and value['compose_project'] == owner.project,
            'NATIVE_POLICY_OWNER')
    return raw


def private_service_body(owner, binding, binding_path, state_dir, lock_file):
    e = history().source_module().evidence()
    normal = e.render_service(binding, str(binding_path), str(state_dir), str(lock_file))
    old = 'ExecStart=/usr/bin/python3 ' + str(owner.release / 'scripts/ops/lib/owner-beta-paper-evidence.py') + ' run --binding ' + str(binding_path)
    new = 'ExecStart=/usr/bin/python3 ' + str(owner.release / 'scripts/ops/verify-owner-beta-paper-native-source.py') + ' service-run --fixture-root ' + str(owner.root)
    require(normal.count(old + '\n') == 1, 'NATIVE_SERVICE_TEMPLATE')
    # The private owner needs its private runtime dirs; production rendering is unchanged.
    return normal.replace(old + '\n', new + '\n').replace('TimeoutStopSec=12h', 'TimeoutStopSec=120s').replace(
        'RestrictAddressFamilies=AF_UNIX', 'RestrictAddressFamilies=AF_UNIX AF_INET').replace(
        '[Install]\n', 'ReadWritePaths=' + str(owner.root) + '\n[Install]\n')


def aggregate_cases(rows):
    expected = {PREFIX + name for name, _ in CASES}
    require(type(rows) is list and len(rows) == 4 and {r.get('name') for r in rows} == expected,
            'NATIVE_FOUR_REQUIRED')
    require(all(r.get('status') == 'PASS_TEST_ORIGIN' and r.get('cleanup') == 'COMPLETE'
        and r.get('archive_acceptance') is False and r.get('database_changed') is False for r in rows),
        'NATIVE_CASE_UNPROVEN')
    elfs = {row.get('elf_sha256') for row in rows}
    require(len(elfs) == 1 and all(isinstance(pin, str) and re.fullmatch('sha256:[0-9a-f]{64}', pin)
        for pin in elfs), 'NATIVE_TEST_ELF_CHANGED')
    require(all(isinstance(row.get('output_sha256'), str)
        and re.fullmatch('sha256:[0-9a-f]{64}', row['output_sha256']) for row in rows), 'NATIVE_TEST_OUTPUT')
    return dict(status='PASS_TEST_ORIGIN', count=4, cases=sorted(rows, key=lambda r: r['name']),
        receipt_coverage='COMMITTED_AND_ABSENT_ONLY', fixture_parent_origin='PRIVATE_IMPLEMENTATION',
        sql_origin='LIBTEST_IMPLEMENTATION', archive_acceptance=False)


class PrivateService:
    """Constructor-owned service originals for this native fixture only.

    The canonical unit name is deliberately shared with the normal installer:
    an existing foreign unit is a collision, never renamed or adopted. The
    default installer and ordinary genuine-source acceptance path are unchanged.
    """
    def __init__(self, phase):
        require(phase in _PHASES and not phase.closed, 'NATIVE_PHASE_CLOSED')
        self.phase, self.owner = phase, phase.owner
        self.g = history().source_module()
        self.e = self.g.evidence()
        self.root = self.owner.root / SERVICE_STORE
        self.binding_path = self.root / 'binding.json'
        self.policy = self.root / 'policy'
        self.unit = self.g.SYSTEMD / self.g.unit_name(self.owner)
        self.drop = Path(str(self.unit) + '.d')
        self.ledger = self.root / 'service.json'
        self.original_stack = contextlib.ExitStack()
        self.held_originals = {}
        phase.stack.callback(self.original_stack.close)
        self.binding, _raw, _ = self.e._binding_with_helper_projection(str(self.binding_path))
        self.check_binding()

    def check_binding(self):
        b, o = self.binding, self.owner
        actual, _raw, _ = self.e._binding_with_helper_projection(str(self.binding_path))
        require(actual == b and b['release_root'] == str(o.release) and b['project'] == o.project
            and b['data_root'] == str(o.root / 'data') and b['secret_root'] == str(o.root / 'secrets')
            and b['cohort']['owner_user_id'] == self.phase.facts['owner_user_id']
            and b['cohort']['strategy_config_id'] == self.phase.facts['strategy_config_id'], 'NATIVE_SERVICE_OWNER')
        fixed = dict(calendar_dir=self.root / 'calendar', calendar_file=self.root / 'calendar/calendar.json',
            state_dir=self.root / 'collector', lock_file=self.root / 'collector.lock',
            production_env_file=o.release / 'deploy/compose/.env',
            release_manifest_file=o.release / '.lagrange-release-manifest',
            daily_config_file=o.release / '.owner-beta-daily/config.json',
            evidence_root=o.root / 'data/owner-beta-paper-evidence')
        require(all(b[key] == str(path) for key, path in fixed.items()), 'NATIVE_SERVICE_PATHS')
        network = o.docker.object('network', o.network)
        require(network['Id'] == o.network and network['Name'] == b['network'] and network['Internal'] is True
            and network['Labels']['com.docker.compose.project'] == o.project
            and network['Labels']['com.docker.compose.network'] == 'owner-beta-db', 'NATIVE_SOURCE_NETWORK')
        native_policy_bytes(o, self.policy)

    def argv(self):
        return ['/usr/bin/python3', str(self.owner.release / 'scripts/ops/verify-owner-beta-paper-native-source.py'),
                'service-run', '--fixture-root', str(self.owner.root)]

    def originals(self):
        import hashlib
        records = {}
        paths = [(self.unit, 0o644), (self.drop / 'private-policy.conf', 0o644),
            (self.binding_path, 0o600), (self.policy / 'service-status/policy.json', 0o600),
            (self.root / 'inputs.json', 0o400), (self.root / 'native-test', 0o500)]
        paths += [(self.owner.release / relative, 0o644) for relative in (
            'scripts/ops/verify-owner-beta-paper-native-source.py',
            'scripts/ops/lib/owner-beta-paper-native-source.py',
            'scripts/ops/lib/owner-beta-paper-evidence.py')]
        for path, mode in paths:
            if path not in self.held_originals:
                held = self.original_stack.enter_context(self.e.opened(path, exact=(0, 0, mode)))
                before = os.fstat(held)
                maximum = 512 * 1024 * 1024 if path == self.root / 'native-test' else 2 * 1024 * 1024
                require(0 < before.st_size <= maximum, 'NATIVE_SERVICE_ORIGINAL')
                hasher = hashlib.sha256()
                offset = 0
                while chunk := os.pread(held, 1024 * 1024, offset):
                    self.phase.deadline.seconds(cleanup=True)
                    hasher.update(chunk)
                    offset += len(chunk)
                require(offset == before.st_size, 'NATIVE_SERVICE_ORIGINAL')
                record = dict(identity=self.g.inode(before), size=before.st_size,
                    mtime=before.st_mtime_ns, ctime=before.st_ctime_ns,
                    sha256='sha256:' + hasher.hexdigest())
                self.held_originals[path] = (held, file_identity(before), record)
            held, pin, record = self.held_originals[path]
            require(file_identity(os.fstat(held)) == pin == file_identity(os.stat(path, follow_symlinks=False)),
                    'NATIVE_SERVICE_ORIGINAL_CHANGED')
            records[str(path)] = record
        return records

    def save(self, value):
        import uuid
        raw = self.e.canonical(value)
        require(len(raw) <= 65536, 'NATIVE_SERVICE_RECEIPT')
        temporary = self.root / ('.service-' + uuid.uuid4().hex)
        self.g.write_new(temporary, raw, 0o600)
        os.replace(temporary, self.ledger)
        with self.e.opened(self.root, directory=True, exact=(0, 0, 0o700)) as fd:
            os.fsync(fd)

    def load(self):
        raw = self.e.read_file(self.ledger, 65536, exact=(0, 0, 0o600))
        value = self.e.decode(raw)
        require(set(value) == {'schema', 'root', 'commit', 'assets', 'backend', 'attempts', 'state'}
            and value['schema'] == 'private-native4-service-v1' and value['root'] == list(self.owner.root_id)
            and value['commit'] == self.owner.commit and value['state'] in
            ('INSTALLED', 'STARTING', 'RUNNING', 'STOPPED', 'FAILED', 'RETIRED'), 'NATIVE_SERVICE_RECEIPT')
        require(value['state'] != 'RETIRED' and self.originals() == value['assets'], 'NATIVE_SERVICE_CHANGED')
        self.check_binding()
        expected = private_service_body(self.owner, self.binding, self.binding_path,
            self.binding['state_dir'], self.binding['lock_file']).encode()
        require(self.e.read_file(self.unit, 65536, exact=(0, 0, 0o644)) == expected
            and self.e.read_file(self.drop / 'private-policy.conf', 65536, exact=(0, 0, 0o644))
                == native_drop_body(self.owner, self.policy), 'NATIVE_SERVICE_CHANGED')
        network = self.owner.docker.object('network', value['backend'])
        require(network['Id'] == value['backend'] and network['Name'] == self.owner.project + '_backend'
            and network['Internal'] is False and network['Labels']['com.docker.compose.project'] == self.owner.project
            and network['Labels']['com.docker.compose.network'] == 'backend', 'NATIVE_BACKEND_CHANGED')
        return value

    def state(self, *, absent=False):
        props = self.g.PROPERTIES + ('ExecMainCode', 'ExecMainStatus', 'Result')
        text = self.g.ctl(self.owner, 'show', '--all', '--property=' + ','.join(props), self.g.unit_name(self.owner))
        result = {}
        for line in text.splitlines():
            key, sep, value = line.partition('=')
            require(sep and key in props and key not in result, 'NATIVE_UNIT_UNKNOWN')
            result[key] = value
        require(set(result) == set(props) and result['Id'] == self.g.unit_name(self.owner)
            and not result['TriggeredBy'] and not result['Triggers'], 'NATIVE_UNIT_UNKNOWN')
        if absent and result['LoadState'] == 'not-found':
            require(result['MainPID'] == '0' and result['ActiveState'] == 'inactive'
                and not result['FragmentPath'] and not result['DropInPaths'], 'NATIVE_UNIT_UNKNOWN')
            return result
        argv = re.search(r'(?:^|; )argv\[\]=([^;]+) ;', result['ExecStart'])
        require(result['LoadState'] == 'loaded' and result['FragmentPath'] == str(self.unit)
            and result['DropInPaths'] == str(self.drop / 'private-policy.conf')
            and result['UnitFileState'] == 'disabled' and result['NeedDaemonReload'] == 'no'
            and result['ExecStart'].startswith('{ path=/usr/bin/python3 ; ') and argv is not None
            and argv[1] == ' '.join(self.argv()) and result['ExecStart'].count('argv[]=') == 1,
            'NATIVE_UNIT_CHANGED')
        return result

    def process(self, state):
        pid = int(state['MainPID'])
        require(pid > 1 and state['ActiveState'] == 'active' and state['SubState'] == 'running'
            and state['ControlGroup'] == '/system.slice/' + self.g.unit_name(self.owner)
            and re.fullmatch('[0-9a-f]{32}', state['InvocationID']) is not None
            and int(state['ExecMainStartTimestampMonotonic']) > 0, 'NATIVE_PROCESS_UNKNOWN')
        observed = self.e.process(pid)
        require(self.g.inode(os.stat(f'/proc/{pid}/exe')) == self.g.inode(os.stat('/usr/bin/python3'))
            and Path(f'/proc/{pid}/cmdline').read_bytes().split(b'\0')
                == [x.encode() for x in self.argv()] + [b'']
            and Path(f'/proc/{pid}/cgroup').read_text() == '0::' + state['ControlGroup'] + '\n',
            'NATIVE_PROCESS_CHANGED')
        f = history().fixture_module()
        try:
            f.service_status_namespace(self.policy, pid)
        except (f.FixtureError, OSError):
            raise NativeFixtureError('NATIVE_POLICY_NAMESPACE') from None
        require(self.e.process(pid) == observed, 'NATIVE_PROCESS_CHANGED')
        return dict(identity=observed, boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip(),
            invocation_id=state['InvocationID'], started_monotonic_us=int(state['ExecMainStartTimestampMonotonic']),
            cgroup=state['ControlGroup'])

    def observe_start(self, attempt, state):
        process = self.process(state)
        require(process['boot_id'] == attempt['boot_id']
            and process['started_monotonic_us'] * 1000 >= attempt['observed']['monotonic_ns'],
            'NATIVE_PROCESS_GENERATION')
        require(attempt['process'] is None or attempt['process'] == process, 'NATIVE_PROCESS_CHANGED')
        attempt['process'] = process
        # The existing observation accepts INVALID/STARTING/null binding only
        # for this actual publisher/boot and never upgrades it to readiness.
        arm = self.g.arm_observation(self.owner, self.binding, process)
        if arm is not None:
            require(attempt['arm_sha256'] is None or attempt['arm_sha256'] == arm, 'NATIVE_ARM_CHANGED')
            attempt['arm_sha256'] = arm
        return arm

    def install(self, elf_path):
        import types
        self.phase.quiescent()
        self.g.host_units(self.owner)
        self.g.provider_idle(self.owner)
        self.g.absent(self.unit)
        self.g.absent(self.drop)
        self.g.absent(self.ledger)
        require(self.state(absent=True)['LoadState'] == 'not-found', 'NATIVE_FOREIGN_UNIT')
        copy_test_elf(self.phase, elf_path, self.root / 'native-test')
        (self.root / 'runs').mkdir(mode=0o700)
        (self.root / 'observations').mkdir(mode=0o700)
        network_name = self.owner.project + '_backend'
        found = self.owner.output(['network', 'ls', '--filter', 'name=^' + network_name + '$', '--format', '{{.ID}}'])
        require(not found, 'NATIVE_FOREIGN_NETWORK')
        # An uncertain create acknowledgement retains the actual intended name
        # and fixture. It must never be retried/adopted as a fresh installation.
        self.save(dict(schema='private-native4-service-v1', root=list(self.owner.root_id), commit=self.owner.commit,
            assets={}, backend=None, attempts=[], state='FAILED'))
        try:
            backend = self.owner.output(['network', 'create', '--label', 'com.docker.compose.project=' + self.owner.project,
                '--label', 'com.docker.compose.network=backend', network_name])
            require(re.fullmatch('[0-9a-f]{64}', backend) is not None, 'NATIVE_BACKEND_UNKNOWN')
            self.save(dict(schema='private-native4-service-v1', root=list(self.owner.root_id), commit=self.owner.commit,
                assets={}, backend=backend, attempts=[], state='FAILED'))
        except BaseException:
            self.owner.original_custody_uncertain = True
            raise
        args = types.SimpleNamespace(binding=str(self.binding_path), release_root=str(self.owner.release),
            commit=self.owner.commit, env_file=self.binding['production_env_file'],
            calendar_dir=self.binding['calendar_dir'], state_dir=self.binding['state_dir'],
            lock_file=self.binding['lock_file'], systemd_dir=str(self.g.SYSTEMD), mode='install-disabled')
        _PRIVATE_INSTALLERS.add(self)
        def render(b, binding, state_dir, lock_file):
            require(self in _PRIVATE_INSTALLERS and b == self.binding and binding == str(self.binding_path)
                and state_dir == self.binding['state_dir'] and lock_file == self.binding['lock_file'],
                'NATIVE_INSTALL_OWNER')
            return private_service_body(self.owner, b, binding, state_dir, lock_file)
        try:
            import io
            with contextlib.redirect_stdout(io.StringIO()):
                self.e.install_witness(args, _render_service=render,
                    _systemctl=lambda *argv: self.g.ctl(self.owner, *argv))
            self.drop.mkdir(mode=0o755)
            os.chmod(self.drop, 0o755)
            self.g.write_new(self.drop / 'private-policy.conf', native_drop_body(self.owner, self.policy), 0o644)
            self.g.ctl(self.owner, 'daemon-reload')
            self.save(dict(schema='private-native4-service-v1', root=list(self.owner.root_id), commit=self.owner.commit,
                assets=self.originals(), backend=backend, attempts=[], state='INSTALLED'))
            self.load()
            self.g.stopped(self.owner, self.state())
            self.g.host_units(self.owner, [self.g.unit_name(self.owner)])
        except BaseException:
            self.owner.original_custody_uncertain = True
            raise
        finally:
            _PRIVATE_INSTALLERS.discard(self)
        return dict(status='INSTALLED_DISABLED', native_cases='NOT_RUN', archive_acceptance=False)

    def session(self):
        """One bounded future session, with actual stopped proof before return."""
        import fcntl
        import uuid
        self.phase.quiescent()
        value = self.load()
        require(value['state'] in ('INSTALLED', 'STOPPED'), 'NATIVE_SERVICE_NOT_SETTLED')
        self.g.host_units(self.owner, [self.g.unit_name(self.owner)])
        self.g.stopped(self.owner, self.state())
        self.g.provider_idle(self.owner)
        generation = str(uuid.uuid4())
        attempt = dict(generation=generation, boot_id=Path('/proc/sys/kernel/random/boot_id').read_text().strip(),
            observed=self.e.sample(), deadline_monotonic_ns=self.phase.deadline.value, process=None, arm_sha256=None,
            result_sha256=None, disposition='STARTING')
        invocation = dict(schema='private-native4-invocation-v1', generation=generation,
            boot_id=attempt['boot_id'], deadline_monotonic_ns=self.phase.deadline.value,
            root=list(self.owner.root_id), commit=self.owner.commit)
        original = self.root / 'runs' / (generation + '.json')
        self.g.write_new(original, self.e.canonical(invocation), 0o400)
        value['attempts'].append(attempt)
        value['state'] = 'STARTING'
        self.save(value)
        # Keep the exact OFD open. The actual service must acquire a separate
        # OFD, then release it after its real Source/native cleanup.
        fcntl.flock(self.owner.lock_fd, fcntl.LOCK_UN)
        settled = False
        try:
            self.g.ctl(self.owner, '--no-block', 'start', self.g.unit_name(self.owner))
            while True:
                self.phase.deadline.seconds()
                require(self.load()['attempts'][-1]['generation'] == generation, 'NATIVE_ATTEMPT_CHANGED')
                state = self.state()
                if state['Job'] not in ('', '0'):
                    time.sleep(min(0.1, self.phase.deadline.seconds()))
                    continue
                if state['ActiveState'] in ('inactive', 'failed'):
                    self.g.stopped(self.owner, state)
                    settled = True
                    require(state['Result'] == 'success' and state['ExecMainStatus'] == '0'
                        and attempt['process'] is not None and attempt['arm_sha256'] is not None,
                        'NATIVE_SERVICE_FAILED')
                    break
                require(state['ActiveState'] in ('activating', 'active'), 'NATIVE_SERVICE_UNKNOWN')
                if state['ActiveState'] == 'active':
                    if attempt['arm_sha256'] is None:
                        self.observe_start(attempt, state)
                    else:
                        require(self.process(state) == attempt['process'], 'NATIVE_PROCESS_CHANGED')
                    value['state'] = 'RUNNING'
                    self.save(value)
                time.sleep(min(0.1, self.phase.deadline.seconds()))
        finally:
            if not settled:
                self.phase.deadline.cleanup_phase()
                try:
                    self.g.ctl(self.owner, '--no-block', 'stop', self.g.unit_name(self.owner))
                    while True:
                        self.phase.deadline.seconds(cleanup=True)
                        state = self.state()
                        if state['Job'] in ('', '0') and state['ActiveState'] in ('inactive', 'failed'):
                            self.g.stopped(self.owner, state)
                            settled = True
                            break
                        time.sleep(min(0.1, self.phase.deadline.seconds(cleanup=True)))
                except BaseException:
                    self.owner.original_custody_uncertain = True
            if settled:
                fcntl.flock(self.owner.lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                self.phase.revalidate(cleanup=True)
                self.phase.quiescent(cleanup=True)
                require(not self.owner.docker.object('network', value['backend'])['Containers'],
                        'NATIVE_BACKEND_UNSETTLED')
            else:
                self.owner.original_custody_uncertain = True
                raise NativeFixtureError('NATIVE_SERVICE_CLEANUP_UNKNOWN')
        output = self.root / 'runs' / (generation + '.result.json')
        raw = self.e.read_file(output, 65536, exact=(0, 0, 0o400))
        result = self.e.decode(raw)
        require(result['generation'] == generation and result['process'] == attempt['process']['identity']
            and result['boot_id'] == attempt['boot_id'] and result['cleanup'] == 'COMPLETE', 'NATIVE_SERVICE_RESULT')
        attempt.update(result_sha256=self.e.digest(raw), disposition='CLOSED')
        value['state'] = 'STOPPED'
        self.save(value)
        self.load()
        return dict(status='SESSION_CLOSED', session=result['session'], count=result['session_count'],
            native_cases=len(result['cases']), archive_acceptance=False)

    def observations(self):
        value = self.load()
        self.g.stopped(self.owner, self.state())
        require(value['state'] == 'STOPPED' and all(a['disposition'] == 'CLOSED' for a in value['attempts']),
                'NATIVE_SERVICE_NOT_SETTLED')
        rows, originals = [], {}
        for attempt in value['attempts']:
            raw = self.e.read_file(self.root / 'runs' / (attempt['generation'] + '.result.json'), 65536,
                                   exact=(0, 0, 0o400))
            result = self.e.decode(raw)
            require(self.e.digest(raw) == attempt['result_sha256'] and result['generation'] == attempt['generation']
                and result['process'] == attempt['process']['identity'] and result['cleanup'] == 'COMPLETE',
                'NATIVE_SERVICE_RESULT')
            for row in result['cases']:
                output = self.e.read_file(self.root / 'observations' / (row['output_sha256'][7:] + '.out'),
                                          65536, exact=(0, 0, 0o400))
                named_success(output, row['name'].removeprefix(PREFIX), 0)
                require(self.e.digest(output) == row['output_sha256'], 'NATIVE_OUTPUT_CHANGED')
                rows.append(row)
            for key, leaf in result['originals'].items():
                require(key in ('committed', 'absent') and key not in originals
                    and re.fullmatch('[0-9a-f]{64}', leaf) is not None, 'NATIVE_ORIGINAL_SELECTION')
                originals[key] = self.owner.root / 'native4-originals' / leaf
        return rows, originals

    def retire(self):
        self.phase.quiescent()
        value = self.load()
        self.g.host_units(self.owner, [self.g.unit_name(self.owner)])
        self.g.stopped(self.owner, self.state())
        require(value['state'] in ('INSTALLED', 'STOPPED') and not self.owner.docker.object('network', value['backend'])['Containers'],
                'NATIVE_SERVICE_NOT_SETTLED')
        # Revalidate actual originals immediately before removing only ours.
        require(self.originals() == value['assets'], 'NATIVE_SERVICE_CHANGED')
        self.original_stack.close()
        self.held_originals.clear()
        (self.drop / 'private-policy.conf').unlink()
        self.drop.rmdir()
        self.unit.unlink()
        self.g.ctl(self.owner, 'daemon-reload')
        require(self.state(absent=True)['LoadState'] == 'not-found', 'NATIVE_UNIT_REMAINS')
        self.owner.output(['network', 'rm', value['backend']], cleanup=True)
        self.g.host_units(self.owner)
        value['state'] = 'RETIRED'
        self.save(value)


def copy_test_elf(phase, source, destination):
    import hashlib
    e = history().source_module().evidence()
    with e.opened(source, exact=(0, 0, 0o500)) as fd:
        before = os.fstat(fd)
        require(0 < before.st_size <= 512 * 1024 * 1024 and os.pread(fd, 4, 0) == b'\x7fELF', 'NATIVE_TEST_ELF')
        out = os.open(destination, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC, 0o500)
        try:
            offset = 0
            while chunk := os.pread(fd, 1024 * 1024, offset):
                phase.deadline.seconds()
                position = 0
                while position < len(chunk):
                    position += os.write(out, chunk[position:])
                offset += len(chunk)
            require(offset == before.st_size and file_identity(os.fstat(fd)) == file_identity(before)
                and file_identity(os.stat(source, follow_symlinks=False)) == file_identity(before), 'NATIVE_TEST_ELF_CHANGED')
            os.fchmod(out, 0o500)
            os.fsync(out)
        finally:
            os.close(out)
    with e.opened(Path(destination).parent, directory=True) as parent:
        os.fsync(parent)


def session_count(phase, binding):
    """Count actual complete linked sessions, never a caller progress counter."""
    cohort = binding['cohort']['cohort_sha256']
    require(re.fullmatch('sha256:[0-9a-f]{64}', cohort) is not None, 'NATIVE_COHORT')
    rows = phase.owner.observer.json("SELECT coalesce(jsonb_agg(x.session_date ORDER BY x.session_date),'[]'::jsonb) "
        "FROM (SELECT DISTINCT s.session_date FROM public.owner_beta_paper_source_slots s "
        "JOIN public.owner_beta_paper_source_runs r ON r.slot_id=s.slot_id "
        "JOIN public.owner_beta_paper_source_admissions a ON a.slot_id=s.slot_id "
        "WHERE s.cohort_sha256='" + cohort + "') x;", role='migration_owner')
    require(type(rows) is list and len(rows) <= 4 and len(set(rows)) == len(rows), 'NATIVE_SESSION_COUNT')
    return len(rows)


def run_session_cases(phase, scheduler, caller):
    """All Source-side dispatch is in the actual publisher's retained scope."""
    count = session_count(phase, scheduler.b)
    require(1 <= count <= 4, 'NATIVE_SESSION_COUNT')
    originals = {}
    if count == 3:
        caller.source_case(scheduler, CASES[1][0])
    elif count == 4:
        caller.source_case(scheduler, CASES[0][0])
        # Both originals are produced under the same genuine current Source
        # scope, before it is revoked; ABSENT never dispatches its SQL.
        originals['absent'] = caller.preserve_enable_original(scheduler, commit=False).name
        originals['committed'] = caller.preserve_enable_original(scheduler, commit=True).name
    return count, originals


def service_run(root):
    """Only the retained exact private unit may invoke this installed entrypoint."""
    import uuid
    h = history()
    f = h.fixture_module()
    g = h.source_module()
    e = g.evidence()
    root = Path(root)
    require(root.is_absolute() and root.parent == Path('/var/lib')
        and root.name.startswith('lagrange-paper-operations-fixture-'), 'NATIVE_SERVICE_ROOT')
    checkpoint = e.decode(e.read_file(root / CHECKPOINT, 65536, exact=(0, 0, 0o400)))
    commit = checkpoint['commit']
    release = root / 'releases' / commit
    require(Path(__file__).resolve() == release / 'scripts/ops/lib/owner-beta-paper-native-source.py',
            'NATIVE_INSTALLED_ENTRYPOINT_REQUIRED')
    ledger = e.decode(e.read_file(root / SERVICE_STORE / 'service.json', 65536, exact=(0, 0, 0o600)))
    require(ledger['state'] in ('STARTING', 'RUNNING') and ledger['attempts'], 'NATIVE_SERVICE_INVOCATION')
    attempt = ledger['attempts'][-1]
    generation = str(uuid.UUID(attempt['generation']))
    path = root / SERVICE_STORE / 'runs' / (generation + '.json')
    raw = e.read_file(path, 65536, exact=(0, 0, 0o400))
    invocation = e.decode(raw)
    require(set(invocation) == {'schema', 'generation', 'boot_id', 'deadline_monotonic_ns', 'root', 'commit'}
        and invocation['schema'] == 'private-native4-invocation-v1' and invocation['generation'] == generation
        and invocation['commit'] == commit and invocation['root'] == list(f.faults.directory_identity(root))
        and invocation['boot_id'] == Path('/proc/sys/kernel/random/boot_id').read_text().strip()
        and invocation['deadline_monotonic_ns'] == attempt['deadline_monotonic_ns'], 'NATIVE_SERVICE_INVOCATION')
    deadline = h.phase_deadline(invocation['deadline_monotonic_ns'])
    manifest = e.read_file(release / '.lagrange-release-manifest', 65536, exact=(0, 0, 0o600))
    inputs_path = root / SERVICE_STORE / 'inputs.json'
    inputs_raw = e.read_file(inputs_path, 65536, exact=(0, 0, 0o400))
    inputs = e.decode(inputs_raw)
    caller = None
    with f.termination_cleanup(), f.bounded_run(deadline):
        with OwnedPhase(release, commit, manifest, deadline, root / CHECKPOINT,
                        root / SERVICE_STORE / 'policy') as phase:
            private = PrivateService(phase)
            private.load()
            state = private.state()
            parent = private.process(state)
            require(parent['identity'] == e.process(os.getpid()) and parent['boot_id'] == invocation['boot_id']
                and parent['started_monotonic_us'] * 1000 >= attempt['observed']['monotonic_ns'],
                'NATIVE_SERVICE_PARENT')
            require(e.read_file(path, 65536, exact=(0, 0, 0o400)) == raw, 'NATIVE_INVOCATION_CHANGED')
            # Material stays borrowed until every actual helper/caller closes.
            with acquisition_lease(phase, inputs), h.HistoricalMaterial(
                    release, inputs['approved_base'], inputs['raw_root'], deadline) as material:
                runtime = h.initialize_runtime(phase.owner, material, inputs['config_source'], existing=True)
                h.seed_user_and_control(runtime, existing=True)
                try:
                    with phase.session(private.binding_path, runtime) as scheduler:
                        with NativeCaller(phase, private.root / 'native-test') as caller:
                            caller.check_identity()
                            count, originals = run_session_cases(phase, scheduler, caller)
                        session = scheduler.j.session
                        material.revalidate()
                finally:
                    runtime.close()
                require(e.read_file(inputs_path, 65536, exact=(0, 0, 0o400)) == inputs_raw,
                        'NATIVE_INPUTS_CHANGED')
            private.load()
            require(private.process(private.state()) == parent, 'NATIVE_PARENT_CHANGED')
        rows = caller.report()
        for row in rows:
            output = caller.outputs[row['name'].removeprefix(PREFIX)]
            require(e.digest(output) == row['output_sha256'], 'NATIVE_OUTPUT_CHANGED')
            g.write_new(root / SERVICE_STORE / 'observations' / (row['output_sha256'][7:] + '.out'), output, 0o400)
        result = dict(schema='private-native4-session-result-v1', generation=generation,
            process=parent['identity'], boot_id=invocation['boot_id'], session=session,
            session_count=count, cases=rows, originals=originals, cleanup='COMPLETE', archive_acceptance=False)
        g.write_new(root / SERVICE_STORE / 'runs' / (generation + '.result.json'), e.canonical(result), 0o400)
        require(e.read_file(path, 65536, exact=(0, 0, 0o400)) == raw, 'NATIVE_INVOCATION_CHANGED')
    return dict(status='SESSION_OBSERVED', archive_acceptance=False)


def accept(phase):
    """Join retained service observations with actual offline native readbacks.

    The return is pending until the surrounding phase closes. This method does
    not turn an external JSON readiness report into four successes.
    """
    private = PrivateService(phase)
    private.load()
    rows, originals = private.observations()
    require(len(rows) == 2 and {r['name'] for r in rows} == {PREFIX + CASES[0][0], PREFIX + CASES[1][0]}
        and set(originals) == {'committed', 'absent'}, 'NATIVE_SOURCE_CASES_REQUIRED')
    phase.quiescent()
    control = phase.owner.observer.json('SELECT to_jsonb(c) FROM public.owner_beta_paper_control c;', role='migration_owner')
    require(control['state'] == 'ENABLED', 'NATIVE_COMMIT_REQUIRED')
    later_off(phase)
    with NativeCaller(phase, private.root / 'native-test') as caller:
        caller.check_identity()
        caller.readback_case(originals['committed'], committed=True)
        caller.readback_case(originals['absent'], committed=False)
    phase.quiescent()
    # Remove only the retained exact private service and empty owned egress
    # network after all native borrowers/DB backends have actually settled.
    private.retire()
    return rows, caller


def continue_phase(source, commit, manifest, deadline, checkpoint, action, *, elf_path=None):
    h = history()
    f = h.fixture_module()
    root = Path(checkpoint).parent
    policy = root / SERVICE_STORE / 'policy'
    require(action in ('install', 'session', 'accept', 'dispose'), 'NATIVE_PHASE_SELECTION')
    pending = None
    with f.termination_cleanup(), f.bounded_run(deadline), NativePolicy(policy):
        with OwnedPhase(source, commit, manifest, deadline, checkpoint, policy) as phase:
            if action == 'install':
                require(elf_path is not None, 'NATIVE_TEST_ELF_REQUIRED')
                result = PrivateService(phase).install(elf_path)
            elif action == 'session':
                result = PrivateService(phase).session()
            elif action == 'accept':
                pending = accept(phase)
                dispose_native_phase(phase)
            else:
                private = PrivateService(phase)
                private.retire()
                dispose_native_phase(phase)
                result = dict(status='DISPOSED', archive_acceptance=False)
        if pending is not None:
            rows, caller = pending
            result = aggregate_cases(rows + caller.report())
            # Final finite result is emitted only after DB/network/root removal.
            require(phase.owner.cleanup.get('containers') == 'REMOVED'
                and phase.owner.cleanup.get('inputs') == 'REMOVED_AFTER_DISPOSITION', 'NATIVE_DISPOSAL_UNPROVEN')
            result['fixture_disposition'] = 'REMOVED'
    return result


def file_identity(value):
    return (value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns, value.st_ctime_ns,
            value.st_uid, value.st_gid, value.st_mode, value.st_nlink)


def dispose_native_phase(phase):
    """Actual disposal only after service retirement and publisher-lock proof."""
    import fcntl
    h = history()
    g = h.source_module()
    e = g.evidence()
    o = phase.owner
    phase.quiescent()
    require(not any(c.phase is phase for c in _CALLERS) and not any(p is phase for p in _SCHEDULERS.values()),
            'NATIVE_BORROWERS_OPEN')
    ledger = e.decode(e.read_file(o.root / SERVICE_STORE / 'service.json', 65536, exact=(0, 0, 0o600)))
    require(ledger['state'] == 'RETIRED' and ledger['root'] == list(o.root_id), 'NATIVE_SERVICE_NOT_RETIRED')
    g.absent(g.SYSTEMD / g.unit_name(o))
    g.absent(Path(str(g.SYSTEMD / g.unit_name(o)) + '.d'))
    g.host_units(o)
    g.provider_idle(o)
    b, _ = e.binding(str(o.root / SERVICE_STORE / 'binding.json'))
    for path in (Path(b['lock_file']), Path(b['evidence_root']) / '.control/lock'):
        fd = phase.stack.enter_context(e.opened(path, write=True, exact=(0, 0, 0o600)))
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    with e.opened(Path(b['evidence_root']) / 'current' / e.CURRENT_LEAF, exact=(0, 10001, 0o440)) as fd:
        current = e.decode(os.pread(fd, 65537, 0))
        e.validate_current(current)
        require(current['status'] == 'INVALID' and current['binding'] == b, 'NATIVE_SOURCE_STILL_CURRENT')
        probe = e.OfdFlock(fcntl.F_WRLCK, os.SEEK_SET, 0, 0, 0)
        require(e.ctypes.sizeof(e.OfdFlock) == 32 and e.OfdFlock.from_buffer_copy(
            fcntl.fcntl(fd, getattr(fcntl, 'F_OFD_GETLK', 36), bytes(probe))).type == fcntl.F_UNLCK,
            'NATIVE_SOURCE_FENCE_HELD')
    phase.revalidate()
    o.deadline.cleanup_phase()
    try:
        def close_originals():
            # The actual fixture supervisor calls this only after owned helper,
            # backend, PG container and network disposition, before tree removal.
            phase.stack.close()
            o.lock_fd = None  # stack owned this exact descriptor
        o.dispose(close_originals)
        require(o.cleanup['containers'] == 'REMOVED' and o.cleanup['inputs'] == 'REMOVED_AFTER_DISPOSITION'
            and not os.path.lexists(o.root), 'NATIVE_DISPOSAL_UNPROVEN')
    except BaseException:
        o.original_custody_uncertain = True
        _UNRESOLVED_PHASES.add(phase)
        raise
    phase.closed = True
    _PHASES.discard(phase)


@contextlib.contextmanager
def acquisition_lease(phase, inputs):
    """Retain the actual external shared state and copied credential originals."""
    h = history()
    g = h.source_module()
    e = g.evidence()
    acquisition = inputs['acquisition']
    root, state = Path(acquisition['input_root']), Path(acquisition['coordination_root'])
    require(state == root / 'state', 'NATIVE_SHARED_STATE_SELECTION')
    stack = contextlib.ExitStack()
    phase.stack.callback(stack.close)  # UNKNOWN keeps the entire phase alive.
    pins = []
    for path, owners, exact in ((root, (0,), (0, 0, 0o700)),
            (state, (0,), (0, 10001, 0o750)),
            (state / 'kis-read-coordination', (0, 10001), (10001, 10001, 0o700))):
        fd = stack.enter_context(e.opened(path, directory=True, owners=owners, exact=exact))
        pins.append((path, fd, g.inode(os.fstat(fd))))
    require(pins[1][2] == acquisition['coordination_identity'], 'NATIVE_SHARED_STATE_CHANGED')
    originals = []
    for leaf in ('kis_app_key', 'kis_app_secret'):
        source = stack.enter_context(e.opened(root / leaf, exact=(0, 0, 0o400)))
        raw = os.pread(source, 4097, 0)
        require(0 < len(raw) <= 4096, 'NATIVE_CREDENTIAL_ORIGINAL')
        originals.append((root / leaf, source, file_identity(os.fstat(source)), raw))
        for service in ('research-worker', 'research-range-raw'):
            target = phase.owner.root / 'secrets' / service / leaf
            fd = stack.enter_context(e.opened(target, owners=(0, 10001), exact=(10001, 10001, 0o400)))
            require(os.pread(fd, 4097, 0) == raw, 'NATIVE_CREDENTIAL_CHANGED')
            originals.append((target, fd, file_identity(os.fstat(fd)), raw))
    def check():
        phase.revalidate(cleanup=True)
        for path, fd, identity in pins:
            require(g.inode(os.fstat(fd)) == identity == g.inode(os.stat(path, follow_symlinks=False)),
                    'NATIVE_SHARED_STATE_CHANGED')
        for path, fd, identity, raw in originals:
            require(file_identity(os.fstat(fd)) == identity == file_identity(os.stat(path, follow_symlinks=False))
                and os.pread(fd, len(raw) + 1, 0) == raw, 'NATIVE_CREDENTIAL_CHANGED')
    check()
    try:
        yield
        check()
    except BaseException:
        phase.owner.original_custody_uncertain = True
        raise
    else:
        stack.close()
