"""OS-level process isolation for the NT backtest worker (plan Todo 20).

Windows enforces memory / CPU-time / active-process limits and kill-on-close
through a Job Object (the child is created suspended, assigned to the job, and
resumed; every limit probed green on this host). POSIX falls back to
`resource` rlimits for memory and CPU plus a dedicated process group for
termination; one-node-per-process has no unprivileged POSIX equivalent
(cgroups / the Compose container leg cover it there) so that limit is
Windows-only by design.

`spawn` additionally translates the limits into the environment contract the
child runtime guard (`guard.py`) consumes: `LAGRANGE_NETWORK_DISABLED`,
`LAGRANGE_RO_MOUNTS`, `LAGRANGE_STATUS_FILE`, `LAGRANGE_CONTROL_FILE`.
"""
from __future__ import annotations

import os
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

CREATE_SUSPENDED = 0x00000004
CREATE_NEW_PROCESS_GROUP = 0x00000200

_CTRL_BREAK_EVENT = 1


class IsolationError(RuntimeError):
    """The platform could not apply the requested isolation."""


def interpreter_path() -> str:
    """The real interpreter to spawn children with.

    Under uv on Windows the venv `python.exe` is a trampoline that launches
    the managed CPython as a CHILD process - incompatible with one-node-per-
    process (ActiveProcessLimit=1 would kill it). `sys._base_executable`
    resolves to the real interpreter; when it differs from `sys.executable`
    it is used, and the venv site-packages are added to PYTHONPATH by
    [`venv_site_packages`].
    """
    base = getattr(sys, "_base_executable", None)
    if base and os.path.normcase(base) != os.path.normcase(sys.executable):
        return base
    return sys.executable


def venv_site_packages() -> str | None:
    if os.path.normcase(interpreter_path()) == os.path.normcase(sys.executable):
        return None
    if os.name == "nt":
        return os.path.join(sys.prefix, "Lib", "site-packages")
    return os.path.join(
        sys.prefix, "lib", f"python{sys.version_info.major}.{sys.version_info.minor}", "site-packages"
    )


@dataclass(frozen=True)
class IsolationLimits:
    """Resource and runtime limits applied to one worker child process."""

    memory_bytes: int | None = None
    cpu_seconds: float | None = None
    wall_seconds: float | None = None
    active_processes: int | None = 1
    network_disabled: bool = True
    readonly_mounts: tuple[str, ...] = ()

    def child_env(self, base: dict[str, str]) -> dict[str, str]:
        env = dict(base)
        if self.network_disabled:
            env["LAGRANGE_NETWORK_DISABLED"] = "1"
        if self.readonly_mounts:
            env["LAGRANGE_RO_MOUNTS"] = os.pathsep.join(self.readonly_mounts)
        return env


@dataclass(frozen=True)
class Termination:
    """How a child ended after a terminate() request."""

    method: str
    exit_code: int | None
    graceful: bool


def _posix_limits(limits: IsolationLimits):
    def apply() -> None:
        import resource

        if limits.cpu_seconds is not None:
            resource.setrlimit(resource.RLIMIT_CPU, (int(limits.cpu_seconds), int(limits.cpu_seconds) + 1))
        if limits.memory_bytes is not None:
            soft = max(limits.memory_bytes, 64 * 1024 * 1024)
            resource.setrlimit(resource.RLIMIT_AS, (soft, soft))

    return apply


class ProcessIsolation:
    """Isolation applied to one spawned child process (one node per process)."""

    def __init__(self, limits: IsolationLimits) -> None:
        self.limits = limits
        self._windows_job = _WindowsJob(limits) if os.name == "nt" else None

    @property
    def backend(self) -> str:
        return "windows-job-object" if self._windows_job is not None else "posix-rlimit"

    def spawn(
        self,
        argv: list[str],
        *,
        cwd: str | os.PathLike | None = None,
        env: dict[str, str] | None = None,
        stdout=None,
        stderr=None,
        stdin=None,
        text: bool = False,
    ) -> subprocess.Popen:
        env = self.limits.child_env(env or os.environ.copy())
        if os.name == "nt":
            proc = subprocess.Popen(
                argv,
                cwd=cwd,
                env=env,
                stdout=stdout,
                stderr=stderr,
                stdin=stdin,
                text=text,
                creationflags=CREATE_SUSPENDED | CREATE_NEW_PROCESS_GROUP,
            )
            self._windows_job.assign(proc)
            self._windows_job.resume(proc)
        else:
            proc = subprocess.Popen(
                argv,
                cwd=cwd,
                env=env,
                stdout=stdout,
                stderr=stderr,
                stdin=stdin,
                text=text,
                start_new_session=True,
                preexec_fn=_posix_limits(self.limits),
            )
        proc._lagrange_control_file = env.get("LAGRANGE_CONTROL_FILE")
        return proc

    def wait(self, proc: subprocess.Popen, timeout: float) -> int:
        return proc.wait(timeout=timeout)

    def terminate(self, proc: subprocess.Popen, grace_seconds: float = 5.0) -> Termination:
        if self._control_file(proc) is not None:
            Path(self._control_file(proc)).write_text("STOP", encoding="utf-8")
        else:
            self._send_graceful_signal(proc)
        try:
            exit_code = proc.wait(timeout=grace_seconds)
            return Termination(method="signal", exit_code=exit_code, graceful=True)
        except subprocess.TimeoutExpired:
            self.kill(proc)
            exit_code = proc.wait()
            return Termination(method="kill", exit_code=exit_code, graceful=False)

    def kill(self, proc: subprocess.Popen) -> None:
        if self._windows_job is not None:
            self._windows_job.terminate_all()
        elif os.name == "posix":
            import signal

            try:
                os.killpg(proc.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        else:
            proc.kill()

    def close(self) -> None:
        if self._windows_job is not None:
            self._windows_job.close()

    def _send_graceful_signal(self, proc: subprocess.Popen) -> None:
        if os.name == "nt":
            ctrl = _generate_ctrl_break(proc.pid)
            if ctrl != 0:
                proc.terminate()
        else:
            import signal

            try:
                os.killpg(proc.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass

    def _control_file(self, proc: subprocess.Popen) -> str | None:
        return getattr(proc, "_lagrange_control_file", None)


class _WindowsJob:
    """ctypes wrapper over a Job Object (kernel32 + ntdll)."""

    def __init__(self, limits: IsolationLimits) -> None:
        import ctypes
        from ctypes import wintypes

        self._ctypes = ctypes
        self._kernel32 = ctypes.windll.kernel32
        self._ntdll = ctypes.windll.ntdll
        self._wintypes = wintypes

        class _BasicLimits(ctypes.Structure):
            _fields_ = [
                ("PerProcessUserTimeLimit", ctypes.c_longlong),
                ("PerJobUserTimeLimit", ctypes.c_longlong),
                ("LimitFlags", wintypes.DWORD),
                ("MinimumWorkingSetSize", ctypes.c_size_t),
                ("MaximumWorkingSetSize", ctypes.c_size_t),
                ("ActiveProcessLimit", wintypes.DWORD),
                ("Affinity", ctypes.c_size_t),
                ("PriorityClass", wintypes.DWORD),
                ("SchedulingClass", wintypes.DWORD),
            ]

        class _IoCounters(ctypes.Structure):
            _fields_ = [
                ("ReadOperationCount", ctypes.c_ulonglong),
                ("WriteOperationCount", ctypes.c_ulonglong),
                ("OtherOperationCount", ctypes.c_ulonglong),
                ("ReadTransferCount", ctypes.c_ulonglong),
                ("WriteTransferCount", ctypes.c_ulonglong),
                ("OtherTransferCount", ctypes.c_ulonglong),
            ]

        class _ExtendedLimits(ctypes.Structure):
            _fields_ = [
                ("BasicLimitInformation", _BasicLimits),
                ("IoInfo", _IoCounters),
                ("ProcessMemoryLimit", ctypes.c_size_t),
                ("JobMemoryLimit", ctypes.c_size_t),
                ("PeakProcessMemoryUsed", ctypes.c_size_t),
                ("PeakJobMemoryUsed", ctypes.c_size_t),
            ]

        self._ExtendedLimits = _ExtendedLimits
        self._handle = self._kernel32.CreateJobObjectW(None, None)
        if not self._handle:
            raise IsolationError("CreateJobObjectW failed")

        JOB_OBJECT_LIMIT_ACTIVE_PROCESS = 0x8
        JOB_OBJECT_LIMIT_PROCESS_MEMORY = 0x100
        JOB_OBJECT_LIMIT_PROCESS_TIME = 0x2
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x2000
        JobObjectExtendedLimitInformation = 9

        info = _ExtendedLimits()
        flags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        if limits.active_processes is not None:
            flags |= JOB_OBJECT_LIMIT_ACTIVE_PROCESS
            info.BasicLimitInformation.ActiveProcessLimit = limits.active_processes
        if limits.memory_bytes is not None:
            flags |= JOB_OBJECT_LIMIT_PROCESS_MEMORY
            info.ProcessMemoryLimit = limits.memory_bytes
        if limits.cpu_seconds is not None:
            flags |= JOB_OBJECT_LIMIT_PROCESS_TIME
            info.BasicLimitInformation.PerProcessUserTimeLimit = int(limits.cpu_seconds * 10_000_000)
        info.BasicLimitInformation.LimitFlags = flags
        ok = self._kernel32.SetInformationJobObject(
            self._handle, JobObjectExtendedLimitInformation, ctypes.byref(info), ctypes.sizeof(info)
        )
        if not ok:
            self.close()
            raise IsolationError("SetInformationJobObject failed")

    def assign(self, proc: subprocess.Popen) -> None:
        ok = self._kernel32.AssignProcessToJobObject(self._handle, int(proc._handle))
        if not ok:
            raise IsolationError("AssignProcessToJobObject failed")

    def resume(self, proc: subprocess.Popen) -> None:
        status = self._ntdll.NtResumeProcess(int(proc._handle))
        if status != 0:
            raise IsolationError(f"NtResumeProcess failed with status {status}")

    def terminate_all(self) -> None:
        self._kernel32.TerminateJobObject(self._handle, 137)

    def close(self) -> None:
        if self._handle:
            self._kernel32.CloseHandle(self._handle)
            self._handle = None


def _generate_ctrl_break(pid: int) -> int:
    import ctypes

    kernel32 = ctypes.windll.kernel32
    return kernel32.GenerateConsoleCtrlEvent(_CTRL_BREAK_EVENT, pid)
