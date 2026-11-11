#!/usr/bin/env python3
"""Private native4 implementation fixture; never official archive acceptance.

Bootstrap, install, one real bounded future session per invocation, and accept
retain actual originals and refuse unknown cleanup. Runtime requires its explicit
private inputs and isolated venue; defining these phases does not activate them.
"""
import argparse
import importlib.util
from pathlib import Path
import os
import shutil
import sys
import tempfile

sys.dont_write_bytecode = True


def module():
    name = 'owner_beta_paper_native_source'
    path = Path(__file__).with_name('lib') / 'owner-beta-paper-native-source.py'
    if name not in sys.modules:
        spec = importlib.util.spec_from_file_location(name, path)
        result = importlib.util.module_from_spec(spec)
        sys.modules[name] = result
        spec.loader.exec_module(result)
    result = sys.modules[name]
    if Path(result.__file__).resolve() != path.resolve():
        raise RuntimeError('NATIVE_MODULE')
    return result


def parser():
    p = argparse.ArgumentParser(allow_abbrev=False, description=__doc__)
    for name in ('source-root', 'commit', 'manifest-file', 'approved-base', 'raw-root', 'daily-config',
                 'generic-bundle', 'generic-session', 'source-inputs', 'checkpoint', 'test-elf', 'fixture-root'):
        p.add_argument('--' + name)
    p.add_argument('--deadline-monotonic-ns', type=int)
    p.add_argument('phase', choices=('bootstrap', 'install', 'session', 'accept', 'dispose', 'service-run'))
    return p


def main(argv=None):
    args = parser().parse_args(argv)
    native = module()
    h = native.history()
    f = h.fixture_module()
    policy = None
    policy_scope = None
    failed = False
    result = None
    try:
        native.require(sys.platform == 'linux' and os.geteuid() == 0 and os.getegid() == 0,
                       'NATIVE_PRIVATE_ROOT_REQUIRED')
        if args.phase == 'service-run':
            native.require(args.fixture_root is not None and all(getattr(args, name) is None for name in
                ('source_root','commit','manifest_file','approved_base','raw_root','daily_config','generic_bundle',
                 'generic_session','source_inputs','checkpoint','test_elf','deadline_monotonic_ns')),
                 'NATIVE_SERVICE_ARGUMENTS')
            result = native.service_run(args.fixture_root)
        else:
            native.require(all(getattr(args, name) is not None for name in
                ('source_root','commit','manifest_file','deadline_monotonic_ns')) and args.fixture_root is None,
                'NATIVE_PHASE_ARGUMENTS')
            h.phase_deadline(args.deadline_monotonic_ns)
            source, manifest, deadline = f.validated_inputs(args.source_root, args.commit,
                args.manifest_file, args.deadline_monotonic_ns)
            if args.phase == 'bootstrap':
                native.require(all(getattr(args,name) is not None for name in
                    ('approved_base','raw_root','daily_config','generic_bundle','generic_session','source_inputs'))
                    and args.checkpoint is None and args.test_elf is None, 'NATIVE_BOOTSTRAP_ARGUMENTS')
                policy = Path(tempfile.mkdtemp(prefix='lagrange-paper-genuine-policy-', dir='/var/tmp'))
                (policy / 'service-status').mkdir(mode=0o700)
                policy_scope = native.NativePolicy(policy)
                with f.termination_cleanup(), f.bounded_run(deadline), policy_scope:
                    result = native.bootstrap(source, args.commit, manifest, deadline,
                        approved_base=args.approved_base, raw_root=args.raw_root, config_source=args.daily_config,
                        generic_bundle=args.generic_bundle, generic_session=args.generic_session, policy_root=policy,
                        source_inputs=args.source_inputs)
            else:
                native.require(args.checkpoint is not None and all(getattr(args,name) is None for name in
                    ('approved_base','raw_root','daily_config','generic_bundle','generic_session','source_inputs'))
                    and (args.test_elf is not None) == (args.phase == 'install'), 'NATIVE_PHASE_ARGUMENTS')
                result = native.continue_phase(source, args.commit, manifest, deadline, args.checkpoint,
                    args.phase, elf_path=args.test_elf)
        native.require(result['archive_acceptance'] is False, 'NATIVE_RESULT')
    except (Exception, KeyboardInterrupt):
        failed = True
    finally:
        if policy is not None and (policy_scope is None
                or policy_scope not in f._UNRESOLVED_POLICY_SCOPES):
            try:
                shutil.rmtree(policy)
            except (Exception, KeyboardInterrupt):
                failed = True
    if failed:
        print('PAPER_NATIVE_SOURCE: PHASE_UNPROVEN')
        return 1
    print(h.canonical(result).decode('ascii'))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
