"""Private CLI cleanup tests; no namespace or native process is started."""
import contextlib
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import types
import unittest
from unittest import mock


class PolicyCleanupTests(unittest.TestCase):
    def run_bootstrap(self, *, restore_fails=False, constructor_fails=False, body_fails=False):
        path = Path(__file__).with_name("verify-owner-beta-paper-native-source.py")
        spec = importlib.util.spec_from_file_location("native_policy_cleanup_cli", path)
        cli = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cli)
        unresolved = set()

        class Scope:
            def __enter__(self):
                return self

            def __exit__(self, *_exc):
                if restore_fails:
                    unresolved.add(self)
                    raise RuntimeError("NAMESPACE_RESTORE_UNKNOWN")

        scope = Scope()
        fixture = types.SimpleNamespace(
            _UNRESOLVED_POLICY_SCOPES=unresolved,
            validated_inputs=mock.Mock(return_value=("source", b"manifest", object())),
            termination_cleanup=contextlib.nullcontext,
            bounded_run=lambda _deadline: contextlib.nullcontext(),
        )
        history = types.SimpleNamespace(fixture_module=lambda: fixture,
            phase_deadline=mock.Mock(), canonical=lambda value: json.dumps(value).encode("ascii"))

        def require(value, code):
            if not value:
                raise RuntimeError(code)

        native = types.SimpleNamespace(history=lambda: history, require=require,
            NativePolicy=mock.Mock(return_value=scope),
            bootstrap=mock.Mock(return_value={"archive_acceptance": False}))
        if constructor_fails:
            native.NativePolicy.side_effect = RuntimeError("POLICY_CONSTRUCTOR_FAILED")
        if body_fails:
            native.bootstrap.side_effect = RuntimeError("BOOTSTRAP_FAILED")
        argv = ["bootstrap", "--source-root", "/source", "--commit", "a" * 40,
            "--manifest-file", "/manifest", "--deadline-monotonic-ns", "123",
            "--approved-base", "/base", "--raw-root", "/raw", "--daily-config", "/config",
            "--generic-bundle", "/bundle", "--generic-session", "2026-11-11",
            "--source-inputs", "/inputs"]
        with tempfile.TemporaryDirectory() as root, contextlib.ExitStack() as stack:
            for name in ("subprocess.Popen", "subprocess.run", "socket.socket", "os.chown", "os.fchown"):
                stack.enter_context(mock.patch(name, side_effect=AssertionError("UNPLANNED_EFFECT")))
            stack.enter_context(mock.patch.object(cli, "module", return_value=native))
            stack.enter_context(mock.patch.object(cli.os, "geteuid", return_value=0))
            stack.enter_context(mock.patch.object(cli.os, "getegid", return_value=0))
            stack.enter_context(mock.patch.object(cli.tempfile, "mkdtemp", return_value=root))
            removed = stack.enter_context(mock.patch.object(cli.shutil, "rmtree"))
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                code = cli.main(argv)
            return code, removed.call_args_list, output.getvalue(), native.bootstrap.call_count

    def test_unresolved_namespace_retains_policy_and_refuses_success(self):
        code, removed, output, calls = self.run_bootstrap(restore_fails=True)
        self.assertEqual((code, removed, calls), (1, [], 1))
        self.assertEqual(output, "PAPER_NATIVE_SOURCE: PHASE_UNPROVEN\n")

    def test_acknowledged_scope_cleanup_removes_temporary_policy(self):
        code, removed, output, calls = self.run_bootstrap()
        self.assertEqual((code, len(removed), calls), (0, 1, 1))
        self.assertEqual(json.loads(output), {"archive_acceptance": False})

    def test_constructor_failure_without_namespace_effect_allows_cleanup(self):
        code, removed, output, calls = self.run_bootstrap(constructor_fails=True)
        self.assertEqual((code, len(removed), calls), (1, 1, 0))
        self.assertEqual(output, "PAPER_NATIVE_SOURCE: PHASE_UNPROVEN\n")

    def test_bootstrap_failure_with_acknowledged_scope_cleanup_removes_policy(self):
        code, removed, output, calls = self.run_bootstrap(body_fails=True)
        self.assertEqual((code, len(removed), calls), (1, 1, 1))
        self.assertEqual(output, "PAPER_NATIVE_SOURCE: PHASE_UNPROVEN\n")


if __name__ == "__main__":
    unittest.main()
