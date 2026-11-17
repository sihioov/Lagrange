#!/usr/bin/env python3
"""Drain the Owner V2 reader for an image build, and restore it afterwards.

The drained-readers build route (``release-build-layout.sh``) requires the
container ``lagrange-station-owner-equity-v2-runner-1`` to be exited with exit
code 0 at every build gate.  No checked-in command produced that state:
``owner-beta-daily-runtime.py drain`` is closed over the three Daily services.
This tool performs the smallest graceful drain of the one exact V2 generation:
record it, ``docker update --restart=no``, ``docker kill --signal=TERM``,
bounded ``docker wait``, and require exit 0.  It never sends SIGKILL, never runs
``docker stop``/``rm`` and never retries a failed drain.  ``--restore`` is a
separate explicit step: it restores the recorded restart policy on the exact
recorded generation, starts that same container and waits for healthy.

Ownership, manifest and generation checks are the installed application
release's own code.  Output is value-free; the root 0600 progress record in the
explicit ``--state-dir`` holds the identifiers.
"""
from __future__ import annotations

import importlib.util
import os
import sys
import time
from pathlib import Path

sys.dont_write_bytecode = True


def _load_helpers():
    path = Path(__file__).resolve().with_name("lib") / "reader-generation.py"
    spec = importlib.util.spec_from_file_location("reader_generation", path)
    module = importlib.util.module_from_spec(spec)
    sys.modules["reader_generation"] = module
    spec.loader.exec_module(module)
    return module


rg = _load_helpers()
require, ToolError = rg.require, rg.ToolError

PREFIX = "READER_DRAIN"
SCHEMA = "lagrange-build-reader-drain-progress-v1"
SERVICE = "owner-equity-v2-runner"
PROJECT = "lagrange-station"  # the only project release-build-layout.sh attests
CONTAINER_NAME = "/" + PROJECT + "-" + SERVICE + "-1"
PROGRESS_NAME = "build-reader-drain"
# Compose stop_grace_period for this service is 16m (work timeout 15m); the
# bounded wait must not end before Docker's own graceful window would.
DRAIN_WAIT_SECONDS = 1020
RESTORE_HEALTH_SECONDS = 300
POLL_SECONDS = 2.0


def parse(argv):
    parser = rg.Parser(prog="build-reader-drain.py", code="READER_DRAIN_ARGUMENTS",
                       description="Drain or restore the exact Owner V2 reader generation.")
    modes = parser.add_mutually_exclusive_group(required=True)
    for mode in ("plan", "preflight", "apply", "restore"):
        modes.add_argument("--" + mode, dest="mode", action="store_const", const=mode)
    parser.add_argument("--install-root", required=True)
    parser.add_argument("--application-commit", required=True)
    parser.add_argument("--reader-from-commit", required=True)
    parser.add_argument("--state-dir", required=True)
    rg.reject_repeated(argv, ("--install-root", "--application-commit", "--reader-from-commit",
                              "--state-dir", "--plan", "--preflight", "--apply", "--restore"),
                       "READER_DRAIN_ARGUMENTS")
    args = parser.parse_args(argv)
    require(rg.absolute(args.install_root) and rg.absolute(args.state_dir)
            and rg.is_commit(args.application_commit) and rg.is_commit(args.reader_from_commit),
            "READER_DRAIN_ARGUMENTS")
    return args


def arguments_record(args):
    return {"install_root": args.install_root, "application_commit": args.application_commit,
            "reader_from_commit": args.reader_from_commit}


class Drain:
    def __init__(self, args, host, *, clock=time.monotonic, sleep=time.sleep):
        self.args, self.host = args, host
        self.clock, self.sleep = clock, sleep
        self.docker = host.docker

    # -- observation -----------------------------------------------------
    def bind(self):
        try:
            self.host.validate_current()
        except Exception as exc:
            raise ToolError("READER_DRAIN_CURRENT", rg.cause_of(exc)) from None
        try:
            project = self.host.project()
            images = self.host.manifest_images(self.args.reader_from_commit)
            require(project == PROJECT and isinstance(images, dict)
                    and isinstance(images.get(SERVICE), str), "READER_DRAIN_OWNER")
            return self.host.ownership(project, self.args.reader_from_commit, images[SERVICE])
        except ToolError:
            raise
        except Exception as exc:
            raise ToolError("READER_DRAIN_OWNER", rg.cause_of(exc)) from None

    def inspect(self, own, identity, code):
        try:
            value = self.docker.object("container", identity, 15)
            generation = self.host.owned(value, own, SERVICE, identity)
        except Exception as exc:
            raise ToolError(code, rg.cause_of(exc)) from None
        require(value.get("Name") == CONTAINER_NAME, code)
        return value, list(generation)

    def single(self, own, code):
        ids = rg.compose_ids(self.docker, own.project, SERVICE, code)
        require(len(ids) == 1, code)
        return ids[0]

    def observe_running(self):
        own = self.bind()
        identity = self.single(own, "READER_DRAIN_OWNER")
        value, generation = self.inspect(own, identity, "READER_DRAIN_OWNER")
        require(rg.running_healthy(value), "READER_DRAIN_STATE")
        policy = rg.restart_policy(value, "READER_DRAIN_POLICY")
        reader = {"identity": generation[0], "created": generation[1], "started_at": generation[2],
                  "restart_count": generation[3], "image": own.image_id, "restart_policy": policy}
        return own, reader

    def same(self, own, reader, code):
        require(rg.compose_ids(self.docker, own.project, SERVICE, code) == [reader["identity"]], code)
        value, generation = self.inspect(own, reader["identity"], code)
        expected = [reader["identity"], reader["created"], reader["started_at"], reader["restart_count"]]
        require(generation == expected, code)
        return value, generation

    # -- apply -----------------------------------------------------------
    def drain(self, progress):
        own, reader = self.observe_running()
        record = progress.create({"schema": SCHEMA, "arguments": arguments_record(self.args),
                                  "phase": "draining", "code": None, "signalled": False,
                                  "reader": reader})
        try:
            identity = reader["identity"]
            try:
                self.docker.call(["update", "--restart=no", identity], 15)
            except Exception as exc:
                raise ToolError("READER_DRAIN_POLICY", rg.cause_of(exc)) from None
            value, _ = self.same(own, reader, "READER_DRAIN_CHANGED")
            require(value["HostConfig"].get("RestartPolicy") == rg.STOPPED_POLICY
                    and value["State"].get("Running") is True, "READER_DRAIN_POLICY")
            # Journal the signal before sending it: a crash after this point
            # must never be read as "TERM was not sent".
            record = progress.replace(record, dict(record, signalled=True))
            try:
                self.docker.call(["kill", "--signal=TERM", identity], 15)
            except Exception as exc:
                raise ToolError("READER_DRAIN_SIGNAL", rg.cause_of(exc)) from None
            try:
                rows = self.docker.call(["wait", identity], DRAIN_WAIT_SECONDS).splitlines()
            except Exception as exc:
                raise ToolError("READER_DRAIN_EXIT", rg.cause_of(exc)) from None
            require(rows == ["0"], "READER_DRAIN_EXIT")
            value, _ = self.same(own, reader, "READER_DRAIN_CHANGED")
            require(rg.exited_zero(value)
                    and value["HostConfig"].get("RestartPolicy") == rg.STOPPED_POLICY,
                    "READER_DRAIN_EXIT")
        except ToolError as exc:
            progress.replace(record, dict(record, phase="drain-failed", code=exc.code))
            raise
        except Exception:
            progress.replace(record, dict(record, phase="drain-failed", code="READER_DRAIN_INTERNAL"))
            raise ToolError("READER_DRAIN_INTERNAL") from None
        progress.replace(record, dict(record, phase="drained"))
        return "DRAINED"

    # -- restore ---------------------------------------------------------
    def restore(self, progress):
        record = progress.read()
        require(record is not None and record.get("schema") == SCHEMA
                and record.get("arguments") == arguments_record(self.args)
                and record.get("phase") in ("draining", "drained", "drain-failed", "restoring"),
                "READER_RESTORE_PROGRESS")
        reader = record["reader"]
        own = self.bind()
        require(own.image_id == reader["image"], "READER_RESTORE_CHANGED")
        recorded = [reader["identity"], reader["created"], reader["started_at"], reader["restart_count"]]

        def observe():
            code = "READER_RESTORE_CHANGED"
            require(rg.compose_ids(self.docker, own.project, SERVICE, code) == [reader["identity"]], code)
            return self.inspect(own, reader["identity"], code)

        observe()
        if record["phase"] != "restoring":
            record = progress.replace(record, dict(record, phase="restoring", code=None))
        signalled = record["signalled"]
        # A TERM may have been sent to a still-running original start: never restart it.
        rg.restore_generation(
            self.docker, observe, recorded, reader["restart_policy"], signalled=lambda _: signalled,
            clock=self.clock, sleep=self.sleep, health_seconds=RESTORE_HEALTH_SECONDS,
            poll_seconds=POLL_SECONDS,
            codes={"changed": "READER_RESTORE_CHANGED", "state": "READER_RESTORE_STATE",
                   "policy": "READER_RESTORE_POLICY", "start": "READER_RESTORE_START",
                   "health": "READER_RESTORE_HEALTH"})
        progress.replace(record, dict(record, phase="restored"))
        return "RESTORED"


def previous_cycle(progress):
    """None, or a finished (restored) record that a new cycle may move aside."""
    record = progress.read()
    if record is None:
        return None
    require(record.get("schema") == SCHEMA and record.get("phase") == "restored", "READER_DRAIN_PROGRESS")
    return record


PLAN_LINES = (
    "READER_DRAIN: PLAN",
    "  apply: bind the single owner-equity-v2-runner container to the reader commit's installed manifest;",
    "         require running and healthy; journal generation and restart policy;",
    "         docker update --restart=no; docker kill --signal=TERM; bounded docker wait; require exit 0",
    "  a failed drain is never retried; --restore is a separate explicit step",
    "  a new cycle in the same state dir is allowed only after RESTORED (the old record is moved aside)",
    "  restore: recorded policy on the exact recorded generation; docker start; bounded wait for healthy",
    "PLAN_ONLY: no Docker call, no state write",
)


def main(argv=None, *, host_factory=None, uid=None, out=None, err=None,
         clock=time.monotonic, sleep=time.sleep):
    out = sys.stdout if out is None else out
    err = sys.stderr if err is None else err
    argv = list(sys.argv[1:] if argv is None else argv)
    progress = None
    try:
        args = parse(argv)
        if args.mode == "plan":
            for line in PLAN_LINES:
                print(line, file=out)
            return 0
        try:
            if host_factory is None:
                uid = 0
                host = rg.ReleaseHost(args.install_root, args.application_commit, "READER_DRAIN_ROOT")
            else:
                uid = os.geteuid() if uid is None else uid
                host = host_factory(args)
        except ToolError:
            raise
        except Exception as exc:
            raise ToolError("READER_DRAIN_ROOT", rg.cause_of(exc)) from None
        rg.state_directory(args.state_dir, uid, "READER_DRAIN_STATE_DIR")
        progress = rg.Progress(args.state_dir, PROGRESS_NAME, uid, "READER_DRAIN_PROGRESS")
        drain = Drain(args, host, clock=clock, sleep=sleep)
        if args.mode == "preflight":
            previous_cycle(progress)
            drain.observe_running()
            print("READER_DRAIN: PREFLIGHT", file=out)
            return 0
        progress.lock()
        if args.mode == "apply":
            previous = previous_cycle(progress)
            if previous is not None:
                # Only a fully restored cycle is moved aside (no-clobber); any
                # other phase still blocks a new drain.
                drain.observe_running()
                progress.retire(previous, rg.utc_stamp())
            result = drain.drain(progress)
        else:
            result = drain.restore(progress)
        print("READER_DRAIN: " + result, file=out)
        return 0
    except ToolError as exc:
        print("READER_DRAIN: FAILED " + exc.code, file=err)
        if exc.cause:
            print("READER_DRAIN: CAUSE " + exc.cause, file=err)
        return 1
    except Exception:
        print("READER_DRAIN: FAILED READER_DRAIN_INTERNAL", file=err)
        return 1
    finally:
        if progress is not None:
            progress.unlock()


if __name__ == "__main__":
    raise SystemExit(main())
