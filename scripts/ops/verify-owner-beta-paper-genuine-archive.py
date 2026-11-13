#!/usr/bin/env python3
"""Private, explicit-input genuine Paper history preparation/acceptance CLI.

Not a hosted G2 case, Source activation service, readiness override or Owner
acceptance. The explicit source-service phase controls the existing authorized
immutable-release service in its owned private policy namespace. Production remains CLOSED.
"""
from __future__ import annotations

import argparse
import importlib.util
from pathlib import Path
import sys

sys.dont_write_bytecode = True


def history_module():
    name = "owner_beta_paper_genuine_history"
    path = Path(__file__).with_name("lib") / "owner-beta-paper-genuine-history.py"
    module = sys.modules.get(name)
    if module is None:
        spec = importlib.util.spec_from_file_location(name, path)
        module = importlib.util.module_from_spec(spec)
        sys.modules[name] = module
        spec.loader.exec_module(module)
    if Path(module.__file__).resolve() != path.resolve():
        raise RuntimeError("HISTORY_SOURCE")
    return module


def parser():
    root = argparse.ArgumentParser(allow_abbrev=False, description=__doc__)
    root.add_argument("--source-root", required=True)
    root.add_argument("--commit", required=True)
    root.add_argument("--manifest-file", required=True)
    root.add_argument("--deadline-monotonic-ns", required=True, type=int,
        help="Inherited absolute monotonic_ns cutoff, at most 90 minutes; never extended.")
    phases = root.add_subparsers(dest="phase", required=True)
    bootstrap = phases.add_parser("bootstrap", allow_abbrev=False,
        help="New disposable OFF-reader release, native configure and fixture HTTP consent.")
    for name in ("approved-base", "raw-root", "daily-config", "generic-bundle", "generic-session"):
        bootstrap.add_argument("--" + name, required=True)
    for name in ("observe-reset", "enable", "recover", "readback", "produce", "source-service", "accept"):
        phase = phases.add_parser(name, allow_abbrev=False)
        phase.add_argument("--checkpoint", required=True,
            help="Protected bootstrap locator; all live originals are independently rechecked.")
        phase.set_defaults(caller_key=None, generation=None, sequence=0)
        if name in {"enable", "recover"}:
            phase.add_argument("--caller-key", required=True)
        if name == "recover":
            phase.add_argument("--proposal-file", required=True)
        if name == "readback":
            phase.add_argument("--generation", required=True)
            phase.add_argument("--sequence", type=int, default=0)
        if name == "produce":
            phase.add_argument("--producer", required=True,
                choices=("daily", "beta-preview", "paper-cycle", "simulation", "expire-claim"))
            for member in ("approved-base", "raw-root", "daily-config"):
                phase.add_argument("--" + member, required=True)
        if name == "accept":
            phase.add_argument("--storage-config", required=True)
        if name == "source-service":
            phase.add_argument("--action", choices=("install", "start", "status", "stop"), required=True)
            phase.add_argument("--binding-file", required=True,
                help="Protected original witness binding for the later published prepared release.")
    return root


def main(argv=None):
    args = parser().parse_args(argv)
    history = history_module()
    try:
        with history.fixture_module().termination_cleanup():
            result = history.run(args)
        # A preparatory phase never turns into whole-lifecycle PASS merely by
        # returning. The acceptance consumer additionally checks every C case.
        if args.phase == "accept":
            history.require(result.get("status") == "PASS" and result.get("archive_acceptance") is True
                and result.get("cases") and all(case.get("status") == "PASS" for case in result["cases"]),
                "HISTORY_ACCEPTANCE_UNPROVEN")
    except (Exception, KeyboardInterrupt):
        print("PAPER_GENUINE_HISTORY_FAILED:PHASE_UNPROVEN")
        return 1
    print(history.canonical(result).decode("ascii"))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
