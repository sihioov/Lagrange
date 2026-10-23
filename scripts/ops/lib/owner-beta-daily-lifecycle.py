"""Bounded daemon-event evidence for one disposable wholeflow nonce.

No Docker mutations or global event/log capture. Container labels and network
names require separate streams: Docker network events do not carry labels.
Process creation is not subscription readiness; the driver must await each
owned create event before starting processes. No stream reconnect/replay.
"""
from __future__ import annotations

import hashlib
import json
import os
import re
import selectors
import subprocess
import threading
import time

LABEL = "io.lagrange.daily-wholeflow"
SERVICE = LABEL + "-service"
ROLES = {"postgres", "bootstrap", "migrate", "control", "api", "producer", "admitter", "worker"}
HEX = re.compile(r"[a-f0-9]{64}")
NONCE = re.compile(r"lagrange-wholeflow-[a-f0-9]{32}")
MAX_LINE = 16384
MAX_WIRE_BYTES = 1024 * 1024
MAX_EVENTS = 512
ENV = {"PATH": "/usr/sbin:/usr/bin:/sbin:/bin", "LANG": "C.UTF-8", "LC_ALL": "C.UTF-8"}


class EvidenceError(Exception):
    pass


def require(condition, code="WHOLEFLOW_EVENTS_INVALID"):
    if not condition:
        raise EvidenceError(code)


def unique(pairs):
    value = {}
    for key, item in pairs:
        require(key not in value)
        value[key] = item
    return value


def sanitize(raw, nonce, kind):
    """Never persist arbitrary attributes, images, names or free-form errors."""
    try:
        value = json.loads(raw, object_pairs_hook=unique)
        require(isinstance(value, dict) and value.get("Type") == kind and value.get("scope") == "local")
        action = value.get("Action")
        require(action in ({"create", "start", "die", "destroy", "oom"} if kind == "container"
                           else {"create", "destroy"}))
        actor = value["Actor"]
        identity, attrs = actor["ID"], actor["Attributes"]
        require(isinstance(identity, str) and HEX.fullmatch(identity) and isinstance(attrs, dict))
        name = attrs.get("name")
        stamp = value.get("timeNano")
        require(type(stamp) is int and 0 < stamp < 10**20)
        row = {"kind": kind, "id": identity, "name": name, "action": action, "timeNano": stamp}
        if kind == "network":
            require(name in {nonce + "-initial", nonce + "-recovered"})
        else:
            role, image = attrs.get(SERVICE), attrs.get("image")
            require(isinstance(role, str) and role in ROLES and attrs.get(LABEL) == nonce)
            require(isinstance(name, str) and re.fullmatch(re.escape(nonce + "-" + role) + r"-[1-9][0-9]{0,2}", name))
            require(isinstance(image, str) and re.fullmatch(r"sha256:[a-f0-9]{64}", image))
            row.update(role=role, image=image)
            if action == "die":
                code = attrs.get("exitCode")
                require(isinstance(code, str) and re.fullmatch(r"0|[1-9][0-9]{0,2}", code) and int(code) <= 255)
                row["exitCode"] = int(code)
        return row
    except (KeyError, TypeError, ValueError, UnicodeError, RecursionError):
        raise EvidenceError("WHOLEFLOW_EVENTS_INVALID") from None


def validate(rows, objects):
    require(objects and len(objects) <= 128, "WHOLEFLOW_EVENTS_OBJECTS")
    expected = {(obj["kind"], obj["id"]): obj for obj in objects}
    require(len(expected) == len(objects), "WHOLEFLOW_EVENTS_OBJECTS")
    grouped = {}
    for row in rows:
        key = (row["kind"], row["id"])
        require(key in expected, "WHOLEFLOW_EVENTS_FOREIGN")
        obj = expected[key]
        require(row["name"] == obj["name"] and (row["kind"] == "network" or
                (row["image"] == obj["image"] and row["role"] == obj["role"])), "WHOLEFLOW_EVENTS_BINDING")
        grouped.setdefault(key, []).append(row)
    require(set(grouped) == set(expected), "WHOLEFLOW_EVENTS_INCOMPLETE")
    for key, events in grouped.items():
        actions = [event["action"] for event in events]
        require(actions == (["create", "start", "die", "destroy"] if key[0] == "container"
                            else ["create", "destroy"]), "WHOLEFLOW_EVENTS_INCOMPLETE")
        require(all(a["timeNano"] <= b["timeNano"] for a, b in zip(events, events[1:])),
                "WHOLEFLOW_EVENTS_ORDER")


class Recorder:
    def __init__(self, nonce, fd, timeout_seconds):
        require(isinstance(nonce, str) and NONCE.fullmatch(nonce), "WHOLEFLOW_EVENTS_NONCE")
        require(1 <= timeout_seconds <= 2100, "WHOLEFLOW_EVENTS_BOUND")
        self.nonce, self.fd = nonce, fd
        self.end = time.monotonic() + timeout_seconds
        stamp = time.time_ns()
        self.since = f"{stamp // 10**9}.{stamp % 10**9:09d}"
        self.until = str(int(time.time()) + timeout_seconds)
        self.condition = threading.Condition()
        self.stopping = threading.Event()
        self.rows, self.processes, self.threads = [], [], []
        self.error = None
        self.wire_bytes = self.saved_bytes = 0
        self.digest = hashlib.sha256()

    def fail(self, code):
        with self.condition:
            if self.error is None:
                self.error = code
            self.condition.notify_all()

    def check(self):
        if self.error:
            raise EvidenceError(self.error)
        require(time.monotonic() < self.end, "WHOLEFLOW_EVENTS_DEADLINE")

    def command(self, kind):
        args = ["/usr/bin/docker", "--host", "unix:///var/run/docker.sock", "events",
                "--since", self.since, "--until", self.until, "--format", "{{json .}}",
                "--filter", "type=" + kind, "--filter", "scope=local"]
        if kind == "container":
            args += ["--filter", "label=" + LABEL + "=" + self.nonce]
            actions = ("create", "start", "die", "destroy", "oom")
        else:
            args += ["--filter", "network=" + self.nonce + "-initial",
                     "--filter", "network=" + self.nonce + "-recovered"]
            actions = ("create", "destroy")
        for action in actions:
            args += ["--filter", "event=" + action]
        return args

    def start(self):
        require(not self.processes, "WHOLEFLOW_EVENTS_ALREADY_STARTED")
        try:
            for kind in ("container", "network"):
                process = subprocess.Popen(self.command(kind), env=ENV, stdin=subprocess.DEVNULL,
                                           stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, bufsize=0)
                self.processes.append(process)
                thread = threading.Thread(target=self.read, args=(process, kind), daemon=True)
                self.threads.append(thread)
                thread.start()
        except OSError:
            self.fail("WHOLEFLOW_EVENTS_START_FAILED")
            self.close()
            raise EvidenceError(self.error) from None

    def read(self, process, kind):
        pending = b""
        try:
            with selectors.DefaultSelector() as selector:
                selector.register(process.stdout, selectors.EVENT_READ)
                while not self.stopping.is_set():
                    self.check()
                    if not selector.select(min(0.2, max(0, self.end - time.monotonic()))):
                        continue
                    chunk = os.read(process.stdout.fileno(), 4096)
                    if not chunk:
                        require(self.stopping.is_set(), "WHOLEFLOW_EVENTS_STREAM_LOST")
                        return
                    with self.condition:
                        self.wire_bytes += len(chunk)
                        require(self.wire_bytes <= MAX_WIRE_BYTES, "WHOLEFLOW_EVENTS_LIMIT")
                    pending += chunk
                    while b"\n" in pending:
                        raw, pending = pending.split(b"\n", 1)
                        require(len(raw) <= MAX_LINE, "WHOLEFLOW_EVENTS_LIMIT")
                        row = sanitize(raw, self.nonce, kind)
                        data = (json.dumps(row, sort_keys=True, separators=(",", ":")) + "\n").encode()
                        with self.condition:
                            require(len(self.rows) < MAX_EVENTS, "WHOLEFLOW_EVENTS_LIMIT")
                            # Complete writes and fsync precede readiness/completion notification.
                            offset = 0
                            while offset < len(data):
                                written = os.write(self.fd, data[offset:])
                                require(written > 0, "WHOLEFLOW_EVENTS_WRITE_FAILED")
                                offset += written
                            os.fsync(self.fd)
                            self.digest.update(data)
                            self.saved_bytes += len(data)
                            self.rows.append(row)
                            self.condition.notify_all()
                    require(len(pending) <= MAX_LINE, "WHOLEFLOW_EVENTS_LIMIT")
        except EvidenceError as error:
            self.fail(str(error))
        except BaseException:
            self.fail("WHOLEFLOW_EVENTS_READ_FAILED")

    def wait_for(self, kind, identity, action, timeout=10):
        deadline = min(self.end, time.monotonic() + timeout)
        with self.condition:
            while True:
                self.check()
                if any(r["kind"] == kind and r["id"] == identity and r["action"] == action for r in self.rows):
                    return
                require(time.monotonic() < deadline, "WHOLEFLOW_EVENTS_WAIT_TIMEOUT")
                self.condition.wait(min(0.1, max(0, deadline - time.monotonic())))

    def close(self):
        # Stop only the two owned, read-only docker-events clients. Never signal
        # a container, daemon, production service or another process tree.
        self.stopping.set()
        for process in self.processes:
            if process.poll() is None:
                try:
                    process.terminate()
                except ProcessLookupError:
                    pass
        for thread in self.threads:
            thread.join(timeout=2)
            if thread.is_alive():
                self.fail("WHOLEFLOW_EVENTS_READER_NOT_DRAINED")
        for process in self.processes:
            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=2)
                self.fail("WHOLEFLOW_EVENTS_CLIENT_NOT_DRAINED")
            process.stdout.close()

    def finish(self, objects):
        try:
            deadline = time.monotonic() + 10
            for obj in objects:
                self.wait_for(obj["kind"], obj["id"], "destroy", max(0, deadline - time.monotonic()))
            with self.condition:
                self.check()
                validate(self.rows, objects)
        except EvidenceError as error:
            self.fail(str(error))
        finally:
            self.close()
        if not self.error:
            try:
                # Include any already-buffered event received during client drain.
                validate(self.rows, objects)
            except EvidenceError as error:
                self.fail(str(error))
        return {"status": "INCOMPLETE" if self.error else "COMPLETE", "code": self.error,
                "events": len(self.rows), "bytes": self.saved_bytes, "sha256": self.digest.hexdigest(),
                "objects": len(objects), "wire_bytes": self.wire_bytes,
                "reconnects": 0, "processes_drained": all(p.poll() is not None for p in self.processes),
                "readers_drained": all(not t.is_alive() for t in self.threads)}
