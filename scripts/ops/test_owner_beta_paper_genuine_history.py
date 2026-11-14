"""Source-only tests. All subprocess and privilege effects are forbidden.

Mocks below check orchestration/refusal, never genuine acceptance or authority.
The private selected-image/PG/Source integration remains NOT_RUN here.
"""
import contextlib
import dataclasses
import importlib.util
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import types
import unittest
from unittest import mock

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent


def import_file(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


class Guarded(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.guard = contextlib.ExitStack()
        for target in ("subprocess.Popen", "subprocess.run", "os.chown", "os.fchown",
                       "socket.socket", "os.pidfd_open"):
            cls.guard.enter_context(mock.patch(target, side_effect=AssertionError("UNPLANNED_EFFECT")))
        cls.m = import_file("test_genuine_history_subject", HERE / "lib/owner-beta-paper-genuine-history.py")
        cls.cli = import_file("test_genuine_history_cli", HERE / "verify-owner-beta-paper-genuine-archive.py")

    @classmethod
    def tearDownClass(cls):
        cls.guard.close()

    def owner(self):
        return types.SimpleNamespace(context=object(), lock_fd=19, contexts=[], context_modules={},
            deadline=types.SimpleNamespace(value=123456789, seconds=mock.Mock(return_value=10)))


class Inputs(Guarded):
    def test_current_v3_registry_retains_275_and_77_originals(self):
        record = self.m.approved_record(self.m.ROOT)
        self.assertEqual(record["price_source"]["file_count"], 275)
        self.assertEqual(record["action_source"]["file_count"], 77)
        module = self.m.load("paper_genuine_history_simulation", "scripts/ops/lib/owner-beta-simulation-operations.py")
        original = (self.m.ROOT / "configs/evidence" / module.APPROVAL_NAME).read_bytes()
        with self.assertRaises(Exception):
            module.approved_record(original.replace(b"275", b"187"))

    def test_v3_thirteen_actual_service_order_and_revision(self):
        fixture = self.m.fixture_module()
        commit = "a" * 40
        rows = ["LAGRANGE_RELEASE_MANIFEST_V3", "commit|" + commit]
        rows += ["image|" + name + "|lagrange-station-" + name + ":" + commit + "|sha256:" + "b" * 64 + "|" + commit
                 for name in fixture.ops.daily.policy.MANIFEST_V3_SERVICES]
        data = ("\n".join(rows) + "\n").encode()
        self.assertEqual(len(self.m.image_manifest(data, commit)), 13)
        for invalid in (data.replace(b"V3", b"V2"), ("\n".join(rows[:-1]) + "\n").encode(),
                        data.replace(("|" + commit + "\n").encode(), ("|" + "c" * 40 + "\n").encode(), 1)):
            with self.subTest(invalid=invalid[:30]), self.assertRaises(Exception):
                self.m.image_manifest(invalid, commit)

    def test_deadline_is_original_builtin_integer_with_reserve_and_cap(self):
        now = 10**12
        with mock.patch.object(self.m.time, "monotonic_ns", return_value=now):
            for value in (True, float(now + 200_000_000_000), now + 120_000_000_000,
                          now + self.m.MAX_PHASE_NS + 1):
                with self.subTest(value=value), self.assertRaises(self.m.HistoryError):
                    self.m.phase_deadline(value)
            value = now + 300_000_000_000
            self.assertEqual(self.m.phase_deadline(value).value, value)

    def test_current_candidate_cannot_retrofit_bootstrap_release(self):
        owner = types.SimpleNamespace(commit="a" * 40, bootstrap_commit="a" * 40, prepared_process_sha256="sha256:" + "b" * 64)
        with self.assertRaises(self.m.HistoryError):
            self.m.prepared_release(owner)
        owner.commit = "c" * 40
        owner.prepared_process_sha256 = None
        with self.assertRaises(self.m.HistoryError):
            self.m.prepared_release(owner)

    def test_required_private_cli_inputs_have_no_default_material_or_ready_flags(self):
        parser = self.cli.parser()
        common = ["--source-root", "/private/source", "--commit", "a" * 40,
            "--manifest-file", "/private/manifest", "--deadline-monotonic-ns", "123"]
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            parser.parse_args(common + ["bootstrap"])
        args = parser.parse_args(common + ["accept", "--checkpoint", "/private/checkpoint", "--storage-config", "/private/config"])
        self.assertEqual(args.phase, "accept")
        with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit):
            parser.parse_args(common + ["accept", "--checkpoint", "/private/checkpoint", "--storage-config", "/private/config", "--ready"])


class ApiConstructor(Guarded):
    def test_real_private_runtime_start_api_uses_parser_supported_owner_paper_mode(self):
        owner=self.owner()
        owner.commit='a'*40;owner.root=Path('/owned');owner.network='private-network';owner.pg='postgres'
        owner.manifest=b'manifest'
        owner.observer=types.SimpleNamespace(json=mock.Mock(return_value=7))
        with mock.patch.object(self.m,'image_manifest',return_value={'api-server':'sha256:image'}):
            runtime=self.m.producer_runtime(owner)
        runtime.base=Path('/var/lib/lagrange/data/artifacts/historical-price-beta-root')
        runtime.secret_root=Path('/owned/secrets')
        runtime.secret_paths={(role,10001):runtime.secret_root/role for role in ('app','admin','audit_writer')}
        pin={'id':'11111111-1111-4111-8111-111111111111','dataset_id':'synthetic-test','version':'v1','manifest_sha256':'a'*64}
        captured={};events=[]
        def create(service,image,uid,network,mounts,env,entry,*args):
            captured.update(service=service,image=image,uid=uid,network=network,mounts=mounts,env=env,entry=entry)
            events.append('create');return 'owned-api'
        with mock.patch.object(runtime,'mount',side_effect=lambda source,target:(str(source),target)), \
             mock.patch.object(runtime,'create',side_effect=create), \
             mock.patch.object(runtime,'command',side_effect=lambda args:events.append(tuple(args))), \
             mock.patch.object(runtime,'inspect',return_value={'NetworkSettings':{'Networks':{'private':{'NetworkID':owner.network,'IPAddress':'172.20.0.2'}}}}), \
             mock.patch.object(runtime,'http',return_value=(200,{})) as http:
            runtime.start_api(pin)
        env=captured['env']
        self.assertEqual(env['OWNER_BETA_PAPER_MODE'],'enabled')
        self.assertEqual(env['OWNER_BETA_ACCESS_MODE'],'owner_only')
        self.assertEqual(env['OWNER_BETA_DAILY_MODE'],'owner_only')
        self.assertEqual(env['OWNER_BETA_PRICE_INPUT_MODE'],'sealed_v1')
        self.assertEqual((captured['entry'],captured['network'],captured['uid']),('/usr/local/bin/api-server',owner.network,10001))
        self.assertEqual(events,['create',('start','owned-api')])
        for role,prefix in (('app','DB'),('admin','ADMIN_DB'),('audit_writer','AUDIT_DB')):
            self.assertEqual([env[prefix+'_'+key] for key in ('HOST','PORT','NAME','USER')],['postgres','5432','lagrange',role])
            self.assertTrue(env[prefix+'_PASSWORD_FILE'].startswith('/run/secrets/'))
            self.assertNotIn(prefix+'_PASSWORD',env)
        self.assertEqual(env['RECOMMENDATION_CURATED_VERSION'],'7')
        self.assertEqual(env['RECOMMENDATION_DATASET_VERSION_ID'],pin['id'])
        self.assertEqual(env['RECOMMENDATION_DATASET_MANIFEST_SHA256'],pin['manifest_sha256'])
        self.assertTrue(env['SESSION_COOKIE_SECURE']=='true' and env['APP_ENV']=='production')
        self.assertFalse(any(key.endswith('_MODE_FILE') for key in env))
        http.assert_called_once_with('GET','/readyz',authenticated=False)


class Lifecycle(Guarded):
    def exercise(self, *, outcome="OK", cleanup="COMPLETE", readback=b'{"status":"COMMITTED"}', exit_error=False, action="enable"):
        owner = self.owner()
        events = []
        result = types.SimpleNamespace(outcome=outcome, cleanup=cleanup, readback=readback)
        controller = types.SimpleNamespace(context=object())
        def invoke(*args, **kwargs):
            events.append("invoke")
            return result
        for name in ("configure", "enable", "recover", "observe_reset", "readback"):
            setattr(controller, name, mock.Mock(side_effect=invoke))
        @contextlib.contextmanager
        def opened(context, lock, *, deadline):
            self.assertIs(context, owner.context)
            self.assertEqual(lock, 19)
            self.assertIs(type(deadline), int)
            self.assertEqual(deadline, owner.deadline.value)
            events.append("open")
            yield controller
            events.append("close")
            if exit_error:
                raise self.m.HistoryError("HISTORY_CLOSE_UNKNOWN")
        module = types.SimpleNamespace(open_lifecycle=opened, ops=object())
        def parse(raw, maximum):
            self.assertEqual(events[-1], "close")
            self.assertEqual(maximum, 1048576)
            return json.loads(raw)
        patches = contextlib.ExitStack()
        patches.enter_context(mock.patch.object(self.m, "lifecycle_module", return_value=module))
        patches.enter_context(mock.patch.object(self.m, "lifecycle_context", return_value=owner.context))
        patches.enter_context(mock.patch.object(self.m, "fixture_module", return_value=types.SimpleNamespace(
            ops=types.SimpleNamespace(journal=types.SimpleNamespace(_json=parse, MAX_NATIVE_EXACT=1048576)))))
        return owner, controller, events, patches, lambda: self.m.lifecycle_action(owner, action, caller_key="retained-key", proposal=b"{}")

    def test_independent_committed_readback_is_consumed_after_owner_exit(self):
        owner, controller, events, patches, invoke = self.exercise()
        with patches:
            self.assertEqual(invoke().outcome, "OK")
        self.assertEqual(events, ["open", "invoke", "close"])
        controller.enable.assert_called_once_with("retained-key")
        self.assertIn(controller.context, owner.contexts)

    def test_ack_loss_blocked_and_unknown_cleanup_never_retry_or_succeed(self):
        for options in ({"outcome": "COMMIT_UNKNOWN"}, {"outcome": "BLOCKED"}, {"cleanup": "UNKNOWN"},
                        {"readback": None}, {"readback": b'{"status":"ABSENT"}'}, {"exit_error": True}):
            with self.subTest(options=options):
                _, controller, _, patches, invoke = self.exercise(**options)
                with patches, self.assertRaises(self.m.HistoryError):
                    invoke()
                controller.enable.assert_called_once()

    def test_observe_reset_uses_reset_readback_semantics(self):
        _, controller, events, patches, invoke = self.exercise(action="observe_reset", readback=b'{"status":"RESET"}')
        with patches:
            self.assertEqual(invoke().cleanup, "COMPLETE")
        controller.observe_reset.assert_called_once_with()
        self.assertEqual(events[-1], "close")

    def test_context_adapter_reopens_protected_release_and_rejects_changes(self):
        @dataclasses.dataclass(frozen=True)
        class Context:
            release: str
            commit: str
            images: dict
            values: dict
            overlays: list
        expected = Context("/owned/releases/rev", "rev", {"image": "id"}, {"root": "owned"}, [])
        owner = types.SimpleNamespace(release=Path(expected.release), commit="rev", context=expected)
        daily = types.SimpleNamespace(Context=Context, read_manifest=mock.Mock(return_value=expected.images),
            read_context=mock.Mock(return_value=(expected.values, [])))
        self.assertEqual(self.m.lifecycle_context(owner, types.SimpleNamespace(daily=daily)), expected)
        daily.read_context.return_value = ({"root": "foreign"}, [])
        with self.assertRaises(self.m.HistoryError):
            self.m.lifecycle_context(owner, types.SimpleNamespace(daily=daily))


class Acceptance(Guarded):
    def fixture(self, rows):
        class Owner:
            pass
        owner = Owner()
        owner.context = object()
        owner.lock_fd = 11
        owner.observer = object()
        owner.deadline = types.SimpleNamespace(seconds=mock.Mock())
        owner.source, owner.commit, owner.manifest = "/source", "c" * 40, b"original"
        runner = types.SimpleNamespace(results=rows, archive_acceptance=mock.Mock())
        fixture = types.SimpleNamespace(FixtureOwner=Owner, FixtureRunner=mock.Mock(return_value=runner),
            driver=types.SimpleNamespace(ArchiveAcceptance=types.SimpleNamespace(FAULTS=("ciphertext", "intake-cancel"))))
        return fixture, owner, runner

    def test_direct_c_consumer_and_all_required_results(self):
        rows = [{"case": case, "status": "PASS"} for case in
            ("ARCHIVE_LIFECYCLE", "ARCHIVE_CIPHERTEXT", "ARCHIVE_INTAKE_CANCEL")]
        fixture, owner, runner = self.fixture(rows)
        values = {"BACKUP_ROOT": "/owned/backup"}
        with mock.patch.object(self.m, "fixture_module", return_value=fixture):
            self.assertEqual(self.m.archive_acceptance(owner, values, policy_root="policy"), tuple(rows))
        runner.archive_acceptance.assert_called_once_with(owner, values)

    def test_normal_return_with_unknown_failed_or_missing_case_is_nonacceptance(self):
        for fault in ("UNKNOWN", "FAIL", None):
            rows = [{"case": "ARCHIVE_LIFECYCLE", "status": "PASS"}, {"case": "ARCHIVE_CIPHERTEXT", "status": "PASS"}]
            if fault is not None:
                rows.append({"case": "ARCHIVE_INTAKE_CANCEL", "status": fault})
            fixture, owner, runner = self.fixture(rows)
            with self.subTest(fault=fault), mock.patch.object(self.m, "fixture_module", return_value=fixture), self.assertRaises(self.m.HistoryError):
                self.m.archive_acceptance(owner, {}, policy_root="policy")
            runner.archive_acceptance.assert_called_once()

    def test_cli_does_not_turn_unknown_acceptance_into_success(self):
        common = ["--source-root", "/private/source", "--commit", "a" * 40, "--manifest-file", "/private/manifest",
            "--deadline-monotonic-ns", "123", "accept", "--checkpoint", "/private/checkpoint", "--storage-config", "/private/config"]
        subject = types.SimpleNamespace(fixture_module=lambda: types.SimpleNamespace(termination_cleanup=contextlib.nullcontext),
            run=lambda _: {"status": "PASS", "archive_acceptance": True, "cases": [{"status": "UNKNOWN"}]},
            require=self.m.require, canonical=self.m.canonical)
        with mock.patch.object(self.cli, "history_module", return_value=subject), contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertEqual(self.cli.main(common), 1)
        self.assertEqual(output.getvalue(), "PAPER_GENUINE_HISTORY_FAILED:PHASE_UNPROVEN\n")


class ProducerAdapters(Guarded):
    def test_notification_contention_uses_owned_transaction_and_reaps_it(self):
        fixture = self.m.fixture_module()
        owner = types.SimpleNamespace(pg="owned-pg", argv=lambda args: ["docker", *args], blockers=[],
            deadline=types.SimpleNamespace(seconds=mock.Mock(return_value=10)))
        owner.observer = types.SimpleNamespace(owner=owner, target=mock.Mock(), backends=set(),
            json=mock.Mock(side_effect=[1, 0]))
        child = types.SimpleNamespace(stdin=mock.Mock(closed=False), stdout=mock.Mock(closed=False),
            poll=mock.Mock(side_effect=[None, None, 0]), wait=mock.Mock(return_value=0), returncode=0)
        selector = mock.MagicMock()
        selector.__enter__.return_value = selector
        selector.select.return_value = [object()]
        raw = b'{"pid":42,"start":"2026-11-11 00:00:00+00"}\nHISTORY_NOTIFICATION_LOCKED\n'
        with mock.patch.object(fixture.subprocess, "Popen", return_value=child) as popen, mock.patch.object(
                self.m.selectors, "DefaultSelector", return_value=selector), mock.patch.object(self.m.os, "read", return_value=raw):
            with self.m.notification_lock(owner) as blocker:
                self.assertEqual(blocker.backend["pid"], 42)
        self.assertIn(b"LOCK TABLE public.notifications IN ACCESS EXCLUSIVE MODE", child.stdin.write.call_args_list[0].args[0])
        self.assertEqual(child.stdin.write.call_args_list[-1].args[0], b"ROLLBACK;\n\\q\n")
        child.wait.assert_called_once()
        self.assertEqual(len(owner.blockers), 1)
        self.assertIn("owned-pg", popen.call_args.args[0])
        self.assertIn("backend_start::text", owner.observer.json.call_args.args[0])

    def test_generic_collector_calls_real_binary_and_uses_returned_batch(self):
        batch = "11111111-1111-4111-8111-111111111111"
        runtime = types.SimpleNamespace(config={"entitlement_reference": "fixture-only:test"},
            images={"research-worker": "sha256:" + "a" * 64}, owner=object(), network="owned-network",
            generic_raw=Path("/owned/raw"), curated=Path("/owned/curated"),
            db_env=mock.Mock(return_value=({}, [])), mount=lambda *args: args,
            once=mock.Mock(return_value=self.m.canonical(dict(status="ok", phase="complete", outcome="published", batch_id=batch))))
        pin = {"id": "actual-returned-dataset"}
        with mock.patch.object(self.m, "dataset_pin", return_value=pin) as lookup:
            self.assertIs(self.m.collect_generic(runtime, "2026-11-10", "/private/synthetic"), pin)
        lookup.assert_called_once_with(runtime.owner, batch)
        call = runtime.once.call_args.args
        self.assertEqual(call[2:4], (10001, "owned-network"))
        self.assertEqual(call[6:], ("/usr/local/bin/research-worker", ["--once", "--date", "2026-11-10"]))
        self.assertEqual(call[5]["RESEARCH_FETCH_MODE"], "synthetic")
        self.assertEqual(call[5]["RESEARCH_CANDIDATE_ENABLED"], "false")
        self.assertNotIn("--raw-source", call[7])

    def test_expired_claim_observes_owned_worker_and_never_updates_rows(self):
        claim = "11111111-1111-4111-8111-111111111111"
        token = "22222222-2222-4222-8222-222222222222"
        image = {"Created": "created", "State": {"StartedAt": "start", "Running": True},
            "RestartCount": 0, "Image": "sha256:" + "a" * 64,
            "NetworkSettings": {"Networks": {"owned": {"NetworkID": "network", "IPAddress": "172.20.0.3"}}}}
        stopped = {**image, "State": {**image["State"], "Running": False}}
        observer = types.SimpleNamespace(json=mock.Mock(side_effect=[0, {}, {"id": claim, "token": token},
            [{"pid": 42, "start": "lock-generation", "address": None, "blocked": False},
             {"pid": 41, "start": "actual-generation", "address": "172.20.0.3", "blocked": True}], 0,
            {"token": token, "delivered": None, "expired": True}]))
        runtime = types.SimpleNamespace(config={"owner": claim}, network="network", owner=types.SimpleNamespace(pg="postgres", observer=observer),
            create=mock.Mock(return_value="worker"), command=mock.Mock(), remaining=mock.Mock(return_value=2),
            inspect=mock.Mock(side_effect=[image, {"Containers": {"postgres": {}, "worker": {}}}, image, stopped]))
        blocker = types.SimpleNamespace(backend={"pid": 42, "start": "lock-generation"})
        with mock.patch.object(self.m, "paper_command", return_value=("paper",)), mock.patch.object(
                self.m, "notification_lock", return_value=contextlib.nullcontext(blocker)):
            self.m.expire_owned_outbox_claim(runtime)
        self.assertEqual(runtime.command.call_args_list, [mock.call(["start", "worker"]),
            mock.call(["kill", "--signal=KILL", "worker"]), mock.call(["wait", "worker"])])
        self.assertTrue(all(call.args[0].startswith("SELECT") for call in observer.json.call_args_list))
        self.assertIn("clock_timestamp()", observer.json.call_args_list[-1].args[0])

    def test_generic_catalog_pin_is_read_from_actual_collector_batch(self):
        identifier = "11111111-1111-4111-8111-111111111111"
        pin = dict(id=identifier, dataset_id="krx_eod_bars", version="produced", status="READY",
                   manifest_sha256="f" * 64, storage_path="curated/produced")
        owner = types.SimpleNamespace(observer=types.SimpleNamespace(json=mock.Mock(return_value=[pin])))
        self.assertEqual(self.m.dataset_pin(owner, identifier), pin)
        query = owner.observer.json.call_args.args[0]
        self.assertIn("candidate_raw_batch_datasets", query)
        self.assertIn(identifier, query)
        self.assertNotIn("INSERT", query)
        owner.observer.json.return_value = []
        with self.assertRaises(self.m.HistoryError):
            self.m.dataset_pin(owner, identifier)

    def test_generic_preview_binds_without_auto_apply(self):
        identifier = "11111111-1111-4111-8111-111111111111"
        runtime = types.SimpleNamespace(http=mock.Mock(side_effect=[(201, {"id": identifier}), (200, {}), (202, {"id": identifier})]))
        self.m.generic_preview(runtime, identifier, "2026-11-10")
        bind = runtime.http.call_args_list[1]
        self.assertTrue(bind.args[1].endswith("/bind-strategy"))
        self.assertIs(bind.args[2]["auto_apply_recommendations"], False)
        self.assertFalse(any(call.args[1].endswith("/apply") for call in runtime.http.call_args_list))

    def test_generic_preview_must_be_ready_unapplied_and_not_beta(self):
        identifier = "11111111-1111-4111-8111-111111111111"
        row = dict(dataset=identifier, lineage=None, target=None, applied=None, status="READY")
        query = mock.Mock(return_value=row)
        owner = types.SimpleNamespace(observer=types.SimpleNamespace(json=query))
        self.m.verify_generic_preview(owner, identifier, {"id": identifier})
        for key, value in (("lineage", identifier), ("target", identifier), ("applied", "time"), ("status", "PENDING"), ("dataset", "wrong")):
            query.return_value = {**row, key: value}
            with self.subTest(key=key), self.assertRaises(self.m.HistoryError):
                self.m.verify_generic_preview(owner, identifier, {"id": identifier})

    def test_daily_missing_real_source_reservation_stops_before_producers(self):
        runtime = types.SimpleNamespace(config={"owner": "11111111-1111-4111-8111-111111111111"},
            owner=types.SimpleNamespace(commit="a" * 40, observer=types.SimpleNamespace(json=mock.Mock(return_value=None))),
            daily_once=mock.Mock(), verify_base_views=mock.Mock())
        with self.assertRaises(self.m.HistoryError):
            self.m.daily_cycle(runtime)
        runtime.daily_once.assert_not_called()

    def test_daily_runs_original_three_producers_then_observes_source_links(self):
        runtime = types.SimpleNamespace(config={"owner": "11111111-1111-4111-8111-111111111111"},
            owner=types.SimpleNamespace(commit="a" * 40, observer=types.SimpleNamespace(json=mock.Mock(side_effect=["sha256:" + "f" * 64, 1]))),
            daily_once=mock.Mock(), verify_base_views=mock.Mock(), remaining=mock.Mock(return_value=2))
        self.m.daily_cycle(runtime)
        self.assertEqual(runtime.daily_once.call_args_list, [mock.call("producer", "CANDIDATE_READY"),
            mock.call("admitter", "ADMITTED"), mock.call("worker", "SUCCEEDED")])
        self.assertIn("source_runs", runtime.owner.observer.json.call_args.args[0])

    def test_expired_claim_refuses_foreign_backend_before_start(self):
        runtime = types.SimpleNamespace(owner=types.SimpleNamespace(observer=types.SimpleNamespace(json=mock.Mock(return_value=1))), create=mock.Mock())
        with self.assertRaises(self.m.HistoryError):
            self.m.expire_owned_outbox_claim(runtime)
        runtime.create.assert_not_called()

    def test_material_exit_revalidates_originals_even_after_body_failure(self):
        material = self.m.HistoricalMaterial("source", "base", "raw", object())
        material.stack = mock.MagicMock()
        stack = material.stack
        material.revalidate = mock.Mock(side_effect=self.m.HistoryError("HISTORY_MATERIAL_CHANGED"))
        with self.assertRaises(self.m.HistoryError):
            material.__exit__(ValueError, ValueError("body"), None)
        material.revalidate.assert_called_once()
        stack.__exit__.assert_called_once()
        self.assertIsNone(material.stack)


class PhaseOwnership(Guarded):
    def test_bootstrap_setup_failure_disposes_inside_extraction_custody(self):
        held = []
        @contextlib.contextmanager
        def extractions():
            held.append(True)
            try:
                yield
            finally:
                held.pop()
        owner = types.SimpleNamespace(extractions=extractions(), setup=mock.Mock(side_effect=self.m.HistoryError("HISTORY_SETUP")),
            dispose=mock.Mock(side_effect=lambda close: self.assertEqual(held, [True])))
        fixture = types.SimpleNamespace(FixtureOwner=mock.Mock(return_value=owner))
        deadline = types.SimpleNamespace(cleanup_phase=mock.Mock())
        with mock.patch.object(self.m, "fixture_module", return_value=fixture), \
             mock.patch.object(self.m, "source_module", return_value=types.SimpleNamespace(
                 dispose_with_source_custody=lambda owned: owned.dispose(lambda: None))), \
             self.assertRaises(self.m.HistoryError):
            self.m.bootstrap("/source", "a" * 40, b"manifest", deadline, approved_base="/base", raw_root="/raw",
                config_source="/config", generic_bundle="/bundle", generic_session="2026-11-10", policy_root="policy")
        owner.dispose.assert_called_once()
        deadline.cleanup_phase.assert_called_once()
        self.assertEqual(held, [])

    def test_resume_adopts_later_published_release_and_retains_bootstrap_journal_identity(self):
        actual = self.m.fixture_module()
        root = Path("/var/lib/lagrange-paper-operations-fixture-unit")
        old, new = "a" * 40, "b" * 40
        account = "11111111-1111-4111-8111-111111111111"
        checkpoint = dict(schema=self.m.SCHEMA, root_identity=[1, 2], bootstrap_commit=old,
            manifest_sha256=self.m.digest(b"old-manifest"), project="paper-operations-fixture-unit",
            network="network", postgres="pg", postgres_image="pg-image", postgres_generation=["created", "started", 0, "pg-image"],
            account_id=account, release_hashes={"capability": "old"}, producer_hashes={"producer": "old"},
            facts={"actual": "bootstrap-description"}, installation_id=account, bootstrap_floor=0)
        values = {"LAGRANGE_DATA_DIR": str(root / "data"), "LAGRANGE_ARTIFACTS_DIR": str(root / "artifacts"),
            "LAGRANGE_RUNTIME_SECRET_DIR": str(root / "secrets")}
        reads = {str(root / "genuine-bootstrap.json"): self.m.canonical(checkpoint),
            str(root / "releases" / old / actual.ops.release.MANIFEST_NAME): b"old-manifest",
            str(root / "releases" / new / actual.ops.release.MANIFEST_NAME): b"new-manifest"}
        old_release = types.SimpleNamespace(revalidate=mock.Mock(), process_sha256=None)
        new_release = types.SimpleNamespace(revalidate=mock.Mock(), process_sha256="actual-new-process")
        journal = types.SimpleNamespace(installation_id=account, epoch_floor=1, revalidate=mock.Mock())
        observer = types.SimpleNamespace(generation=None, target=mock.Mock(), json=mock.Mock(return_value=account))
        live = types.SimpleNamespace(probe_history=lambda: contextlib.nullcontext(types.SimpleNamespace(_configured=True)))
        with contextlib.ExitStack() as patches:
            for target, replacement in (("private_policy", mock.Mock()), ("image_manifest", mock.Mock()),
                    ("release_descriptors", mock.Mock(side_effect=[({"capability": "old"}, {"producer": "old"}),
                        ({"capability": "new"}, {"producer": "new"})]))):
                patches.enter_context(mock.patch.object(self.m, target, replacement))
            for obj, name, replacement in ((self.m.os, "open", mock.Mock(return_value=19)),
                    (self.m.os, "close", mock.Mock()), (self.m.fcntl, "flock", mock.Mock()),
                    (actual.faults, "directory_identity", mock.Mock(return_value=(1, 2))),
                    (actual, "FixtureDocker", mock.Mock()), (actual, "P3ExtractionCustody", mock.Mock()),
                    (actual, "PgObserver", mock.Mock(return_value=observer)), (actual.ops, "_file", mock.Mock()),
                    (actual.ops.daily.install, "read_root_file", mock.Mock(side_effect=lambda path, **_: reads[path])),
                    (actual.ops.journal, "open_journal", mock.Mock(return_value=contextlib.nullcontext(journal))),
                    (actual.ops.release, "verify", mock.Mock(side_effect=[contextlib.nullcontext(old_release), contextlib.nullcontext(new_release)])),
                    (actual.ops.daily, "read_manifest", mock.Mock(return_value={"image": "new"})),
                    (actual.ops.daily, "read_context", mock.Mock(return_value=(values, []))),
                    (actual.ops.daily.policy, "validate_current_release", mock.Mock()),
                    (actual.ops, "open_context", mock.Mock(return_value=contextlib.nullcontext(live)))):
                patches.enter_context(mock.patch.object(obj, name, replacement))
            owner, facts = self.m.resume_owner("/source", new, b"new-manifest", object(), root / "genuine-bootstrap.json", policy_root="policy")
            actual.ops.daily.policy.validate_current_release.assert_called_once_with(str(root), str(root / "releases" / new), new)
            self.assertEqual(owner.commit, new)
            self.assertEqual(owner.bootstrap_commit, old)
            self.assertEqual(owner.prepared_process_sha256, "actual-new-process")
            self.assertEqual(owner.release_hashes, {"capability": "new"})
            self.assertEqual(owner.producer_hashes, {"producer": "new"})
            self.assertEqual(owner.account, account)
            self.assertEqual(owner.observer.generation, tuple(checkpoint["postgres_generation"]))
            self.assertEqual(facts, checkpoint["facts"])
            self.m.prepared_release(owner)

    def run_acceptance(self, *, failure=False, namespace_failure=False, custody_unknown=False):
        events = []
        deadline = types.SimpleNamespace(cleanup_phase=lambda: events.append("cleanup-phase"))
        held = []
        @contextlib.contextmanager
        def extractions():
            held.append(True)
            try:
                yield
            finally:
                held.pop()
        owner = types.SimpleNamespace(extractions=extractions(), lock_fd=None, cleanup={},
            observer=types.SimpleNamespace(target=lambda: events.append("target")))
        def dispose(close):
            self.assertEqual(held, [True], "P3 extraction custody must remain entered through supervisor disposal")
            events.append("dispose")
            owner.cleanup.update(containers="REMOVED", inputs="REMOVED_AFTER_DISPOSITION")
        owner.dispose = dispose
        @contextlib.contextmanager
        def policy(path):
            events.append("namespace-enter")
            yield
            events.append("namespace-exit")
            if namespace_failure:
                raise self.m.HistoryError("HISTORY_NAMESPACE_UNKNOWN")
        def acceptance(*_args, **kwargs):
            self.assertIs(kwargs["source_custody"].owner, owner)
            events.append("acceptance")
            if failure:
                raise self.m.HistoryError("HISTORY_ACCEPTANCE_UNPROVEN")
            return {"status": "PASS", "archive_acceptance": True, "cases": [{"status": "PASS"}]}
        fixture = types.SimpleNamespace(validated_inputs=lambda *_: (Path("/source"), b"manifest", deadline),
            bounded_run=lambda _: contextlib.nullcontext(), ServiceStatusScope=policy,
            _UNRESOLVED_POLICY_SCOPES=set())
        args = types.SimpleNamespace(phase="accept", source_root="/source", commit="a" * 40,
            manifest_file="/manifest", deadline_monotonic_ns=1, checkpoint="/checkpoint")
        patches = contextlib.ExitStack()
        for target, value in (("fixture_module", fixture), ("phase_deadline", deadline),
                              ("resume_owner", (owner, {}))):
            patches.enter_context(mock.patch.object(self.m, target, return_value=value))
        patches.enter_context(mock.patch.object(self.m, "retained_phase", side_effect=acceptance))
        @contextlib.contextmanager
        def source_custody(owned):
            self.assertIs(owned, owner)
            if custody_unknown:
                raise RuntimeError("SOURCE_DISPOSITION_UNPROVEN")
            yield types.SimpleNamespace(owner=owner)
        patches.enter_context(mock.patch.object(self.m, "source_module", return_value=types.SimpleNamespace(
            require_source_custody_for_acceptance=source_custody,
            dispose_with_source_custody=lambda owned, custody: owned.dispose(lambda: None))))
        patches.enter_context(mock.patch.object(self.m.os, "geteuid", return_value=0))
        patches.enter_context(mock.patch.object(self.m.os, "getegid", return_value=0))
        # Only a reversible ordinary-user tempfile is used. No privilege call.
        directory = tempfile.mkdtemp(prefix="genuine-history-unit-")
        patches.enter_context(mock.patch.object(self.m.tempfile, "mkdtemp", return_value=directory))
        return events, patches, lambda: self.m.run(args), Path(directory)

    def test_acceptance_and_disposal_precede_success_and_namespace_exit(self):
        events, patches, run, path = self.run_acceptance()
        with patches:
            result = run()
        self.assertEqual(result["status"], "PASS")
        self.assertEqual(events, ["namespace-enter", "acceptance", "cleanup-phase", "dispose", "namespace-exit"])
        self.assertFalse(path.exists())

    def test_failed_or_unknown_acceptance_still_attempts_owned_disposition(self):
        events, patches, run, path = self.run_acceptance(failure=True)
        with patches, self.assertRaises(self.m.HistoryError):
            run()
        self.assertIn("dispose", events)
        self.assertFalse(path.exists())

    def test_unknown_source_custody_blocks_disposal_on_acceptance_failure(self):
        events, patches, run, path = self.run_acceptance(failure=True, custody_unknown=True)
        with patches, self.assertRaises(Exception) as caught:
            run()
        self.assertEqual(str(caught.exception), "SOURCE_DISPOSITION_UNPROVEN")
        self.assertNotIn("dispose", events)
        self.assertFalse(path.exists())

    def test_unknown_source_custody_blocks_disposal_on_acceptance_success(self):
        events, patches, run, path = self.run_acceptance(custody_unknown=True)
        with patches, self.assertRaises(Exception) as caught:
            run()
        self.assertEqual(str(caught.exception), "SOURCE_DISPOSITION_UNPROVEN")
        self.assertNotIn("dispose", events)
        self.assertFalse(path.exists())

    def test_namespace_close_uncertainty_prevents_success(self):
        events, patches, run, path = self.run_acceptance(namespace_failure=True)
        with patches, self.assertRaises(self.m.HistoryError):
            run()
        self.assertIn("dispose", events)
        self.assertFalse(path.exists())


class RawManifestLinePins(Guarded):
    """Producer rules, not this module's: storage.rs appends serde compact JSON + LF;
    the V3 registry pins hash that physical line including its LF
    (paper_archive_sources.rs pinned_terminated_line_hash, ADR-0006). Daily's
    manifest_line_sha256 excludes the LF (daily_price_only/source.rs)."""
    BATCHES = {"price_source": ("kis-daily-range", "bars", "11111111-1111-1111-1111-111111111111"),
               "action_source": ("kis", "corporate_actions", "22222222-2222-2222-2222-222222222222")}

    def material(self, rule):
        refs = self.m.load("paper_genuine_history_references", "scripts/ops/lib/owner-beta-paper-backup-references.py")
        values, record = {}, {}
        for kind, (provider, raw_kind, batch_id) in self.BATCHES.items():
            body = ("opaque " + kind).encode()
            row = {"batch_id": batch_id, "provider": provider, "market": "kr", "date": "2026-08-28",
                   "retrieved_at": "2026-08-28T07:30:00Z", "mode": "credentialed", "entitlement_reference": None,
                   "files": [{"kind": raw_kind, "file_name": "page.json", "content_hash": self.m.digest(body),
                              "size_bytes": len(body), "request": {"endpoint": "fixture:read-only", "query": [],
                              "headers": [], "mode": "credentialed"}}]}
            other = dict(row, batch_id="33333333-3333-3333-3333-333333333333", date="2026-08-27")
            line = json.dumps(row, separators=(",", ":")).encode()
            batch = "provider=" + provider + "/market=kr/date=2026-08-28/batch=" + batch_id
            batch_json = json.dumps(row, indent=2).encode() + b"\n"
            values["manifests/provider=" + provider + "/market=kr/manifest.jsonl"] = (
                json.dumps(other, separators=(",", ":")).encode() + b"\n" + line + b"\n")
            values[batch + "/batch.json"], values[batch + "/page.json"] = batch_json, body
            record[kind] = {"batch_id": batch_id, "batch_json_sha256": self.m.digest(batch_json),
                            "manifest_line_sha256": self.m.digest(rule(line)), "file_count": 1}
        lock = tempfile.TemporaryFile()
        self.addCleanup(lock.close)
        os.fchmod(lock.fileno(), 0o640)
        material = self.m.HistoricalMaterial.__new__(self.m.HistoricalMaterial)
        material.members, material.directories, material.bytes, material.record = {}, [], 0, record
        material.deadline = types.SimpleNamespace(seconds=mock.Mock(return_value=10))
        material.base = types.SimpleNamespace(files=refs.files)
        material._open = lambda path: (None, "commit.lock", lock.fileno(), os.fstat(lock.fileno()))
        material._read = lambda path, maximum: values[path]
        return material

    def test_registry_pin_including_lf_selects_the_actual_manifest_line(self):
        self.material(lambda line: line + b"\n")._read_sources()

    def test_lf_excluded_daily_hash_is_not_a_v3_registry_pin(self):
        with self.assertRaisesRegex(self.m.HistoryError, "^HISTORY_RAW_MANIFEST$"):
            self.material(lambda line: line)._read_sources()


if __name__ == "__main__":
    unittest.main()
