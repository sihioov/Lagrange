"""Owned private lifetime for the unchanged immutable Source service.

Receipts locate originals. Neither a receipt nor any return value is native,
Source-session, account, trading, or acceptance authority. Unknown custody never
permits fixture disposal. Imports have no runtime effects.
"""
from __future__ import annotations

import contextlib
import fcntl
import importlib.util
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import tempfile
import time
import types

SYSTEMD = Path('/etc/systemd/system')
STORE = '.genuine-source'
RECEIPT = 'custody.json'
LIMIT = 65536
MAX_NS = 90 * 60 * 10**9
ENV = {'PATH': '/usr/sbin:/usr/bin:/sbin:/bin', 'HOME': '/root', 'LANG': 'C.UTF-8',
       'PYTHONDONTWRITEBYTECODE': '1', 'DOCKER_HOST': 'unix:///var/run/docker.sock'}


class SourceCustodyUnknown(Exception):
    """Finite diagnostic: retain original roots and DB on failure."""


def require(value, code='SOURCE_DISPOSITION_UNPROVEN'):
    if not value:
        raise SourceCustodyUnknown(code)


def history():
    name = 'owner_beta_paper_genuine_history'
    path = Path(__file__).with_name('owner-beta-paper-genuine-history.py')
    if name not in sys.modules:
        spec = importlib.util.spec_from_file_location(name, path)
        module = importlib.util.module_from_spec(spec)
        sys.modules[name] = module
        spec.loader.exec_module(module)
    module = sys.modules[name]
    require(Path(module.__file__).resolve() == path.resolve(), 'SOURCE_MODULE')
    return module


def evidence():
    return history().load('paper_genuine_service_evidence', 'scripts/ops/lib/owner-beta-paper-evidence.py')


def tick(owner, cap=30):
    require(type(owner.deadline.value) is int and 0 < owner.deadline.value - time.monotonic_ns() <= MAX_NS,
            'SOURCE_DEADLINE')
    return owner.deadline.seconds(cap, cleanup=True)


def command(owner, argv, *, missing_unit=False):
    """Fixed internal argv only; never return/print provider or command prose."""
    timeout = tick(owner)
    try:
        # Temporary regular output bounds memory; RLIMIT_FSIZE also bounds disk.
        # No shell, inherited descriptors, stdin, or caller environment.
        with tempfile.TemporaryFile() as output:
            import resource
            def limits():
                resource.setrlimit(resource.RLIMIT_FSIZE, (LIMIT, LIMIT))
            result = subprocess.run(argv, stdin=subprocess.DEVNULL, stdout=output,
                stderr=subprocess.DEVNULL, env=ENV, close_fds=True, timeout=timeout,
                check=False, preexec_fn=limits)
            require(output.tell() <= LIMIT, 'SOURCE_COMMAND_UNKNOWN')
            output.seek(0)
            raw = output.read(LIMIT + 1)
        tick(owner)
        require(len(raw) <= LIMIT, 'SOURCE_COMMAND_UNKNOWN')
        text = raw.decode('ascii', 'strict')
        require(result.returncode == 0 or missing_unit and result.returncode in (1, 4)
            and 'LoadState=not-found' in text.splitlines(), 'SOURCE_COMMAND_UNKNOWN')
        return text
    except (OSError, UnicodeError, subprocess.SubprocessError):
        raise SourceCustodyUnknown('SOURCE_COMMAND_UNKNOWN') from None


def ctl(owner, *args):
    argv = ['/usr/bin/systemctl', '--no-pager', *args]
    if args[:1] == ('show',):
        # Some systemd versions return a missing-unit status alongside typed
        # properties. unit_state still requires the complete exact envelope.
        return command(owner, argv, missing_unit=True)
    return command(owner, argv)


def unit_name(owner):
    require(re.fullmatch('[0-9a-f]{40}', owner.commit) is not None, 'SOURCE_OWNER')
    return 'lagrange-kis-witness-' + owner.commit[:7] + '.service'


def inode(info):
    return [info.st_dev, info.st_ino, info.st_uid, info.st_gid, stat.S_IMODE(info.st_mode)]


def original(path, mode, *, directory=False, members=None):
    """Use the Source no-link/no-ACL protected walk, plus original generations."""
    e = evidence()
    with e.opened(path, directory=directory, exact=(0, 0, mode)) as fd:
        info = os.fstat(fd)
        value = {'identity': inode(info)}
        if directory:
            if members is not None:
                require(sorted(os.listdir(fd)) == sorted(members), 'SOURCE_MEMBERS_CHANGED')
                value['members'] = sorted(members)
        else:
            require(0 <= info.st_size <= LIMIT, 'SOURCE_FILE_CHANGED')
            data = os.read(fd, LIMIT + 1)
            value.update(size=info.st_size, mtime=info.st_mtime_ns, ctime=info.st_ctime_ns,
                         sha256=e.digest(data))
            require(len(data) == info.st_size, 'SOURCE_FILE_CHANGED')
        return value


def absent(path):
    # lexists, including dangling links. Parent walk forbids symlink ancestors.
    with evidence().opened(Path(path).parent, directory=True) as fd:
        try:
            os.stat(Path(path).name, dir_fd=fd, follow_symlinks=False)
        except FileNotFoundError:
            return
    raise SourceCustodyUnknown('SOURCE_FOREIGN_UNIT')


def write_new(path, data, mode=0o600):
    # The caller already owns the protected parent. O_EXCL forbids retrofits.
    with evidence().opened(Path(path).parent, directory=True) as parent:
        fd = os.open(Path(path).name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW | os.O_CLOEXEC,
                     mode, dir_fd=parent)
        try:
            os.fchmod(fd, mode)
            offset = 0
            while offset < len(data):
                offset += os.write(fd, data[offset:])
            os.fsync(fd)
        finally:
            os.close(fd)
        os.fsync(parent)


def save(owner, receipt):
    root = owner.root / STORE
    # Atomic description update, with no external content or secret values.
    raw = evidence().canonical(receipt)
    require(len(raw) <= LIMIT, 'SOURCE_RECEIPT')
    temporary = root / ('.receipt-' + os.urandom(12).hex())
    write_new(temporary, raw)
    os.replace(temporary, root / RECEIPT)
    with evidence().opened(root, directory=True, exact=(0, 0, 0o700)) as fd:
        os.fsync(fd)


def owned(owner, *, prepared=False):
    h = history()
    fixture = h.fixture_module()
    require(isinstance(owner, fixture.FixtureOwner), 'SOURCE_OWNER')
    tick(owner)
    require(fixture.faults.directory_identity(owner.root) == owner.root_id, 'SOURCE_ROOT_CHANGED')
    require(owner.root.parent == Path('/var/lib') and owner.root.name.startswith('lagrange-paper-operations-fixture-'),
            'SOURCE_OWNER')
    if prepared:
        h.prepared_release(owner)
    if owner.lock_fd is not None:
        fixture.ops._file(owner.lock_fd, 0o600)
        require(inode(os.fstat(owner.lock_fd)) == inode(os.stat(owner.root / fixture.ops.LOCK_NAME,
            follow_symlinks=False)), 'SOURCE_LOCK_CHANGED')


def owner_snapshot(owner):
    """Read actual release/journal/account/DB originals while original OFD held."""
    owned(owner, prepared=True)
    h = history()
    ops = h.fixture_module().ops
    require(owner.lock_fd is not None and not owner.original_custody_uncertain, 'SOURCE_LOCK_UNKNOWN')
    with ops.journal.open_journal(str(owner.root), owner.lock_fd) as journal:
        state = {'installation': journal.installation_id, 'floor': journal.epoch_floor}
        journal.revalidate()
    ops.daily.policy.validate_current_release(str(owner.root), str(owner.release), owner.commit)
    require(ops.daily.install.read_root_file(str(owner.release / ops.release.MANIFEST_NAME),
        gid=0, mode=0o600, maximum=LIMIT) == owner.manifest, 'SOURCE_RELEASE_CHANGED')
    with ops.release.verify(str(owner.root), str(owner.release), owner.commit) as checked:
        require(checked.process_sha256 == owner.prepared_process_sha256 and
            h.release_descriptors(owner, checked) == (owner.release_hashes, owner.producer_hashes), 'SOURCE_RELEASE_CHANGED')
        checked.revalidate()
    values, overlays = ops.daily.read_context(str(owner.release), owner.commit, 'disable')
    require(values == owner.context.values and overlays == owner.context.overlays, 'SOURCE_CONTEXT_CHANGED')
    owner.observer.target()
    require(owner.observer.json('SELECT to_json(account_id) FROM owner_beta_paper_control;', role='migration_owner')
            == owner.account, 'SOURCE_ACCOUNT_CHANGED')
    state.update(root=list(owner.root_id), lock=inode(os.fstat(owner.lock_fd)),
        db=list(owner.observer.generation), account=owner.account, manifest=evidence().digest(owner.manifest),
        release=original(owner.release, stat.S_IMODE(os.stat(owner.release).st_mode), directory=True))
    return state


def binding_owner(owner, path):
    """Reuse binding-assets and helper-context validators, then bind this owner."""
    e = evidence()
    b, raw, _projection = e._binding_with_helper_projection(str(path))
    values = owner.context.values
    network = owner.docker.object('network', owner.network)
    require(network['Id'] == owner.network and network['Internal'] is True
        and network['Labels']['com.docker.compose.project'] == owner.project, 'SOURCE_NETWORK_CHANGED')
    require(b['release_root'] == str(owner.release) and b['cohort']['verifier_revision'] == owner.commit
        and b['project'] == owner.project and b['network'] == network['Name']
        and b['database'] == (values['POSTGRES_DB'] or 'lagrange')
        and b['data_root'] == str(owner.root / 'data')
        and b['secret_root'] == str(owner.root / 'secrets')
        and b['production_env_file'] == str(owner.release / 'deploy/compose/.env')
        and b['cohort']['release_manifest_sha256'] == e.digest(owner.manifest), 'SOURCE_BINDING_OWNER')
    # Immutable prepared process binds account, strategy, consent and cohort.
    raw_process = e.read_file(owner.release / 'deploy/owner-beta-paper/process.json', LIMIT, exact=(0, 10001, 0o440))
    process = e.decode(raw_process)
    require(e.digest(raw_process) == owner.prepared_process_sha256 and process['account_id'] == owner.account
        and process['daily_cohort_sha256'] == b['cohort']['cohort_sha256']
        and process['owner_user_id'] == b['cohort']['owner_user_id']
        and process['strategy_config_id'] == b['cohort']['strategy_config_id'], 'SOURCE_BINDING_OWNER')
    for key in ('calendar_dir', 'calendar_file', 'state_dir', 'lock_file'):
        require(Path(b[key]).is_relative_to(owner.root), 'SOURCE_BINDING_OWNER')
    return b, raw


def policy_bytes(owner, policy_root):
    e = evidence()
    selected = Path(policy_root)
    if selected.parent == Path('/var/tmp') and selected.name.startswith('lagrange-paper-genuine-policy-'):
        # Only the policy leaf is projected. Actual data/config/state originals
        # remain visible under the unchanged /var/lib/lagrange parent.
        fixture = history().fixture_module()
        try:
            fixture.service_status_namespace(selected, os.getpid())
            selected = fixture.SERVICE_STATUS_TARGET
        except (fixture.FixtureError, OSError):
            raise SourceCustodyUnknown('SOURCE_POLICY_MOUNT') from None
        path = selected / 'policy.json'
    else:
        require(selected == paths(owner)[3], 'SOURCE_POLICY_OWNER')
        path = selected / 'service-status/policy.json'
    raw = e.read_file(path, LIMIT, exact=(0, 0, 0o600))
    value = e.decode(raw, canonical_required=False)
    history().fixture_module().ops.daily.policy.validate_policy(value)
    require(value['release_commit'] == owner.commit and value['compose_project'] == owner.project, 'SOURCE_POLICY_OWNER')
    return raw


def paths(owner):
    root = owner.root / STORE
    unit = SYSTEMD / unit_name(owner)
    return root, unit, Path(str(unit) + '.d'), root / 'policy', root / 'binding.json'


def drop_body(policy):
    require(re.fullmatch('/[A-Za-z0-9/._-]+', str(policy)) is not None, 'SOURCE_POLICY_PATH')
    return history().fixture_module().service_status_drop_body(policy)


def assets(owner, *, retired=False):
    root, unit, drop, policy, binding = paths(owner)
    b, _ = evidence().structural_binding(binding)
    specs = [
            (root, 0o700, True, ['binding.json', 'policy', RECEIPT]),
            (binding, 0o600, False, None), (policy, 0o700, True, ['service-status']),
            (policy / 'service-status', 0o700, True, ['policy.json']),
            (policy / 'service-status/policy.json', 0o600, False, None),
            (SYSTEMD, 0o755, True, None)]
    for lock in (Path(b['lock_file']), Path(b['evidence_root']) / '.control/lock'):
        specs += [(lock, 0o600, False, None),
                  (lock.parent, stat.S_IMODE(os.stat(lock.parent).st_mode), True, None)]
    if not retired:
        specs += [
            (unit, 0o644, False, None), (drop, 0o755, True, ['private-policy.conf']),
            (drop / 'private-policy.conf', 0o644, False, None)]
    return {str(path): original(path, mode, directory=directory, members=members)
        for path, mode, directory, members in specs}


def load_receipt(owner):
    root, unit, drop, policy, binding = paths(owner)
    raw = evidence().read_file(root / RECEIPT, LIMIT, exact=(0, 0, 0o600))
    r = evidence().decode(raw)
    require(set(r) == {'schema', 'commit', 'root', 'binding_original', 'assets', 'phase', 'process', 'attempt'}
        and r['schema'] == 'private-genuine-source-v1' and r['commit'] == owner.commit
        and r['root'] == list(owner.root_id), 'SOURCE_RECEIPT')
    require(r['phase'] in ('INSTALLED', 'STARTING', 'ARMED', 'STOPPING', 'STOPPED', 'RETIRING', 'RETIRED'),
            'SOURCE_CUSTODY_UNKNOWN')
    retired = r['phase'] in ('RETIRING', 'RETIRED')
    if retired:
        absent(unit)
        absent(drop)
        expected = {k: v for k, v in r['assets'].items() if k not in (str(unit), str(drop), str(drop / 'private-policy.conf'))}
        require(assets(owner, retired=True) == expected, 'SOURCE_ASSETS_CHANGED')
    else:
        require(assets(owner) == r['assets'], 'SOURCE_ASSETS_CHANGED')
    b, raw = binding_owner(owner, binding)
    original_binding = Path(r['binding_original']['path'])
    require(original(original_binding, 0o600) == r['binding_original']['original']
        and original(original_binding.parent, r['binding_original']['parent']['identity'][4], directory=True)
            == r['binding_original']['parent']
        and evidence().read_file(original_binding, LIMIT, exact=(0, 0, 0o600)) == raw, 'SOURCE_BINDING_CHANGED')
    if not retired:
        require(evidence().read_file(unit, LIMIT, exact=(0, 0, 0o644)) ==
                evidence().render_service(b, str(binding), b['state_dir'], b['lock_file']).encode()
            and evidence().read_file(drop / 'private-policy.conf', LIMIT, exact=(0, 0, 0o644)) == drop_body(policy),
            'SOURCE_UNIT_CHANGED')
    policy_bytes(owner, policy)
    return r, b


PROPERTIES = ('Id', 'LoadState', 'FragmentPath', 'DropInPaths', 'ExecStart', 'MainPID', 'ExecMainStartTimestampMonotonic',
    'ControlGroup', 'InvocationID', 'ActiveState', 'SubState', 'UnitFileState', 'Job', 'NeedDaemonReload',
    'TriggeredBy', 'Triggers')


def unit_state(owner, *, absent_ok=False):
    text = ctl(owner, 'show', '--all', '--property=' + ','.join(PROPERTIES), unit_name(owner))
    state = {}
    for line in text.splitlines():
        key, sep, value = line.partition('=')
        require(sep and key in PROPERTIES and key not in state, 'SOURCE_UNIT_UNKNOWN')
        state[key] = value
    require(set(state) == set(PROPERTIES) and state['Id'] == unit_name(owner)
        and state['Job'] in ('', '0') and not state['TriggeredBy'] and not state['Triggers'], 'SOURCE_UNIT_UNKNOWN')
    if absent_ok and state['LoadState'] == 'not-found':
        require(state['MainPID'] == '0' and state['ActiveState'] == 'inactive'
                and not state['FragmentPath'] and not state['DropInPaths'], 'SOURCE_UNIT_UNKNOWN')
        return state
    _, unit, drop, _, _ = paths(owner)
    binding = paths(owner)[4]
    expected_argv = '/usr/bin/python3 ' + str(owner.release / 'scripts/ops/lib/owner-beta-paper-evidence.py') + ' run --binding ' + str(binding)
    argv = re.search(r'(?:^|; )argv\[\]=([^;]+) ;', state['ExecStart'])
    require(state['LoadState'] == 'loaded' and state['FragmentPath'] == str(unit)
        and state['DropInPaths'] == str(drop / 'private-policy.conf')
        and state['UnitFileState'] == 'disabled' and state['NeedDaemonReload'] == 'no'
        and state['ExecStart'].startswith('{ path=/usr/bin/python3 ; ')
        and argv is not None and argv[1] == expected_argv and state['ExecStart'].count('argv[]=') == 1,
        'SOURCE_UNIT_CHANGED')
    return state


def host_units(owner, allowed=()):
    """Keep production's all-unit foreign-installation guard intact."""
    ops = history().fixture_module().ops
    context = types.SimpleNamespace(install_root=str(owner.root), values={
        'LAGRANGE_DATA_DIR': str(owner.root / 'data'), 'LAGRANGE_ARTIFACTS_DIR': str(owner.root / 'artifacts'),
        'LAGRANGE_RUNTIME_SECRET_DIR': str(owner.root / 'secrets')})
    deadline = types.SimpleNamespace(remaining=lambda cap=30: tick(owner, cap))
    observer = ops._HostStopObserver(types.SimpleNamespace(context=context, deadline=deadline))
    # Its protected unit walker/validator is reused; transport remains capped.
    observer.command = lambda argv, **_: command(owner, argv)
    # Production systemctl expects command bytes.
    observer.systemctl = lambda *argv: ctl(owner, *argv)
    try:
        units = observer.units_now()
        require(set(units) <= set(allowed), 'SOURCE_FOREIGN_UNIT')
        jobs = ctl(owner, 'list-jobs', '--no-legend', '--plain')
        require(not any('lagrange-' in row or str(owner.root) in row for row in jobs.splitlines()),
                'SOURCE_FOREIGN_JOB')
        return units
    finally:
        observer.close()


def process_state(owner, state):
    pid = int(state['MainPID'])
    require(pid > 1 and state['ActiveState'] == 'active' and state['SubState'] == 'running'
        and re.fullmatch('[0-9a-f]{32}', state['InvocationID']) is not None
        and state['ControlGroup'] == '/system.slice/' + unit_name(owner)
        and int(state['ExecMainStartTimestampMonotonic']) > 0, 'SOURCE_PROCESS_UNKNOWN')
    e = evidence()
    p = e.process(pid)
    require(inode(os.stat(f'/proc/{pid}/exe')) == inode(os.stat('/usr/bin/python3')), 'SOURCE_PROCESS_CHANGED')
    require(Path(f'/proc/{pid}/cgroup').read_text() == '0::' + state['ControlGroup'] + '\n', 'SOURCE_CGROUP_CHANGED')
    _, _, _, policy, binding = paths(owner)
    b, _ = binding_owner(owner, binding)
    # Exact interpreter/argv and effective policy mount; no wrapper ExecStart.
    expected = ['/usr/bin/python3', b['release_root'] + '/scripts/ops/lib/owner-beta-paper-evidence.py',
                'run', '--binding', str(binding)]
    require(Path(f'/proc/{pid}/cmdline').read_bytes().split(b'\0') == [x.encode() for x in expected] + [b''],
            'SOURCE_PROCESS_CHANGED')
    fixture = history().fixture_module()
    try:
        mounted = fixture.service_status_namespace(policy, pid)
    except (fixture.FixtureError, OSError):
        raise SourceCustodyUnknown('SOURCE_POLICY_MOUNT') from None
    require((mounted / 'policy.json').read_bytes() == policy_bytes(owner, policy), 'SOURCE_POLICY_MOUNT')
    require(e.process(pid) == p, 'SOURCE_PROCESS_CHANGED')
    return {'identity': p, **{k: state[k] for k in ('ControlGroup', 'InvocationID', 'ExecMainStartTimestampMonotonic')},
        'namespace': list((os.stat(f'/proc/{pid}/ns/mnt').st_dev, os.stat(f'/proc/{pid}/ns/mnt').st_ino))}


def stopped(owner, state):
    require(state['ActiveState'] in ('inactive', 'failed') and state['MainPID'] == '0', 'SOURCE_NOT_STOPPED')
    cgroup = Path('/sys/fs/cgroup/system.slice') / unit_name(owner)
    if cgroup.exists():
        require(not (cgroup / 'cgroup.procs').read_text().strip()
            and 'populated 0' in (cgroup / 'cgroup.events').read_text().splitlines(), 'SOURCE_CGROUP_UNKNOWN')


def attempt_process(owner, receipt, state):
    p = process_state(owner, state)
    if receipt['process'] is not None:
        require(receipt['process'] == p, 'SOURCE_PROCESS_CHANGED')
    else:
        attempt = receipt['attempt']
        require(receipt['phase'] == 'STARTING' and type(attempt) is dict
            and attempt['boot'] == Path('/proc/sys/kernel/random/boot_id').read_text().strip()
            and int(state['ExecMainStartTimestampMonotonic']) * 1000 >= attempt['monotonic_ns'],
            'SOURCE_PROCESS_CHANGED')
    return p


def provider_idle(owner):
    """Do not assume private projects isolate broker-wide writer coordination."""
    foreign_processes(owner)
    rows = command(owner, ['/usr/bin/docker', '--host', 'unix:///var/run/docker.sock',
        'ps', '--all', '--no-trunc', '--format', '{{.ID}}']).splitlines()
    require(len(rows) <= 4096 and len(rows) == len(set(rows)), 'SOURCE_WRITER_UNKNOWN')
    for cid in rows:
        require(re.fullmatch('[0-9a-f]{64}', cid) is not None, 'SOURCE_WRITER_UNKNOWN')
        value = owner.docker.object('container', cid)
        labels = value.get('Config', {}).get('Labels') or {}
        service = labels.get('com.docker.compose.service', '')
        name = value.get('Name', '')
        running = value.get('State', {}).get('Running')
        require(type(running) is bool, 'SOURCE_WRITER_UNKNOWN')
        related = labels.get('com.docker.compose.project') == owner.project or any(
            m.get('Source', '').startswith(str(owner.root) + '/') for m in value.get('Mounts', []))
        # Even a stopped unknown private object retains inputs and can be
        # restarted; only its actual owning consumer may dispose it.
        require(cid == owner.pg or not (related or running and
            (service in ('research-worker', 'owner-beta-runner') or 'kis-' in name)),
            'SOURCE_FOREIGN_WRITER')


def foreign_processes(owner):
    # A manual collector is not necessarily represented by a systemd unit or
    # Docker label. Inspect bounded argv only; never environments or output.
    processes = [p for p in Path('/proc').iterdir() if p.name.isascii() and p.name.isdigit()]
    require(len(processes) <= 8192, 'SOURCE_WRITER_UNKNOWN')
    scripts = {'kis-daily-production.sh', 'backfill-production.sh', 'backfill-kis.py'}
    for process in processes:
        tick(owner)
        try:
            with (process / 'cmdline').open('rb') as stream:
                raw = stream.read(32769)
        except FileNotFoundError:
            continue  # exited during the observation; no process is signalled
        except OSError:
            raise SourceCustodyUnknown('SOURCE_WRITER_UNKNOWN') from None
        require(len(raw) <= 32768, 'SOURCE_WRITER_UNKNOWN')
        argv = raw.split(b'\0')
        leaves = [os.fsdecode(part).rsplit('/', 1)[-1] for part in argv[:4]]
        source = 'owner-beta-paper-evidence.py' in leaves and b'run' in argv[1:5]
        require(not source and not (set(leaves) & scripts), 'SOURCE_FOREIGN_WRITER')


def quiescent(owner):
    provider_idle(owner)
    owner.observer.target()
    value = owner.observer.json("SELECT json_build_object('clients',(SELECT count(*) FROM pg_stat_activity "
        "WHERE datname=current_database() AND backend_type='client backend' AND pid<>pg_backend_pid()),"
        "'prepared',(SELECT count(*) FROM pg_prepared_xacts WHERE database=current_database()));",
        role='migration_owner')
    require(type(value) is dict and set(value) == {'clients', 'prepared'}
        and all(type(v) is int and v == 0 for v in value.values()), 'SOURCE_BACKEND_UNKNOWN')


def current_observation(owner, b, process):
    e = evidence()
    current = e.decode(e.read_file(Path(b['evidence_root']) / 'current' / e.CURRENT_LEAF,
        LIMIT, exact=(0, 10001, 0o440)))
    e.validate_current(current)
    if current['status'] == 'INVALID' and current['reason'] == 'STARTING' and current['binding'] is None:
        require(current['publisher'] == process['identity']
            and current['boot_id'] == Path('/proc/sys/kernel/random/boot_id').read_text().strip(),
            'SOURCE_CURRENT_UNKNOWN')
        return None
    require(current['binding'] == b, 'SOURCE_CURRENT_UNKNOWN')
    if current['status'] == 'INVALID':
        # Install-disabled's original STOPPED Current is expected while the
        # real interpreter starts. Failure invalidation also waits for actual
        # stopped process/cgroup proof; it never becomes an ARM disposition.
        return None
    require(current['status'] == 'CURRENT' and current['publisher'] == process['identity'], 'SOURCE_CURRENT_UNKNOWN')
    now = e.sample()
    e.clock_step(current['observed'], now)
    require(now['monotonic_ns'] - current['observed']['monotonic_ns'] <= 30 * 10**9,
            'SOURCE_CURRENT_UNKNOWN')
    return current, now


def arm_observation(owner, b, process):
    """A genuine future ARM prefix proves barrier return, never completion."""
    e = evidence()
    observed = current_observation(owner, b, process)
    if observed is None:
        return None
    current, now = observed
    root = Path(b['evidence_root']) / 'events'
    with e.opened(root, directory=True, exact=(0, 10001, 0o750)) as fd:
        names = sorted(os.listdir(fd))
        require(len(names) <= 4096, 'SOURCE_INVENTORY_BOUND')
    found = []
    for name in names:
        tick(owner)
        require(re.fullmatch('[0-9a-f]{64}', name) is not None, 'SOURCE_EVENT_UNKNOWN')
        folder = root / name
        with e.opened(folder, directory=True, exact=(0, 10001, 0o750)) as fd:
            leaves = sorted(os.listdir(fd))
            # Source.publish stages an exact dot-UUID before immutable link.
            # Observe again instead of calling that partial publication an ARM.
            staging = [leaf for leaf in leaves if leaf.startswith('.')]
            require(all(re.fullmatch(r'\.[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}',
                leaf) is not None for leaf in staging) and len(staging) <= 1, 'SOURCE_EVENT_UNKNOWN')
            if staging:
                return None
            require(len(leaves) <= 64 and leaves == [f'{i:06}.json' for i in range(len(leaves))], 'SOURCE_EVENT_UNKNOWN')
        # Only a freshly completed ARMED prefix is a safe startup disposition.
        if len(leaves) != 3:
            continue
        events = [e.decode(e.read_file(folder / leaf, LIMIT, exact=(0, 10001, 0o440))) for leaf in leaves]
        e.verify_chain(events, 'ARMED')
        last = events[-1]
        require(last['slot_id'] == 'sha256:' + name, 'SOURCE_ARM_UNKNOWN')
        if last['arm']['parent'] != process['identity']:
            continue
        require(last['cohort'] == b['cohort'] and last['arm']['boot_id'] == current['boot_id']
            and last['due_realtime_ns'] > time.time_ns() + 60 * 10**9, 'SOURCE_ARM_UNKNOWN')
        claim = e.decode(e.read_file(Path(b['evidence_root']) / '.control' /
            (e.dispatch_key(b['cohort'], last['session']) + '.json'), LIMIT, exact=(0, 0, 0o600)))
        common = ('schema', 'slot_id', 'session', 'due_realtime_ns', 'cohort', 'arm', 'reservation_id', 'invocation_id')
        require(claim == {k: last[k] for k in common}, 'SOURCE_ARM_UNKNOWN')
        endpoint = e.sample()
        e.clock_step(now, endpoint)
        e.clock_step(last['observed'], endpoint)
        e.clock_step(current['observed'], endpoint)
        require(endpoint['monotonic_ns'] - current['observed']['monotonic_ns'] <= 30 * 10**9,
                'SOURCE_CURRENT_UNKNOWN')
        require(last['due_realtime_ns'] > endpoint['realtime_ns'] + 60 * 10**9, 'SOURCE_ARM_UNKNOWN')
        tick(owner)
        found.append(e.digest(e.canonical(last)))
    require(len(found) <= 1, 'SOURCE_ARM_UNKNOWN')
    return found[0] if found else None


def no_borrowers(owner):
    require(all(c._closed for c in owner.contexts) and not owner.helpers and not owner.helper_bindings
        and not owner.threads and not owner.blockers and not owner.tracees
        and not getattr(owner, 'archive_intakes', ()) and owner.stop_writer is None,
        'SOURCE_BORROWERS_OPEN')
    identity = inode(os.fstat(owner.lock_fd))[:2]
    duplicates = []
    for leaf in os.listdir('/proc/self/fd'):
        try:
            if int(leaf) != owner.lock_fd and inode(os.fstat(int(leaf)))[:2] == identity:
                duplicates.append(leaf)
        except OSError:
            pass
    require(not duplicates, 'SOURCE_LOCK_DUPLICATE')


def reacquire(owner, before):
    while True:
        tick(owner)
        require(inode(os.fstat(owner.lock_fd)) == before['lock'], 'SOURCE_LOCK_CHANGED')
        try:
            fcntl.flock(owner.lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            break
        except BlockingIOError:
            time.sleep(tick(owner, 0.05))
    after = owner_snapshot(owner)
    require(after['floor'] >= before['floor'] and {k: v for k, v in before.items() if k != 'floor'}
        == {k: v for k, v in after.items() if k != 'floor'}, 'SOURCE_OWNER_CHANGED')


def install(owner, binding_path, policy_root):
    owned(owner, prepared=True)
    owner_snapshot(owner)
    root, unit, drop, policy, binding = paths(owner)
    # Same-candidate foreign units are never adopted, even byte-identical.
    host_units(owner)
    absent(unit)
    absent(drop)
    require(unit_state(owner, absent_ok=True)['LoadState'] == 'not-found', 'SOURCE_FOREIGN_UNIT')
    provider_idle(owner)
    absent(root)
    b, raw = binding_owner(owner, binding_path)
    policy_raw = policy_bytes(owner, policy_root)
    binding_original = {'path': str(binding_path), 'original': original(binding_path, 0o600),
        'parent': original(binding_path.parent, stat.S_IMODE(os.stat(binding_path.parent).st_mode), directory=True)}
    root.mkdir(mode=0o700)
    policy.mkdir(mode=0o700)
    (policy / 'service-status').mkdir(mode=0o700)
    write_new(policy / 'service-status/policy.json', policy_raw)
    write_new(binding, raw)
    receipt = dict(schema='private-genuine-source-v1', commit=owner.commit, root=list(owner.root_id),
        binding_original=binding_original, assets={}, phase='INSTALLING', process=None, attempt=None)
    save(owner, receipt)  # Record attempted custody before any systemd mutation.
    argv = ['/bin/bash', str(owner.release / 'scripts/ops/install-kis-daily.sh'), '--install-disabled',
        '--raw-source', '--action-supply', '--witness-binding', str(binding), '--release-root', str(owner.release),
        '--code-commit', owner.commit, '--production-env-file', b['production_env_file'],
        '--calendar-dir', b['calendar_dir'], '--state-dir', b['state_dir'], '--lock-file', b['lock_file'],
        '--systemd-dir', str(SYSTEMD)]
    command(owner, argv)
    require(evidence().read_file(unit, LIMIT, exact=(0, 0, 0o644)) ==
        evidence().render_service(b, str(binding), b['state_dir'], b['lock_file']).encode(), 'SOURCE_UNIT_CHANGED')
    absent(drop)
    drop.mkdir(mode=0o755)
    os.chmod(drop, 0o755)
    write_new(drop / 'private-policy.conf', drop_body(policy), 0o644)
    ctl(owner, 'daemon-reload')
    receipt.update(assets=assets(owner), phase='INSTALLED')
    save(owner, receipt)
    load_receipt(owner)
    stopped(owner, unit_state(owner))
    host_units(owner, [unit_name(owner)])
    return {'phase': 'source-service', 'action': 'install', 'status': 'INSTALLED_DISABLED', 'archive_acceptance': False}


def run_source_service(owner, binding_path, policy_root, action):
    """Bounded command; service owns its own multi-day namespace and clock."""
    require(action in ('install', 'start', 'status', 'stop'), 'SOURCE_ACTION')
    owned(owner, prepared=True)
    if action == 'install':
        return install(owner, Path(binding_path), policy_root)
    owner_snapshot(owner)
    r, b = load_receipt(owner)
    require(r['phase'] not in ('RETIRING', 'RETIRED'), 'SOURCE_RETIRED')
    require(Path(binding_path) == Path(r['binding_original']['path']), 'SOURCE_BINDING_CHANGED')
    host_units(owner, [unit_name(owner)])
    state = unit_state(owner)
    if action == 'status':
        if state['ActiveState'] == 'active':
            p = attempt_process(owner, r, state)
            if r['phase'] == 'STARTING':
                require(arm_observation(owner, b, p) is not None, 'SOURCE_STARTUP_UNKNOWN')
                r.update(process=p, phase='ARMED')
                save(owner, r)
            require(r['phase'] == 'ARMED', 'SOURCE_CUSTODY_UNKNOWN')
            require(current_observation(owner, b, p) is not None, 'SOURCE_CURRENT_UNKNOWN')
            status = 'RUNNING'
        else:
            stopped(owner, state)
            r.update(phase='STOPPED', process=None)
            save(owner, r)
            status = 'STOPPED'
        return {'phase': 'source-service', 'action': action, 'status': status, 'archive_acceptance': False}
    if action == 'start':
        stopped(owner, state)
        require(r['phase'] in ('INSTALLED', 'STOPPED') and r['process'] is None, 'SOURCE_CUSTODY_UNKNOWN')
        provider_idle(owner)
        r['attempt'] = {'boot': Path('/proc/sys/kernel/random/boot_id').read_text().strip(),
                        'monotonic_ns': time.monotonic_ns()}
    elif state['ActiveState'] == 'active':
        r['process'] = attempt_process(owner, r, state)
    else:
        stopped(owner, state)
    before = owner_snapshot(owner)
    no_borrowers(owner)
    r['phase'] = 'STARTING' if action == 'start' else 'STOPPING'
    save(owner, r)
    # Same original OFD remains open but unlocked. A dup would share the lock
    # and prevent the real Source lifecycle barrier from acquiring its own OFD.
    fcntl.flock(owner.lock_fd, fcntl.LOCK_UN)
    try:
        ctl(owner, '--no-block', action, unit_name(owner))
        while True:
            tick(owner)
            load_receipt(owner)
            # During a pending job, show intentionally refuses; retry boundedly
            # via explicit Job observation, without treating it as stopped.
            try:
                state = unit_state(owner)
            except SourceCustodyUnknown as error:
                if str(error) != 'SOURCE_UNIT_UNKNOWN':
                    raise
                time.sleep(tick(owner, 0.05))
                continue
            if state['ActiveState'] in ('inactive', 'failed'):
                stopped(owner, state)
                r.update(phase='STOPPED', process=None)
                break
            if action == 'start' and state['ActiveState'] == 'active':
                p = process_state(owner, state)
                arm = arm_observation(owner, b, p)
                if arm is not None:
                    r.update(phase='ARMED', process=p)
                    break
            time.sleep(tick(owner, 0.1))
        reacquire(owner, before)
        load_receipt(owner)
        if r['phase'] == 'ARMED':
            require(process_state(owner, unit_state(owner)) == r['process'], 'SOURCE_PROCESS_CHANGED')
        else:
            stopped(owner, unit_state(owner))
        save(owner, r)
    except BaseException:
        # Only proved ARM/barrier completion or stopped state permits restoring
        # the lock. Never dispose an active unknown service after a CLI error.
        owner.original_custody_uncertain = True
        raise
    if action == 'start':
        require(r['phase'] == 'ARMED', 'SOURCE_START_FAILED')
    return {'phase': 'source-service', 'action': action, 'status': r['phase'], 'archive_acceptance': False}


class Custody:
    """Stopped launch originals, then terminal publisher locks.

    C owns its existing publication locks during archive operations. This guard
    never injects locks into C or replaces its DrainedHandle. The unit remains
    discoverable for C's actual binding collector. Terminal disposal acquires
    its own locks after C has closed, then removes the exact stopped launch path.
    """
    def __init__(self, owner, *, acceptance=False):
        self.owner, self.stack = owner, contextlib.ExitStack()
        self.binding = self.receipt = None
        self.acceptance, self.sealed, self.entered = acceptance, False, False
        self.lock_pins = []

    def check(self):
        owned(self.owner)
        for path, fd, identity in self.lock_pins:
            require(inode(os.fstat(fd)) == identity, 'SOURCE_LOCK_CHANGED')
            with evidence().opened(path, write=True, exact=(0, 0, 0o600)) as named:
                require(inode(os.fstat(named)) == identity, 'SOURCE_LOCK_CHANGED')
        _, unit, drop, _, _ = paths(self.owner)
        if self.receipt is not None:
            r, b = load_receipt(self.owner)
            require(r == self.receipt and b == self.binding, 'SOURCE_ASSETS_CHANGED')
        retired = self.receipt is not None and self.receipt['phase'] in ('RETIRING', 'RETIRED')
        if self.receipt is not None and not self.sealed and not retired:
            host_units(self.owner, [unit_name(self.owner)])
            stopped(self.owner, unit_state(self.owner))
        else:
            host_units(self.owner)
            absent(unit)
            absent(drop)
            require(unit_state(self.owner, absent_ok=True)['LoadState'] == 'not-found')
        if self.binding is not None:
            quiescent(self.owner)
            e = evidence()
            current = e.decode(e.read_file(Path(self.binding['evidence_root']) / 'current' / e.CURRENT_LEAF,
                LIMIT, exact=(0, 10001, 0o440)))
            e.validate_current(current)
            require(current['status'] == 'INVALID' and current['binding'] == self.binding, 'SOURCE_CURRENT_UNKNOWN')

    def publisher_locks(self, stack):
        if self.binding is None:
            return
        b, e = self.binding, evidence()
        for path in (Path(b['lock_file']), Path(b['evidence_root']) / '.control/lock'):
            fd = stack.enter_context(e.opened(path, write=True, exact=(0, 0, 0o600)))
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            pin = (path, fd, inode(os.fstat(fd)))
            self.lock_pins.append(pin)
            stack.callback(self.lock_pins.remove, pin)
        current = Path(b['evidence_root']) / 'current' / e.CURRENT_LEAF
        # Short reader closes before root removal; lock FDs remain held until
        # FixtureOwner's post-DB close_custody callback.
        with e.opened(current, exact=(0, 10001, 0o440)) as fd:
            require(sys.platform == 'linux' and e.ctypes.sizeof(e.OfdFlock) == 32, 'SOURCE_CURRENT_UNKNOWN')
            probe = e.OfdFlock(fcntl.F_WRLCK, os.SEEK_SET, 0, 0, 0)
            # Linux asm-generic/fcntl.h; like Source's F_OFD_SETLK fallback,
            # the real syscall fails closed on unsupported kernel/filesystem.
            require(e.OfdFlock.from_buffer_copy(fcntl.fcntl(fd, getattr(fcntl, 'F_OFD_GETLK', 36), bytes(probe))).type == fcntl.F_UNLCK,
                    'SOURCE_CURRENT_UNKNOWN')

    def seal_for_disposal(self):
        require(self.entered, 'SOURCE_CUSTODY_UNKNOWN')
        self.check()
        if self.sealed:
            return
        self.publisher_locks(self.stack)
        self.check()
        if self.receipt is not None:
            _, unit, drop, _, _ = paths(self.owner)
            # Forget only this proved stopped unit's failed state. Otherwise a
            # removed failed unit can stay loaded as not-found/failed forever.
            ctl(self.owner, 'reset-failed', unit_name(self.owner))
            if self.receipt['phase'] not in ('RETIRING', 'RETIRED'):
                # Preserve the retirement intent before touching launch originals.
                self.receipt['phase'] = 'RETIRING'
                save(self.owner, self.receipt)
                # Never stop a service as an implicit cleanup side effect.
                os.unlink(drop / 'private-policy.conf')
                os.rmdir(drop)
                os.unlink(unit)
            ctl(self.owner, 'daemon-reload')
            self.receipt['phase'] = 'RETIRED'
            save(self.owner, self.receipt)
        self.sealed = True
        self.check()

    def __enter__(self):
        owner = self.owner
        try:
            owned(owner)
            require(not owner.original_custody_uncertain, 'SOURCE_CUSTODY_UNKNOWN')
            root, _, _, _, _ = paths(owner)
            if os.path.lexists(root):
                self.receipt, self.binding = load_receipt(owner)
            self.check()
            self.entered = True
            if self.acceptance:
                # Only the real C implementation may own locks during C's
                # native/SQL archive continuation; do not race its separate OFD.
                with contextlib.ExitStack() as initial:
                    self.publisher_locks(initial)
                    self.check()
            else:
                self.seal_for_disposal()
            return self
        except BaseException:
            self.stack.close()
            raise

    def __exit__(self, *exc):
        try:
            if self.entered and not self.sealed:
                self.check()
        finally:
            self.entered = False
            self.stack.__exit__(*exc)


def require_source_custody_for_acceptance(owner):
    return Custody(owner, acceptance=True)


def dispose_with_source_custody(owner, custody=None):
    if custody is None:
        with Custody(owner) as held:
            return dispose_with_source_custody(owner, held)
    require(type(custody) is Custody and custody.owner is owner and custody.entered, 'SOURCE_CUSTODY_UNKNOWN')
    custody.seal_for_disposal()
    custody.check()  # Independent fresh proof immediately before DB/root teardown.
    owner.dispose(custody.stack.close)
