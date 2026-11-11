"""Protected Paper source-reference traversal and descriptive candidate framing.

An entered source-owned paper-combined Snapshot and complete database stream
select the actual originals. prepare_references retains the shared graph owner;
describe reuses that traversal and adds six-field source_dataset_row frames.
These candidates never confer restore authority: the native consumer independently
reconstructs every domain and source/clone equality while retaining the owners.
Labels/keys use deterministic decimal UTF-8 byte-length framing. P1 reference
rows remain unchanged; supplemental datasets use their own distinct frame kind.
"""
from __future__ import annotations

import contextlib
import datetime
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import stat
import sys
from decimal import Decimal

sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent


def _local(name, filename):
    path = HERE / filename
    existing = sys.modules.get(name)
    if existing is not None:
        if Path(existing.__file__).resolve() != path:
            raise ImportError("REFERENCES_MODULE")
        return existing
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


files = _local("_paper_references_files", "owner-beta-daily-backup-files.py")
audit = _local("_paper_references_audit", "owner-beta-paper-audit.py")
daily = _local("_paper_references_daily", "owner-beta-daily-release.py")
# Importing these libraries does not execute their command-line entry points.
simulation = _local("_paper_references_simulation", "owner-beta-simulation-operations.py")
paper = _local("_paper_references_release", "owner-beta-paper-release.py")
journal = _local("_paper_references_journal", "owner-beta-paper-journal.py")

I64 = (1 << 63) - 1
MAX_DOCUMENT = 2 * 1024 * 1024
MAX_LINE = 2 * 1024 * 1024
MAX_COPY_DEPTH = 64
SHA = re.compile(r"sha256:[0-9a-f]{64}\Z")
UUID = re.compile(r"[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}\Z")
LEAF = re.compile(r"[A-Za-z0-9][A-Za-z0-9._-]{0,191}\Z")
PARTITION = re.compile(r"[a-z][a-z0-9_-]{0,63}\Z")
RAW_KINDS = frozenset(("bars", "reference", "calendar", "corporate_actions", "investor_flow",
    "market_status", "fundamentals", "index_membership", "sector_classification", "candidate_master",
    "disclosure_index", "disclosure_entity_master", "disclosure_entity_profile", "disclosure_version_membership"))
RAW_MODES = frozenset(("synthetic", "credentialed"))
RAW_BATCH_KEYS = frozenset(("batch_id", "provider", "market", "date", "retrieved_at",
                            "mode", "entitlement_reference", "files"))
RAW_FILE_REQUIRED = frozenset(("kind", "file_name", "content_hash", "size_bytes", "request"))
RAW_FILE_OPTIONAL = frozenset(("response_continuation", "copied_from"))
COPY_KEYS = frozenset(("provider", "market", "batch_id", "file_name", "content_hash", "retrieved_at"))


class ReferenceError(ValueError):
    """Messages are fixed codes only; provider/SQL/OS exception prose is suppressed."""


def require(condition, code="REFERENCES_INVALID"):
    if not condition:
        raise ReferenceError(code)


def _sha(raw):
    return "sha256:" + hashlib.sha256(raw).hexdigest()


def _hash(value):
    require(type(value) is str and SHA.fullmatch(value), "REFERENCES_HASH")
    return value


def _integer(value, maximum=I64):
    require(type(value) in (int, Decimal)
            and (type(value) is int or value.is_finite())
            and 0 <= value <= maximum and value == int(value), "REFERENCES_LIMIT")
    return int(value)


def _json(raw, maximum=MAX_DOCUMENT):
    require(type(raw) is bytes and 0 < len(raw) <= maximum, "REFERENCES_DOCUMENT_SIZE")
    try:
        result = audit.parse(raw)
        require(type(result) is dict, "REFERENCES_DOCUMENT")
        return result
    except audit.AuditError:
        raise ReferenceError("REFERENCES_DOCUMENT") from None


def _wire(value):
    """Rust struct JSON preserves field order; economic JSON uses audit.canonical."""
    def convert(v):
        if type(v) is Decimal:
            require(v == v.to_integral_value() and abs(v) <= I64, "REFERENCES_NUMBER")
            return int(v)
        if type(v) is dict:
            return {k: convert(x) for k, x in v.items()}
        if type(v) is list:
            return [convert(x) for x in v]
        require(type(v) in (str, int, bool, type(None)), "REFERENCES_NUMBER")
        return v
    return json.dumps(convert(value), ensure_ascii=False, separators=(",", ":"),
                      allow_nan=False).encode("utf-8")


def _utc9(value):
    """Exact instant comparison without float/microsecond rounding of nanoseconds."""
    require(type(value) is str, "REFERENCES_TIMESTAMP")
    match = re.fullmatch(r"(\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d)(?:\.(\d{1,9}))?(Z|[+-]\d\d:\d\d)", value)
    require(match is not None, "REFERENCES_TIMESTAMP")
    try:
        moment = datetime.datetime.fromisoformat(match[1] + match[3].replace("Z", "+00:00"))
        moment = moment.astimezone(datetime.timezone.utc)
    except ValueError:
        raise ReferenceError("REFERENCES_TIMESTAMP") from None
    return moment.strftime("%Y-%m-%dT%H:%M:%S") + "." + (match[2] or "").ljust(9, "0") + "Z"


def _raw_json(data, maximum=MAX_DOCUMENT):
    _json(data, maximum)  # duplicate keys, nesting and constants fail first
    def integer(text):
        require(text != "-0", "REFERENCES_SOURCE_NUMBER_UNIMPLEMENTED")
        return int(text)
    return json.loads(data, parse_int=integer, parse_float=Decimal)


def _raw_value(row):
    """ManifestEntry serde equality, including optional None defaults and instants.

    Vec ordering is preserved (files, request pairs). Unknown fields are rejected
    by the surrounding closed reader. Original bytes are never normalized/replaced.
    """
    value = dict(row)
    value["retrieved_at"] = _utc9(value["retrieved_at"])
    value["files"] = []
    for item in row["files"]:
        require(type(item) is dict and RAW_FILE_REQUIRED <= set(item) <= RAW_FILE_REQUIRED | RAW_FILE_OPTIONAL
                and type(item["size_bytes"]) is int and 0 <= item["size_bytes"] <= (1 << 64) - 1,
                "REFERENCES_RAW_FILE")
        copied = item.get("copied_from")
        if copied is not None:
            copied = {**copied, "retrieved_at": _utc9(copied["retrieved_at"])}
        value["files"].append({**item, "response_continuation": item.get("response_continuation"),
                               "copied_from": copied})
    return value


def _identity_reference(row, hash_key="wire_sha256"):
    require(type(row) is dict, "REFERENCES_ORIGINAL")
    provider, market, batch, leaf = (row.get(k) for k in ("provider", "market", "batch_id", "file_name"))
    require(type(provider) is str and PARTITION.fullmatch(provider)
            and type(market) is str and PARTITION.fullmatch(market)
            and type(batch) is str and UUID.fullmatch(batch)
            and type(leaf) is str and LEAF.fullmatch(leaf) and leaf != "batch.json",
            "REFERENCES_ORIGINAL")
    return provider, market, batch, leaf, _hash(row.get(hash_key))


def _forbidden(path):
    # No credentials, env files, ephemeral ownership or socket authority.
    parts = path.split("/")
    return any(p == "commit.lock" or p.endswith((".lock", ".sock", ".socket", ".pem", ".key"))
               or p == ".env" or p.endswith(".env") or p in ("credentials", "password", "pgpass")
               or p.startswith(".pgpass") for p in parts)


class _Budget:
    def __init__(self, limits):
        self.entries = limits.entries
        self.bytes = limits.manifest_bytes
        self.used_entries = self.used_bytes = 0

    def add(self, value):
        encoded = audit.canonical(value)
        require(self.used_entries < self.entries and len(encoded) <= self.bytes - self.used_bytes,
                "REFERENCES_INDEX_LIMIT")
        self.used_entries += 1
        self.used_bytes += len(encoded)


class _FramedRoot:
    """A provisional accumulator; no readiness or archive authority."""
    def __init__(self):
        self.hash = hashlib.sha256()
        self.entries = self.bytes = 0
        self.previous = None

    def add(self, table, key, row):
        identity = table.encode("ascii"), key.encode("ascii")
        require(self.previous is None or self.previous < identity, "REFERENCES_ORDER")
        raw = audit.frame(table, key, row)
        require(self.entries < I64 and len(raw) <= I64 - self.bytes, "REFERENCES_OVERFLOW")
        self.hash.update(raw)
        self.entries += 1
        self.bytes += len(raw)
        self.previous = identity

    def _description(self):
        return {"group": "source_files", "sha256": "sha256:" + self.hash.hexdigest(),
                "entries": self.entries, "bytes": self.bytes}


class _Protected:
    """Custody of an existing entered Snapshot; never manufactures originals.

    Source modules are often imported under different names by existing ops
    libraries. Accept their exact Snapshot class from the fixed codec file,
    never duck-typed readers, subclasses, dictionaries or serialized manifests.
    Python code execution inside this process remains part of the trust base.
    """
    def __init__(self, snapshot):
        cls = type(snapshot)
        module = sys.modules.get(cls.__module__)
        require(module is not None and getattr(module, "Snapshot", None) is cls
                and Path(getattr(module, "__file__", "")).resolve() == HERE / "owner-beta-daily-backup-files.py"
                and cls.__name__ == "Snapshot", "REFERENCES_SNAPSHOT")
        require(snapshot.profile == "paper-combined" and snapshot.stack is not None
                and tuple(snapshot.classes) == files.PAPER_CLASSES, "REFERENCES_SNAPSHOT")
        self.snapshot = snapshot
        self.source_class = cls
        self.roots = tuple(snapshot.roots)
        self.descriptors = dict(snapshot.descriptors)
        self.limits = files.Limits(snapshot.limits.total_bytes, snapshot.limits.entries,
                                   snapshot.limits.manifest_bytes)
        self.manifest_bytes = cls.manifest(snapshot)
        manifest = files.validate_manifest(self.manifest_bytes, self.limits, profile="paper-combined")
        self.entries = {e["path"]: dict(e) for e in manifest["entries"]}
        self.identities = {k: tuple(v) for k, v in snapshot.identities.items()}
        require(set(self.identities) == set(self.entries), "REFERENCES_SNAPSHOT")
        require(not any(_forbidden(p) for p in self.entries), "REFERENCES_FORBIDDEN_MEMBER")
        self.failed = False
        self.closed = False
        self.revalidate()
        cls.verify_unchanged(snapshot)

    def revalidate(self):
        require(not self.failed and not self.closed, "REFERENCES_INVALIDATED")
        try:
            s = self.snapshot
            require(s.stack is not None and tuple(s.roots) == self.roots
                    and s.descriptors == self.descriptors and s.identities == self.identities
                    and self.source_class.manifest(s) == self.manifest_bytes,
                    "REFERENCES_SNAPSHOT_CHANGED")
            for root in self.roots:
                with files.open_root(root) as named:
                    retained = self.descriptors[root.name]
                    require(files.identity(os.fstat(named)) == self.identities[root.name]
                            == files.identity(os.fstat(retained)), "REFERENCES_REPLACED")
        except BaseException:
            self.failed = True
            raise

    @contextlib.contextmanager
    def _member(self, path):
        """Only source-owned resolvers below call this with catalog-selected names."""
        self.revalidate()
        handles = []
        try:
            require(type(path) is str and path in self.entries and not _forbidden(path),
                    "REFERENCES_MEMBER")
            parts = files.safe_parts(path)
            require(len(parts) > 1 and self.entries[path]["kind"] == "file", "REFERENCES_MEMBER")
            parent = self.descriptors[parts[0]]
            prefix = parts[0]
            links = []
            for index, name in enumerate(parts[1:]):
                prefix += "/" + name
                record = self.entries.get(prefix)
                directory = index != len(parts) - 2
                require(record is not None and record["kind"] == ("directory" if directory else "file"),
                        "REFERENCES_MEMBER")
                fd = os.open(name, files.DIRECTORY if directory else files.REGULAR, dir_fd=parent)
                handles.append(fd)
                links.append((parent, name, fd, prefix, directory))
                self._check_link(*links[-1])
                parent = fd
            yield handles[-1], self.entries[path]
            for link in reversed(links):
                self._check_link(*link)
            self.revalidate()
        except BaseException:
            self.failed = True
            raise
        finally:
            for fd in reversed(handles):
                os.close(fd)

    def _check_link(self, parent, name, fd, path, directory):
        entry = self.entries[path]
        current = files.checked(fd, directory=directory, owners={entry["uid"]}, groups={entry["gid"]})
        require(stat.S_IMODE(current.st_mode) == entry["mode"]
                and files.identity(current) == self.identities[path]
                == files.identity(os.stat(name, dir_fd=parent, follow_symlinks=False)),
                "REFERENCES_REPLACED")

    def read(self, path, maximum=MAX_DOCUMENT, metadata=None):
        with self._member(path) as (fd, entry):
            require(entry["size"] <= maximum, "REFERENCES_DOCUMENT_SIZE")
            self.metadata(path, metadata)
            data = bytearray()
            hasher = hashlib.sha256()
            remaining = entry["size"]
            while remaining:
                part = os.read(fd, min(files.CHUNK, remaining))
                require(part, "REFERENCES_TRUNCATED")
                hasher.update(part)
                data.extend(part)
                remaining -= len(part)
            require(not os.read(fd, 1), "REFERENCES_SIZE_CHANGED")
            require("sha256:" + hasher.hexdigest() == entry["sha256"], "REFERENCES_CONTENT_CHANGED")
            return bytes(data)

    def hash_file(self, path, expected=None, size=None, metadata=None):
        with self._member(path) as (fd, entry):
            self.metadata(path, metadata)
            require(expected is None or entry["sha256"] == _hash(expected), "REFERENCES_PIN")
            require(size is None or entry["size"] == _integer(size), "REFERENCES_SIZE")
            hasher = hashlib.sha256()
            files.read_chunks(fd, entry["size"], hasher)
            require("sha256:" + hasher.hexdigest() == entry["sha256"], "REFERENCES_CONTENT_CHANGED")
            return entry["sha256"]

    def lines(self, path, maximum=MAX_LINE):
        # Caller must exhaust this generator. Internal traversal always does.
        complete = False
        with self._member(path) as (fd, entry):
            remaining = entry["size"]
            hasher = hashlib.sha256()
            pending = b""
            try:
                while remaining:
                    part = os.read(fd, min(65536, remaining))
                    require(part, "REFERENCES_TRUNCATED")
                    hasher.update(part)
                    remaining -= len(part)
                    pending += part
                    while b"\n" in pending:
                        line, pending = pending.split(b"\n", 1)
                        require(0 < len(line) <= maximum, "REFERENCES_LINE_SIZE")
                        yield line
                    require(len(pending) <= maximum, "REFERENCES_LINE_SIZE")
                require(not pending and not os.read(fd, 1), "REFERENCES_TRUNCATED")
                require("sha256:" + hasher.hexdigest() == entry["sha256"], "REFERENCES_CONTENT_CHANGED")
                complete = True
            finally:
                if not complete:
                    self.failed = True

    def metadata(self, path, expected):
        if expected is not None:
            entry = self.entries[path]
            require((entry["uid"], entry["gid"], entry["mode"]) == expected, "REFERENCES_METADATA")

    def finish(self):
        self.revalidate()
        try:
            self.source_class.verify_unchanged(self.snapshot)
            self.revalidate()
        except BaseException:
            self.failed = True
            raise
        self.closed = True


class _RawIndex:
    """Complete retained committed Raw manifest/batch/content and copy closure.

    Inputs to this index come only from _scan_raw's protected reads. Methods are
    private pure grammar/graph helpers, never source or admission authority.
    """
    def __init__(self, budget):
        self.budget = budget
        self.files = {}
        self.batches = {}

    def add(self, row, batch_path, batch_sha, line_sha, physical_line=1):
        require(set(row) == RAW_BATCH_KEYS and type(row["files"]) is list and row["files"],
                "REFERENCES_RAW_BATCH")
        date = row["date"]
        require(type(date) is str and re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", date),
                "REFERENCES_RAW_BATCH")
        try:
            require(datetime.date.fromisoformat(date).isoformat() == date, "REFERENCES_RAW_BATCH")
            retrieved = datetime.datetime.fromisoformat(row["retrieved_at"].replace("Z", "+00:00"))
            require(retrieved.tzinfo is not None, "REFERENCES_RAW_BATCH")
        except (ValueError, TypeError, AttributeError):
            raise ReferenceError("REFERENCES_RAW_BATCH") from None
        require(type(row["mode"]) is str and row["mode"] in RAW_MODES
                and (row["entitlement_reference"] is None or type(row["entitlement_reference"]) is str),
                "REFERENCES_RAW_BATCH")
        # Validate the partition identity before using it as a hash-map key.
        require(type(row["provider"]) is str and PARTITION.fullmatch(row["provider"])
                and type(row["market"]) is str and PARTITION.fullmatch(row["market"])
                and type(row["batch_id"]) is str and UUID.fullmatch(row["batch_id"]), "REFERENCES_RAW_BATCH")
        identity = row["provider"], row["market"], row["batch_id"]
        equivalent = _raw_value(row)
        prior = self.batches.get(identity)
        if prior is not None:
            require(prior["value"] == equivalent and prior["path"] == batch_path
                    and prior["batch_sha256"] == batch_sha, "REFERENCES_RAW_DUPLICATE_CONFLICT")
            self.budget.add([list(identity), line_sha, physical_line])
            prior["lines"].append([line_sha, physical_line])
            return {item["file_name"] for item in row["files"]}
        self.budget.add([*identity, batch_path, equivalent])
        lines = [[line_sha, physical_line]]
        self.batches[identity] = {"value": equivalent, "path": batch_path,
                                 "batch_sha256": batch_sha, "lines": lines}
        names = set()
        for item in row["files"]:
            require(type(item) is dict and RAW_FILE_REQUIRED <= set(item)
                    <= RAW_FILE_REQUIRED | RAW_FILE_OPTIONAL, "REFERENCES_RAW_FILE")
            key = _identity_reference({**row, "file_name": item["file_name"],
                                       "content_hash": item["content_hash"]}, "content_hash")
            require(key not in self.files and item["file_name"] not in names, "REFERENCES_RAW_DUPLICATE")
            names.add(item["file_name"])
            require(type(item["request"]) is dict and set(item["request"]) == {"endpoint", "query", "headers", "mode"},
                    "REFERENCES_RAW_REQUEST")
            request = item["request"]
            require(type(item["kind"]) is str and item["kind"] in RAW_KINDS
                    and request["mode"] == row["mode"]
                    and type(request["endpoint"]) is str and request["endpoint"], "REFERENCES_RAW_REQUEST")
            for name in ("query", "headers"):
                require(type(request[name]) is list and all(type(pair) is list and len(pair) == 2
                        and all(type(value) is str for value in pair) for pair in request[name]),
                        "REFERENCES_RAW_REQUEST")
            require(item.get("response_continuation") is None or type(item["response_continuation"]) is str,
                    "REFERENCES_RAW_REQUEST")
            _integer(item["size_bytes"])
            origin = item.get("copied_from")
            if origin is not None:
                require(type(origin) is dict and set(origin) == COPY_KEYS, "REFERENCES_RAW_COPY")
                _identity_reference(origin, "content_hash")
            value = {"path": batch_path + "/" + item["file_name"], "batch_sha256": batch_sha,
                     "line_sha256": line_sha, "manifest_lines": lines, "date": date, "retrieved_at": row["retrieved_at"],
                     "entitlement_reference": row["entitlement_reference"],
                     "file": item}
            self.budget.add([list(key), value])
            self.files[key] = value
        return names

    def close(self):
        for key, value in self.files.items():
            visited = {key}
            current = value
            for _ in range(MAX_COPY_DEPTH):
                origin = current["file"].get("copied_from")
                if origin is None:
                    break
                target = _identity_reference(origin, "content_hash")
                require(target not in visited, "REFERENCES_COPY_CYCLE")
                visited.add(target)
                older = self.files.get(target)
                require(older is not None, "REFERENCES_COPY_MISSING")
                require(_utc9(older["retrieved_at"]) == _utc9(origin["retrieved_at"]), "REFERENCES_COPY_LINK")
                for field in ("kind", "content_hash", "size_bytes", "request"):
                    require(older["file"][field] == current["file"][field], "REFERENCES_COPY_LINK")
                require(older["file"].get("response_continuation") == current["file"].get("response_continuation"),
                        "REFERENCES_COPY_LINK")
                current = older
            else:
                raise ReferenceError("REFERENCES_COPY_DEPTH")

    def reference(self, row):
        key = _identity_reference(row)
        require(key in self.files, "REFERENCES_RAW_MISSING")
        return self.files[key]


def _scan_raw(protected, budget):
    index = _RawIndex(budget)
    covered = set()
    for path in sorted(protected.entries, key=lambda s: s.encode("utf-8")):
        match = re.fullmatch(r"raw/manifests/provider=([a-z][a-z0-9_-]{0,63})/market=([a-z][a-z0-9_-]{0,63})/manifest\.jsonl", path)
        if not match:
            continue
        root = next(r for r in protected.roots if r.name == "raw")
        require(protected.entries[path]["uid"] == root.uid
                and protected.entries[path]["gid"] == root.gid, "REFERENCES_RAW_OWNER")
        covered.add(path)
        for physical_line, line in enumerate(protected.lines(path), 1):
            row = _raw_json(line, MAX_LINE)
            require(row.get("provider") == match[1] and row.get("market") == match[2], "REFERENCES_RAW_SCOPE")
            require(type(row.get("batch_id")) is str and UUID.fullmatch(row["batch_id"])
                    and type(row.get("date")) is str
                    and re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", row["date"]), "REFERENCES_RAW_BATCH")
            # The location is constructed from the fixed storage partition contract,
            # never from a SQL row's arbitrary path field.
            directory = ("raw/provider=" + match[1] + "/market=" + match[2]
                         + "/date=" + row["date"] + "/batch=" + row["batch_id"])
            batch_path = directory + "/batch.json"
            raw = protected.read(batch_path, MAX_DOCUMENT)
            require(protected.entries[batch_path]["uid"] == root.uid
                    and protected.entries[batch_path]["gid"] == root.gid, "REFERENCES_RAW_OWNER")
            require(_raw_value(_raw_json(raw)) == _raw_value(row), "REFERENCES_RAW_MANIFEST_BATCH")
            names = index.add(row, directory, _sha(raw), _sha(line), physical_line)
            expected = {batch_path} | {directory + "/" + n for n in names}
            actual = {p for p in protected.entries if p.startswith(directory + "/")}
            require(actual == expected, "REFERENCES_RAW_INVENTORY")
            covered |= expected
            for item in row["files"]:
                protected.hash_file(directory + "/" + item["file_name"], item["content_hash"],
                                    item["size_bytes"])
                entry = protected.entries[directory + "/" + item["file_name"]]
                require(entry["uid"] == root.uid and entry["gid"] == root.gid, "REFERENCES_RAW_OWNER")
    expected_inventory = {"raw"} | covered
    for path in covered:
        parts = path.split("/")
        expected_inventory.update("/".join(parts[:n]) for n in range(1, len(parts)))
    require({p for p in protected.entries if p == "raw" or p.startswith("raw/")}
            == expected_inventory, "REFERENCES_RAW_UNCOMMITTED")
    index.close()
    return index


class _References:
    """Bounded retained SQL reference index, populated only inside real decode."""
    def __init__(self, limits, projection):
        self.tables = {t: {} for t in projection["source_reference_tables"]}
        self.jobs = {}
        self.attempts = {}
        self.budget = _Budget(limits)
        self.done = False
        self.failed = False

    def _row(self, table, key, row):
        require(not self.done and not self.failed and table in self.tables, "REFERENCES_STREAM_STATE")
        require(key not in self.tables[table], "REFERENCES_DUPLICATE")
        self.budget.add([table, key, row])
        self.tables[table][key] = row

    def decode(self, stream):
        require(not self.done and not self.failed, "REFERENCES_STREAM_STATE")
        owner = self
        class ReadThrough:
            def readline(self, maximum):
                line = stream.readline(maximum)
                if type(line) is bytes and line.endswith(b"\n") and len(line) <= maximum:
                    rec = audit.parse(line[:-1])
                    # These rows are provisional until the same decoder validates
                    # every row, group, reference equality and terminal record.
                    if type(rec) is list and len(rec) == 5 and rec[:2] == ["row", "paper_jobs"]:
                        table, key, row = rec[2:]
                        if table in ("jobs", "job_attempts"):
                            target = owner.jobs if table == "jobs" else owner.attempts
                            require(type(key) is str and key not in target, "REFERENCES_DUPLICATE")
                            owner.budget.add([table, key, row])
                            target[key] = row
                return line
        try:
            result = audit.decode(ReadThrough(), self._row)
            self.done = True
            return result
        except BaseException:
            self.failed = True
            self.tables.clear()
            self.jobs.clear()
            self.attempts.clear()
            raise

    def rows(self, table):
        require(self.done and not self.failed, "REFERENCES_STREAM_STATE")
        return self.tables[table].values()

    def keyed(self, table, field):
        result = {}
        for row in self.rows(table):
            value = row[field]
            require(value not in result, "REFERENCES_AMBIGUOUS")
            result[value] = row
        return result


def _source_links(refs, raw):
    slots = refs.keyed("owner_beta_paper_source_slots", "slot_id")
    admissions = refs.keyed("owner_beta_paper_source_admissions", "slot_id")
    daily_rows = refs.keyed("owner_beta_daily_admissions", "admission_sha256")
    source_runs = refs.keyed("owner_beta_paper_source_runs", "slot_id")
    runs = refs.keyed("owner_beta_recommendation_runs", "id")
    reservations = refs.keyed("owner_beta_daily_reservations", "run_id")
    claims = refs.keyed("owner_beta_daily_publication_claims", "run_id")
    original_sets = {}
    for row in refs.rows("owner_beta_paper_source_originals"):
        slot = slots.get(row["slot_id"])
        admission = admissions.get(row["slot_id"])
        require(slot is not None and admission is not None, "REFERENCES_SLOT_ADMISSION")
        daily_row = daily_rows.get(admission["admission_sha256"])
        require(daily_row is not None and daily_row["owner_user_id"] == slot["owner_user_id"]
                and daily_row["policy_sha256"] == slot["policy_sha256"]
                and daily_row["range_end"] == slot["session_date"], "REFERENCES_SLOT_ADMISSION")
        original = raw.reference(row)
        require(original["date"] == slot["session_date"], "REFERENCES_SLOT_ORIGINAL_DATE")
        original_sets.setdefault(row["slot_id"], []).append(_identity_reference(row))
    for slot_id, admission in admissions.items():
        require(slot_id in slots and admission["admission_sha256"] in daily_rows, "REFERENCES_SLOT_ADMISSION")
        identities = sorted(original_sets.get(slot_id, []))
        require(len(identities) == 23 and len(set(identities)) == 23, "REFERENCES_ORIGINAL_SET")
        exact = ("\n".join("|".join(key) for key in identities) + "\n").encode("ascii")
        require(_sha(exact) == admission["original_set_sha256"], "REFERENCES_ORIGINAL_SET")
    for slot_id, source in source_runs.items():
        slot = slots.get(slot_id)
        admission = admissions.get(slot_id)
        run = runs.get(source["run_id"])
        reservation = reservations.get(source["run_id"])
        claim = claims.get(source["run_id"])
        require(all(x is not None for x in (slot, admission, run, reservation, claim)), "REFERENCES_RUN_CHAIN")
        job = refs.jobs.get(source["job_id"])
        attempt = refs.attempts.get(source["attempt_id"])
        require(job is not None and attempt is not None and attempt["job_id"] == source["job_id"]
                and job["owner_user_id"] == slot["owner_user_id"]
                and attempt["attempt_no"] == claim["attempt_no"]
                and attempt["claimed_by"] == claim["claimed_by"],
                "REFERENCES_JOB_ATTEMPT")
        require(run["job_id"] == reservation["job_id"] == claim["job_id"] == source["job_id"]
                and claim["attempt_id"] == source["attempt_id"]
                and run["owner_user_id"] == reservation["owner_user_id"] == claim["owner_user_id"] == slot["owner_user_id"]
                and run["strategy_config_id"] == reservation["strategy_config_id"]
                and run["as_of"] == reservation["session_date"] == slot["session_date"]
                and reservation["admission_sha256"] == admission["admission_sha256"]
                and reservation["activation_epoch"] == slot["activation_epoch"], "REFERENCES_RUN_CHAIN")
    # Immutable beta lineage is authoritative; current controls or the generic
    # recommendation_run_id must never substitute for this historical join.
    lineage = refs.keyed("owner_beta_paper_lineage", "id")
    for target in refs.rows("pending_targets"):
        if target["source_kind"] not in ("OWNER_BETA_DAILY_MANUAL", "OWNER_BETA_DAILY_AUTO"):
            continue
        line = lineage.get(target["beta_lineage_id"])
        require(line is not None and target["recommendation_run_id"] is None
                and all(target[k] is None for k in ("dataset_version", "dataset_version_id", "dataset_manifest_sha256")),
                "REFERENCES_LINEAGE")
        for a, b in (("owner_user_id", "owner_user_id"), ("account_id", "account_id"),
                     ("strategy_config_id", "strategy_config_id"), ("computed_on", "signal_date"),
                     ("effective_date", "effective_session"), ("targets_json", "target_weights_json")):
            require(target[a] == line[b], "REFERENCES_LINEAGE")
        run = runs.get(line["recommendation_run_id"])
        require(run is not None and run["status"] == "SUCCEEDED" and run["trigger_kind"] == "DAILY"
                and run["daily_config_epoch"] == line["config_epoch"]
                and run["daily_activation_epoch"] == line["daily_activation_epoch"]
                and run["owner_user_id"] == line["owner_user_id"]
                and run["strategy_config_id"] == line["strategy_config_id"] and run["as_of"] == line["signal_date"]
                and run["factor_snapshot_sha256"] == line["factor_snapshot_sha256"]
                and run["target_snapshot_sha256"] == line["target_snapshot_sha256"], "REFERENCES_LINEAGE")


# Closed D4 field order from daily_price_only/contract.rs. This is not an input
# schema selector: callers cannot add types, paths, table names or relationships.
_ORIGIN = ("provider", "market", "batch_id", "batch_date", "batch_json_sha256", "manifest_line_sha256",
           "file_name", "kind", "wire_sha256", "size_bytes", "request", "response_continuation",
           "retrieved_at", "retrieval_precision")
_FILE = _ORIGIN + ("original_retrieved_at", "original_retrieval_precision", "copied_from")
_KEY = ("provider", "market", "batch_id", "file_name", "wire_sha256")
_DOCUMENT = ("kind", "session_date", "normalizer_version", "sha256", "size_bytes", "row_count",
             "source_files", "source_evidence_sha256")
_FRONTIER = ("schema", "owner", "policy_sha256", "admission_sha256", "parent_frontier_sha256",
             "range_start", "range_end", "observation_sha256", "source_file_count")
_OBSERVATION_PREFIX = b"owner-beta-daily-source-observation-v1\n"
_ENDPOINTS = {
    "/uapi/domestic-stock/v1/quotations/inquire-daily-itemchartprice": ("bars", "FHKST03010100"),
    "/uapi/domestic-stock/v1/quotations/inquire-price": ("reference", "FHKST01010100"),
    "/uapi/domestic-stock/v1/quotations/chk-holiday": ("calendar", "CTCA0903R"),
    "/uapi/domestic-stock/v1/ksdinfo/paidin-capin": ("corporate_actions", "HHKDB669100C0"),
    "/uapi/domestic-stock/v1/ksdinfo/bonus-issue": ("corporate_actions", "HHKDB669101C0"),
    "/uapi/domestic-stock/v1/ksdinfo/dividend": ("corporate_actions", "HHKDB669102C0"),
    "/uapi/domestic-stock/v1/ksdinfo/merger-split": ("corporate_actions", "HHKDB669104C0"),
    "/uapi/domestic-stock/v1/ksdinfo/rev-split": ("corporate_actions", "HHKDB669105C0"),
    "/uapi/domestic-stock/v1/ksdinfo/cap-dcrs": ("corporate_actions", "HHKDB669106C0"),
}


def _object(value, fields, code="REFERENCES_DAILY_SHAPE"):
    require(type(value) is dict and set(value) == set(fields), code)
    return {key: value[key] for key in fields}


def _date(value):
    require(type(value) is str and re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}", value), "REFERENCES_DATE")
    try:
        require(datetime.date.fromisoformat(value).isoformat() == value, "REFERENCES_DATE")
    except ValueError:
        raise ReferenceError("REFERENCES_DATE") from None
    return value


def _pairs_ordered(values):
    require(type(values) is list and len(values) <= 262144 and all(type(p) is list and len(p) == 2
            and all(type(v) is str for v in p) for p in values), "REFERENCES_REQUEST_PAIRS")
    ordered = sorted(values, key=lambda pair: pair[0].encode("utf-8"))
    require(len({pair[0] for pair in ordered}) == len(ordered), "REFERENCES_REQUEST_PAIRS")
    return ordered


def _request(value, kind):
    result = _object(value, ("endpoint", "query", "headers", "mode"))
    result["query"] = _pairs_ordered(result["query"])
    result["headers"] = _pairs_ordered(result["headers"])
    endpoint = _ENDPOINTS.get(result["endpoint"])
    headers = dict(result["headers"])
    require(result["mode"] == "credentialed" and endpoint is not None and endpoint[0] == kind
            and set(headers) == {"appkey", "appsecret", "authorization", "tr_cont", "tr_id"}
            and all(headers[k] == "[REDACTED]" for k in ("appkey", "appsecret", "authorization"))
            and headers["tr_id"] == endpoint[1] and headers["tr_cont"] in ("", "N"), "REFERENCES_DAILY_REQUEST")
    return result


def _origin(value, raw):
    result = _object(value, _ORIGIN)
    original = raw.reference(result)
    require(result["provider"] in ("kis", "kis-calendar") and result["market"] == "kr"
            and (result["provider"] != "kis-calendar" or result["kind"] == "calendar")
            and result["batch_id"] != "00000000-0000-0000-0000-000000000000"
            and 0 < _integer(result["size_bytes"], 67108864)
            and result["retrieval_precision"] == "batch", "REFERENCES_DAILY_ORIGINAL")
    result["request"] = _request(result["request"], result["kind"])
    marker = result["response_continuation"]
    require(marker is None or type(marker) is str and re.fullmatch(r"[A-Za-z0-9]{0,8}", marker),
            "REFERENCES_DAILY_ORIGINAL")
    require(_date(result["batch_date"]) == original["date"]
            and result["batch_json_sha256"] == original["batch_sha256"]
            and result["manifest_line_sha256"] == original["line_sha256"]
            and result["retrieved_at"] == _utc9(original["retrieved_at"]), "REFERENCES_DAILY_ORIGINAL")
    item = original["file"]
    require(item["kind"] == result["kind"] and item["size_bytes"] == result["size_bytes"]
            and item.get("response_continuation") == marker
            and _request(item["request"], item["kind"]) == result["request"], "REFERENCES_DAILY_ORIGINAL")
    return result, original


def _file_evidence(value, raw):
    source = _object(value, _FILE)
    result, original = _origin({k: source[k] for k in _ORIGIN}, raw)
    require(source["original_retrieval_precision"] == "batch" and type(source["copied_from"]) is list
            and len(source["copied_from"]) <= 16, "REFERENCES_DAILY_COPY")
    copies, seen = [], {_identity_reference(result)}
    current, previous_time = original, result["retrieved_at"]
    for claim in source["copied_from"]:
        copied = current["file"].get("copied_from")
        require(copied is not None and _identity_reference(claim) == _identity_reference(copied, "content_hash"),
                "REFERENCES_DAILY_COPY")
        origin, current = _origin(claim, raw)
        identity = _identity_reference(origin)
        require(identity not in seen and origin["retrieved_at"] <= previous_time, "REFERENCES_DAILY_COPY")
        require(all(origin[k] == result[k] for k in ("kind", "wire_sha256", "size_bytes", "request", "response_continuation")),
                "REFERENCES_DAILY_COPY")
        seen.add(identity)
        copies.append(origin)
        previous_time = origin["retrieved_at"]
    require(current["file"].get("copied_from") is None
            and source["original_retrieved_at"] == previous_time, "REFERENCES_DAILY_COPY")
    result.update(original_retrieved_at=previous_time, original_retrieval_precision="batch", copied_from=copies)
    return result


def _observation(files_value, raw):
    require(type(files_value) is list and 0 < len(files_value) <= 262144, "REFERENCES_OBSERVATION")
    encoded, total = {}, len(_OBSERVATION_PREFIX) + 2
    for value in files_value:
        file = _file_evidence(value, raw)
        wire = _wire(file)
        total += len(wire) + 1
        require(total <= 16777216, "REFERENCES_OBSERVATION_LIMIT")
        encoded[wire] = file
    ordered = sorted(encoded)
    return _OBSERVATION_PREFIX + b"[" + b",".join(ordered) + b"]", [encoded[k] for k in ordered]


def _frontiers(protected, refs, raw, originals):
    """Validate every sealed frontier, including unreferenced retained history."""
    result = {}
    prefix = "daily/frontiers/"
    leaves = set()
    for path in protected.entries:
        if not path.startswith(prefix):
            continue
        parts = path[len(prefix):].split("/")
        require(re.fullmatch(r"[0-9a-f]{64}", parts[0]) and len(parts) in (1, 2)
                and (len(parts) == 1 or parts[1] in ("frontier.json", "observation.json")), "REFERENCES_FRONTIER_INVENTORY")
        leaves.add(parts[0])
    admissions = refs.keyed("owner_beta_daily_admissions", "admission_sha256")
    for leaf in sorted(leaves):
        key = "sha256:" + leaf
        directory = prefix + leaf
        record_raw = protected.read(directory + "/frontier.json", 1048576, (10003, 10001, 0o440))
        observation_raw = protected.read(directory + "/observation.json", 16777216, (10003, 10001, 0o440))
        record = _object(_json(record_raw, 1048576), _FRONTIER)
        require(record["schema"] == "owner-beta-daily-source-frontier-v1" and _wire(record) == record_raw
                and type(record["owner"]) is str and UUID.fullmatch(record["owner"])
                and record["owner"] != "00000000-0000-0000-0000-000000000000"
                and record["observation_sha256"] == key == _sha(observation_raw)
                and _date(record["range_start"]) <= _date(record["range_end"]), "REFERENCES_FRONTIER")
        _hash(record["policy_sha256"])
        _hash(record["admission_sha256"])
        if record["parent_frontier_sha256"] is not None:
            _hash(record["parent_frontier_sha256"])
        require(0 < _integer(record["source_file_count"], 262144), "REFERENCES_FRONTIER")
        require(observation_raw.startswith(_OBSERVATION_PREFIX), "REFERENCES_OBSERVATION")
        values = audit.parse(observation_raw[len(_OBSERVATION_PREFIX):])
        exact, observed = _observation(values, raw)
        require(exact == observation_raw and record["source_file_count"] == len(observed), "REFERENCES_OBSERVATION")
        admission = admissions.get(record["admission_sha256"])
        retained = originals.get(record["admission_sha256"])
        require(retained is not None and retained["manifest"]["range_end"] == record["range_end"]
                and retained["admission"]["policy_sha256"] == record["policy_sha256"], "REFERENCES_FRONTIER_ADMISSION")
        # Retained but not DB-admitted originals remain descriptive inventory.
        if admission is not None:
            require(admission["owner_user_id"] == record["owner"]
                    and admission["policy_sha256"] == record["policy_sha256"]
                    and admission["range_end"] == record["range_end"], "REFERENCES_FRONTIER_ADMISSION")
        raw.budget.add(record)
        result[key] = record
    for key, record in result.items():
        parent = record["parent_frontier_sha256"]
        if parent is not None:
            ancestor = result.get(parent)
            require(ancestor is not None and ancestor["owner"] == record["owner"]
                    and ancestor["policy_sha256"] == record["policy_sha256"]
                    and ancestor["range_start"] == record["range_start"]
                    and ancestor["range_end"] < record["range_end"], "REFERENCES_FRONTIER_PARENT")
    for row in admissions.values():
        key = row["evidence_json"].get("source_frontier_sha256")
        require(key in result and result[key]["admission_sha256"] == row["admission_sha256"], "REFERENCES_FRONTIER_MISSING")
    return result


def _calendar_provider_days(document, target):
    require(document.get("rt_cd") == "0", "REFERENCES_CALENDAR_PROVIDER")
    rows = document.get("output")
    if type(rows) is dict:
        rows = [rows]
    require(type(rows) is list and 0 < len(rows) <= 370, "REFERENCES_CALENDAR_PROVIDER")
    result = {}
    for ordinal, row in enumerate(rows):
        require(type(row) is dict and type(row.get("bass_dt")) is str
                and re.fullmatch(r"[0-9]{8}", row["bass_dt"]) and row.get("opnd_yn") in ("Y", "N"),
                "REFERENCES_CALENDAR_PROVIDER")
        date = row["bass_dt"][:4] + "-" + row["bass_dt"][4:6] + "-" + row["bass_dt"][6:]
        _date(date)
        require(date not in result, "REFERENCES_CALENDAR_CONFLICT")
        result[date] = {"session_date": date, "session_type": "TRADING" if row["opnd_yn"] == "Y" else "CLOSED",
            "evidence_kind": "REQUESTED_TARGET" if date == target else "DERIVED_SURROUNDING",
            "source_row_ordinal": ordinal, "source_row_sha256": _sha(_source_json(row))}
    require(target in result and (datetime.date.fromisoformat(max(result)) - datetime.date.fromisoformat(min(result))).days < 370,
            "REFERENCES_CALENDAR_TARGET")
    return result


def _source_json(value):
    """Sorted source JSON for the exact integer/string subset of serde_json.

    Nonintegral or lexically floating numbers require serde_json's f64/ryu
    contract, which is not the archive Decimal normalizer. Reject that branch
    explicitly rather than changing original source document identities.
    """
    if type(value) is dict:
        return b"{" + b",".join(audit.canonical(k) + b":" + _source_json(value[k])
            for k in sorted(value, key=lambda k: k.encode("utf-8"))) + b"}"
    if type(value) is list:
        return b"[" + b",".join(_source_json(v) for v in value) + b"]"
    if type(value) in (Decimal, float):
        raise ReferenceError("REFERENCES_SOURCE_NUMBER_UNIMPLEMENTED")
    if type(value) is int:
        require(-(1 << 63) <= value <= (1 << 64) - 1, "REFERENCES_SOURCE_NUMBER_UNIMPLEMENTED")
    return audit.canonical(value)


def _normalized_document(value, actual_keys):
    document = _object(value, _DOCUMENT)
    require(document["normalizer_version"] == "daily-raw-reconstruction-v1"
            and document["kind"] in ("bars", "reference", "calendar", "corporate_actions")
            and 0 < _integer(document["size_bytes"], 67108864), "REFERENCES_NORMALIZATION")
    _date(document["session_date"])
    _integer(document["row_count"])
    _hash(document["sha256"])
    require(type(document["source_files"]) is list and 0 < len(document["source_files"]) <= 262144,
            "REFERENCES_NORMALIZATION")
    keys = [_object(value, _KEY) for value in document["source_files"]]
    identities = [tuple(key[k] for k in _KEY) for key in keys]
    require(identities == sorted(set(identities)) and all(k in actual_keys for k in identities),
            "REFERENCES_NORMALIZATION_SOURCE")
    source = {"schema": "owner-beta-daily-normalized-source-v1", "kind": document["kind"],
              "session_date": document["session_date"], "source_files": keys}
    require(document["source_evidence_sha256"] == _sha(_wire(source)), "REFERENCES_NORMALIZATION_SOURCE")
    document["source_files"] = keys
    return document


def _calendar_document(protected, calendar, raw):
    """Reconstruct calendar.rs VerifiedCalendarProjection::document exactly.

    The descriptor comes from protected provider bytes and the resolved original
    request/copy lineage. Neither the adjacent claim nor a supplied descriptor
    chooses the row, evidence kind, source commitment, hash or byte count.
    """
    file = _file_evidence(calendar["source_file"], raw)
    require(file["kind"] == "calendar", "REFERENCES_DAILY_CALENDAR")
    terminal = file["copied_from"][-1] if file["copied_from"] else file
    target = terminal["batch_date"]
    require(dict(file["request"]["query"]) == {"BASS_DT": target.replace("-", ""), "CTX_AREA_FK": "", "CTX_AREA_NK": ""}
            and dict(file["request"]["headers"])["tr_cont"] == "", "REFERENCES_CALENDAR_REQUEST")
    legacy = calendar["legacy_projection"]
    if legacy is not None:
        legacy = _object(legacy, ("source", "source_version", "source_batch_id", "source_file_name", "document_sha256", "retrieved_at"))
        require(legacy["source"] == terminal["provider"] == "kis"
                and legacy["source_version"] == "kis-chk-holiday-v1:schema-1"
                and legacy["source_file_name"] == "calendar.json"
                and legacy["source_batch_id"] == terminal["batch_id"]
                and legacy["retrieved_at"] == terminal["retrieved_at"], "REFERENCES_CALENDAR_LEGACY")
        _hash(legacy["document_sha256"])
    original = protected.read(raw.reference(file)["path"], 67108864)
    require(len(original) == file["size_bytes"] and _sha(original) == file["wire_sha256"],
            "REFERENCES_CALENDAR_ORIGINAL")
    days = _calendar_provider_days(_raw_json(original, 67108864), target)
    date = _date(calendar["civil_date"])
    require(date in days and days[date]["session_type"] == calendar["session_type"], "REFERENCES_DAILY_CALENDAR")
    key = {name: file[name] for name in _KEY}
    commitment = {"schema": "owner-beta-daily-normalized-source-v1", "kind": "calendar",
                  "session_date": date, "source_files": [key]}
    source_sha = _sha(_wire(commitment))
    document = {"schema": "owner-beta-daily-reconstructed-document-v1", "normalizer_version": "daily-raw-reconstruction-v1",
        "kind": "calendar", "session_date": date, "source_evidence_sha256": source_sha,
        "rows": [{"date": date, "session_type": days[date]["session_type"], "evidence_kind": days[date]["evidence_kind"]}]}
    # This closed document contains strings only. Preserve the existing stricter
    # source-row numeric checks above; no f64/ryu approximation is introduced.
    encoded = _source_json(document)
    descriptor = {"kind": "calendar", "session_date": date, "normalizer_version": "daily-raw-reconstruction-v1",
        "sha256": _sha(encoded), "size_bytes": len(encoded), "row_count": 1,
        "source_files": [key], "source_evidence_sha256": source_sha}
    require(calendar["normalized_document_sha256"] == descriptor["sha256"], "REFERENCES_CALENDAR_DOCUMENT")
    return encoded, descriptor


def _daily_nested(protected, prefix, manifest, admission, raw):
    """Original-backed nested evidence and proof links; not full reconstruction.

    In particular KSD economic normalization, historical prefix reconstruction,
    Curated and legacy source readers remain required by the final closed gate.
    """
    sessions = manifest.get("daily_sources")
    require(type(sessions) is list and 0 < len(sessions) <= 20000, "REFERENCES_DAILY_SESSIONS")
    proof_raw = protected.read(prefix + "/normalization.json", 16777216, (10003, 10001, 0o440))
    proof = _object(_json(proof_raw, 16777216), ("schema", "normalizer_version", "evidence_sha256", "documents"))
    require(proof["schema"] == "owner-beta-daily-etf11-normalization-v1"
            and proof["normalizer_version"] == admission.get("normalizer_version") == "daily-raw-reconstruction-v1"
            and _wire(proof) == proof_raw, "REFERENCES_NORMALIZATION")
    actual, documents, dates = {}, {}, []
    counts = dict(instrument_count=11, session_count=len(manifest["sessions"]),
        bar_count=len(manifest["sessions"]) * 11, added_session_count=len(sessions), source_file_count=0,
        calendar_row_count=0, action_chain_count=0, action_page_count=0, cash_dividend_row_count=0,
        normalized_document_count=0)
    previous = "2026-08-28"
    for value in sessions:
        session = _object(value, ("session_date", "previous_frontier", "instruments", "bars", "reference", "calendar", "actions", "normalized_documents"))
        date = _date(session["session_date"])
        require(date > previous and session["previous_frontier"] == previous
                and session["instruments"] == manifest["instruments"], "REFERENCES_DAILY_SESSIONS")
        previous = date
        dates.append(date)
        occurrences = []
        for kind in ("bars", "reference"):
            require(type(session[kind]) is list and len(session[kind]) == 11, "REFERENCES_DAILY_SESSIONS")
            for file in session[kind]:
                require(file.get("kind") == kind, "REFERENCES_DAILY_SESSIONS")
                occurrences.append(file)
        require(type(session["calendar"]) is list and 0 < len(session["calendar"]) <= 370,
                "REFERENCES_DAILY_CALENDAR")
        calendar_dates, calendar_documents = [], {}
        for claim in session["calendar"]:
            calendar = _object(claim, ("civil_date", "session_type", "exchange", "timezone", "source_file",
                                       "normalized_document_sha256", "legacy_projection"))
            require(calendar["exchange"] == "KRX" and calendar["timezone"] == "Asia/Seoul"
                    and calendar["session_type"] in ("TRADING", "CLOSED"), "REFERENCES_DAILY_CALENDAR")
            _, descriptor = _calendar_document(protected, calendar, raw)
            calendar_documents[calendar["civil_date"]] = descriptor
            calendar_dates.append(calendar["civil_date"])
            occurrences.append(calendar["source_file"])
        require(calendar_dates == sorted(set(calendar_dates)), "REFERENCES_DAILY_CALENDAR")
        expected_dates = []
        cursor = datetime.date.fromisoformat(session["previous_frontier"]) + datetime.timedelta(days=1)
        end = datetime.date.fromisoformat(date)
        require((end - cursor).days < 370, "REFERENCES_DAILY_CALENDAR")
        while cursor <= end:
            expected_dates.append(cursor.isoformat())
            cursor += datetime.timedelta(days=1)
        require(calendar_dates == expected_dates and all(value["session_type"] ==
                ("TRADING" if value["civil_date"] == date else "CLOSED") for value in session["calendar"]),
                "REFERENCES_DAILY_CALENDAR")
        counts["calendar_row_count"] += len(calendar_dates)
        require(type(session["actions"]) is list and 0 < len(session["actions"]) <= 32768, "REFERENCES_DAILY_ACTIONS")
        counts["action_chain_count"] += len(session["actions"])
        for claim in session["actions"]:
            action = _object(claim, ("logical_class", "scope", "symbol", "range_start", "range_end", "pages",
                                     "cash_dividends", "cash_dividend_rows_sha256"))
            require(type(action["pages"]) is list and 0 < len(action["pages"]) <= 10
                    and type(action["cash_dividends"]) is list and len(action["cash_dividends"]) <= 100000,
                    "REFERENCES_DAILY_ACTIONS")
            for position, file in enumerate(action["pages"]):
                require(file.get("kind") == "corporate_actions"
                        and file.get("response_continuation") == ("M" if position + 1 < len(action["pages"]) else file.get("response_continuation"))
                        and (position + 1 < len(action["pages"]) or file.get("response_continuation") != "M"),
                        "REFERENCES_DAILY_ACTIONS")
                occurrences.append(file)
            counts["action_page_count"] += len(action["pages"])
            counts["cash_dividend_row_count"] += len(action["cash_dividends"])
        require(len(occurrences) <= 262144, "REFERENCES_DAILY_SESSIONS")
        session_keys = set()
        by_kind = {}
        for claim in occurrences:
            file = _file_evidence(claim, raw)
            require(_wire(file) == _wire(claim), "REFERENCES_DAILY_CANONICAL")
            identity = tuple(file[k] for k in _KEY)
            prior = actual.get(identity[:4])
            require(prior is None or prior == file, "REFERENCES_DAILY_ORIGINAL")
            if prior is None:
                raw.budget.add(file)
                actual[identity[:4]] = file
            session_keys.add(identity)
            by_kind.setdefault(file["kind"], set()).add(identity)
        require(type(session["normalized_documents"]) is list
                and len(session["normalized_documents"]) == len(calendar_dates) + 3, "REFERENCES_NORMALIZATION_DOCUMENTS")
        for value in session["normalized_documents"]:
            document = _normalized_document(value, session_keys)
            key = (document["session_date"], document["kind"])
            require(key not in documents, "REFERENCES_NORMALIZATION_DUPLICATE")
            keys = {tuple(file[k] for k in _KEY) for file in document["source_files"]}
            if document["kind"] == "calendar":
                claims = [claim for claim in session["calendar"] if claim["civil_date"] == document["session_date"]]
                require(len(claims) == 1 and document["sha256"] == claims[0]["normalized_document_sha256"]
                        and document["row_count"] == 1
                        and keys == {tuple(claims[0]["source_file"][k] for k in _KEY)}, "REFERENCES_NORMALIZATION_DOCUMENTS")
                require(document == calendar_documents[document["session_date"]], "REFERENCES_CALENDAR_DOCUMENT")
            else:
                count = sum(len(action["cash_dividends"]) for action in session["actions"]) if document["kind"] == "corporate_actions" else 11
                require(document["session_date"] == date and document["row_count"] == count
                        and keys == by_kind.get(document["kind"]), "REFERENCES_NORMALIZATION_DOCUMENTS")
            raw.budget.add(document)
            documents[key] = document
    require(dates == [date for date in manifest["sessions"] if date > "2026-08-28"], "REFERENCES_DAILY_SESSIONS")
    expected = [documents[k] for k in sorted(documents)]
    require(proof["documents"] == expected and manifest["normalization"]["row_count"] == len(expected),
            "REFERENCES_NORMALIZATION_DOCUMENTS")
    evidence = {"schema": "owner-beta-daily-etf11-evidence-v1", "daily_sources": sessions}
    require(_sha(_wire(evidence)) == proof["evidence_sha256"] == admission.get("evidence_sha256"),
            "REFERENCES_NORMALIZATION_EVIDENCE")
    counts["source_file_count"], counts["normalized_document_count"] = len(actual), len(documents)
    require(admission.get("counts") == counts, "REFERENCES_DAILY_COUNTS")


def _calendar_sources(protected, refs, raw):
    sources = refs.keyed("owner_beta_daily_calendar_sources", "calendar_evidence_sha256")
    claimed = {}
    for row in refs.rows("owner_beta_daily_calendar_days"):
        identity = row["calendar_evidence_sha256"]
        source = sources.get(identity)
        require(source is not None and source["owner_user_id"] == row["owner_user_id"], "REFERENCES_CALENDAR_ORPHAN")
        days = claimed.setdefault(identity, {})
        require(row["session_date"] not in days, "REFERENCES_CALENDAR_CONFLICT")
        days[row["session_date"]] = {k: row[k] for k in ("session_date", "session_type", "evidence_kind", "source_row_ordinal", "source_row_sha256")}
    for key, source in sources.items():
        file = _file_evidence(source["evidence_json"], raw)
        require(file["kind"] == "calendar" and source["normalizer_version"] == "daily-raw-reconstruction-v1", "REFERENCES_CALENDAR_SOURCE")
        target = file["copied_from"][-1]["batch_date"] if file["copied_from"] else file["batch_date"]
        query = {"BASS_DT": target.replace("-", ""), "CTX_AREA_FK": "", "CTX_AREA_NK": ""}
        require(dict(file["request"]["query"]) == query and dict(file["request"]["headers"])["tr_cont"] == "", "REFERENCES_CALENDAR_REQUEST")
        pairs = (("source_provider", "provider"), ("source_market", "market"), ("source_batch_id", "batch_id"),
                 ("source_batch_date", "batch_date"), ("source_file_name", "file_name"),
                 ("source_file_sha256", "wire_sha256"), ("source_manifest_line_sha256", "manifest_line_sha256"))
        terminal = file["copied_from"][-1] if file["copied_from"] else file
        # SQL0060 inserts the terminal origin returned by daily_0060_file,
        # while evidence_json and copy_chain retain the complete delivery.
        require(all(source[a] == terminal[b] for a, b in pairs) and source["requested_target"] == target
                and _utc9(source["source_retrieved_at"]) == _pg_microsecond(terminal["retrieved_at"])
                and source["copy_chain"] == file["copied_from"], "REFERENCES_CALENDAR_SOURCE")
        preimage = ("owner-beta-daily-calendar-source-v1\n" + source["owner_user_id"] + "\n"
                    + source["policy_sha256"] + "\ndaily-raw-reconstruction-v1\n").encode("ascii") + _wire(file)
        require(_sha(preimage) == key, "REFERENCES_CALENDAR_IDENTITY")
        original = raw.reference(file)
        document = _raw_json(protected.read(original["path"], 67108864), 67108864)
        expected = _calendar_provider_days(document, target)
        require(claimed.get(key) == expected, "REFERENCES_CALENDAR_DAYS")
    return sources


def _pg_microsecond(value):
    """SQL timestamptz's microsecond projection; retain the D4 UTC9 original.

    Reproduce PostgreSQL ParseFractionalSecond's fractional double -> rint path,
    including half-microsecond ties, separately from exact UTC9 source hashing.
    No rounded projection changes the original file bytes or completion preimage.
    """
    exact = _utc9(value)
    micros = round(float("0." + exact[20:29]) * 1000000)
    moment = datetime.datetime.fromisoformat(exact[:19] + "+00:00")
    moment += datetime.timedelta(microseconds=micros)
    return moment.strftime("%Y-%m-%dT%H:%M:%S.%f") + "000Z"


def _pg_jsonb(value):
    """Bounded SQL0060 completion identity, never the archive hash format."""
    output = bytearray()
    maximum = 2 * 16777216
    def append(data):
        require(len(data) <= maximum - len(output), "REFERENCES_COMPLETION_LIMIT")
        output.extend(data)
    def emit(item, depth=0):
        require(depth <= 64, "REFERENCES_COMPLETION_LIMIT")
        if type(item) is dict:
            append(b"{")
            for index, key in enumerate(sorted(item, key=lambda k: (len(k.encode("utf-8")), k.encode("utf-8")))):
                if index: append(b", ")
                append(audit.canonical(key))
                append(b": ")
                emit(item[key], depth + 1)
            append(b"}")
        elif type(item) is list:
            append(b"[")
            for index, child in enumerate(item):
                if index: append(b", ")
                emit(child, depth + 1)
            append(b"]")
        elif type(item) is Decimal:
            require(item.is_finite() and abs(item.adjusted()) <= 131072, "REFERENCES_NUMBER")
            size = max(1, item.adjusted() + 1) + max(0, -item.as_tuple().exponent) + 2
            require(size <= maximum - len(output), "REFERENCES_COMPLETION_LIMIT")
            append(format(abs(item) if not item else item, "f").encode("ascii"))
        else:
            append(audit.canonical(item))
    emit(value)
    return bytes(output)


def _retained_file_evidence(raw, key):
    """verify.rs resolve_original_with/file_origin_with over the retained index.

    This is a private reconstruction of the existing D4 wire, not a new original
    or an authority constructor. Every field comes from _scan_raw's checked
    manifest/batch bytes; its first equivalent manifest line remains significant.
    """
    origins, seen, rights = [], set(), None
    for _ in range(17):
        require(key not in seen and key in raw.files, "REFERENCES_DAILY_COPY")
        seen.add(key)
        source = raw.files[key]
        item = source["file"]
        if origins:
            require(source["entitlement_reference"] == rights, "REFERENCES_DAILY_COPY")
        else:
            rights = source["entitlement_reference"]
            require(type(rights) is str and rights, "REFERENCES_DAILY_COPY")
        origin = dict(provider=key[0], market=key[1], batch_id=key[2], batch_date=source["date"],
            batch_json_sha256=source["batch_sha256"], manifest_line_sha256=source["line_sha256"],
            file_name=key[3], kind=item["kind"], wire_sha256=key[4], size_bytes=item["size_bytes"],
            request=_request(item["request"], item["kind"]), response_continuation=item.get("response_continuation"),
            retrieved_at=_utc9(source["retrieved_at"]), retrieval_precision="batch")
        origins.append(origin)
        copied = item.get("copied_from")
        if copied is None:
            break
        key = _identity_reference(copied, "content_hash")
    else:
        raise ReferenceError("REFERENCES_DAILY_COPY")
    file = dict(origins[0], original_retrieved_at=origins[-1]["retrieved_at"],
                original_retrieval_precision="batch", copied_from=origins[1:])
    return _file_evidence(file, raw)


def _poll_calendar_originals(protected, raw, wanted):
    """Recover unpublished calendar proposals without requiring a DB source row.

    daily_ingest::calendars resolves the committed Raw files, then calendar.rs
    constructs a D4 file and date-sorted days in memory before its SQL call.
    The retained Raw metadata/content is the original for this reconstruction.
    Only requested observations are retained, charged to the shared budget.
    """
    require(protected is not None and raw is not None, "REFERENCES_POLL_ORIGINAL_MISSING")
    result = {}
    for key, original in raw.files.items():
        item = original["file"]
        if item["kind"] != "calendar" or key[0] not in ("kis", "kis-calendar") or item["request"]["mode"] != "credentialed":
            continue
        file = _retained_file_evidence(raw, key)
        observation = _sha(_OBSERVATION_PREFIX + b"[" + _wire(file) + b"]")
        if observation not in wanted:
            continue
        terminal = file["copied_from"][-1] if file["copied_from"] else file
        target = terminal["batch_date"]
        require(dict(file["request"]["query"]) == {"BASS_DT": target.replace("-", ""), "CTX_AREA_FK": "", "CTX_AREA_NK": ""}
                and dict(file["request"]["headers"])["tr_cont"] == "", "REFERENCES_CALENDAR_REQUEST")
        body = protected.read(original["path"], 67108864)
        require(len(body) == file["size_bytes"] and _sha(body) == file["wire_sha256"], "REFERENCES_CALENDAR_ORIGINAL")
        days = _calendar_provider_days(_raw_json(body, 67108864), target)
        require(observation not in result, "REFERENCES_AMBIGUOUS")
        raw.budget.add([observation, file, days])
        result[observation] = (file, days)
    return result


def _poll_calendar(row, event, candidates, sources, refs):
    proposal = candidates.get(row["source_frontier_sha256"])
    require(proposal is not None, "REFERENCES_POLL_CALENDAR_ORIGINAL_MISSING")
    file, days = proposal
    identity = _sha(("owner-beta-daily-calendar-source-v1\n" + row["owner_user_id"] + "\n"
        + row["policy_sha256"] + "\ndaily-raw-reconstruction-v1\n").encode("ascii") + _wire(file))
    binding = _sha(b"sql-calendar-v1" + _pg_jsonb([identity, file, [days[k] for k in sorted(days)]]))
    require(row["completion_binding_sha256"] == binding
            and event["source_kind"] == "CALENDAR" and event["range_start"] == min(days)
            and event["range_end"] == max(days) and file["retrieved_at"] <= _utc9(event["recorded_at"]),
            "REFERENCES_POLL_BINDING")
    prior = {key: source for key, source in sources.items() if source["owner_user_id"] == row["owner_user_id"]
             and _utc9(source["projected_at"]) <= _utc9(event["recorded_at"])}
    require(identity not in prior, "REFERENCES_POLL_CALENDAR_REPLAY")
    conflict = any(day["calendar_evidence_sha256"] in prior and day["session_date"] in days
        and day["session_type"] != days[day["session_date"]]["session_type"]
        for day in refs.rows("owner_beta_daily_calendar_days"))
    if event["code"] == "CALENDAR_CONFLICT":
        require(conflict, "REFERENCES_POLL_CALENDAR_CAUSE")
        return
    require(event["code"] == "SEALED_SOURCE_CHANGED" and not conflict, "REFERENCES_POLL_CALENDAR_CAUSE")
    terminal = file["copied_from"][-1] if file["copied_from"] else {k: file[k] for k in _ORIGIN}
    for source in prior.values():
        old = source["evidence_json"]
        old = old["copied_from"][-1] if old["copied_from"] else {k: old[k] for k in _ORIGIN}
        if source["policy_sha256"] == row["policy_sha256"] and all(old[k] == terminal[k]
                for k in ("provider", "market", "batch_id", "file_name")) and old != terminal:
            return
    # A changed Raw path does not retain its replaced metadata/content. Never
    # manufacture the old original from the DB JSON to make this branch pass.
    raise ReferenceError("REFERENCES_POLL_CALENDAR_CAUSE")


def _poll_admission(row, event, frontiers, originals, protected, admissions, events, admission_events, refs):
    frontier = frontiers.get(row["source_frontier_sha256"])
    require(frontier is not None and protected is not None and originals is not None, "REFERENCES_POLL_ADMISSION")
    key = frontier["admission_sha256"]
    original = originals.get(key)
    require(original is not None and frontier["owner"] == row["owner_user_id"]
            and frontier["policy_sha256"] == row["policy_sha256"], "REFERENCES_POLL_ADMISSION")
    # These maps are only the retained traversal cache. Reopen the exact original
    # names through the protected descriptor before using their SQL transport.
    prefix = "daily/admitted/" + _hash(key)[7:]
    admission_bytes = protected.read(prefix + "/admission.json", 1048576, (10003, 10001, 0o440))
    manifest_bytes = protected.read(prefix + "/manifest.json", 16777216, (10003, 10001, 0o440))
    require(_sha(admission_bytes) == key and admission_bytes == _wire(original["admission"])
            and manifest_bytes == _wire(original["manifest"])
            and _sha(manifest_bytes) == original["admission"]["manifest_sha256"], "REFERENCES_POLL_ORIGINAL")
    transport = dict(admission_sha256=key, source_frontier_sha256=row["source_frontier_sha256"], **original)
    require(row["completion_binding_sha256"] == _sha(b"sql-admission-v1" + _pg_jsonb(transport)), "REFERENCES_POLL_BINDING")
    stop = original["manifest"]["range_end"]
    cutoff = _utc9(row["completed_at"] if event is None else event["recorded_at"])
    require(frontier["range_end"] == stop and original["admission"]["policy_sha256"] == row["policy_sha256"],
            "REFERENCES_POLL_ADMISSION")
    parent_key = original["admission"]["parent_admission_sha256"]
    previous = "2026-08-28"
    if parent_key is not None:
        parent = admissions.get(parent_key)
        require(parent is not None and parent["owner_user_id"] == row["owner_user_id"]
                and parent["policy_sha256"] == row["policy_sha256"]
                and _utc9(parent["admitted_at"]) <= cutoff, "REFERENCES_POLL_ADMISSION")
        previous = parent["range_end"]
    sources = refs.keyed("owner_beta_daily_calendar_sources", "calendar_evidence_sha256")
    facts = set()
    for day in refs.rows("owner_beta_daily_calendar_days"):
        source = sources.get(day["calendar_evidence_sha256"])
        if source is not None and source["owner_user_id"] == row["owner_user_id"] == day["owner_user_id"] \
                and source["policy_sha256"] == row["policy_sha256"] \
                and _utc9(source["projected_at"]) <= cutoff:
            facts.add((day["session_date"], day["session_type"]))
    cursor, end = datetime.date.fromisoformat(previous), datetime.date.fromisoformat(stop)
    require(0 < (end - cursor).days <= 370, "REFERENCES_POLL_CALENDAR_EXTENSION")
    while cursor < end:
        cursor += datetime.timedelta(days=1)
        require((cursor.isoformat(), "TRADING" if cursor == end else "CLOSED") in facts, "REFERENCES_POLL_CALENDAR_EXTENSION")
    prior = [a for a in admissions.values() if a["owner_user_id"] == row["owner_user_id"]
             and a["range_end"] == stop and _utc9(a["admitted_at"]) <= cutoff]
    require(len(prior) <= 1, "REFERENCES_POLL_ADMISSION")
    existing = prior[0] if prior else None
    if row["result_code"] == "VERIFIED":
        require(event is None and row["admission_sha256"] == key and existing is not None
                and existing["evidence_json"] == transport and existing["policy_sha256"] == row["policy_sha256"],
                "REFERENCES_POLL_ADMISSION")
    else:
        require(event is not None and event["source_kind"] == "BAR" and event["range_start"] == stop == event["range_end"],
                "REFERENCES_POLL_EVENT")
        changed = existing is not None and (existing["admission_sha256"] != key or existing["evidence_json"] != transport)
        if event["code"] == "SEALED_SOURCE_CHANGED":
            require(changed, "REFERENCES_POLL_ADMISSION_CAUSE")
            return
        require(event["code"] == "SOURCE_CONFLICT" and not changed, "REFERENCES_POLL_ADMISSION_CAUSE")
    # The newly inserted event cannot justify its own SOURCE_CONFLICT. Existing
    # overlapping source history or a pre-existing ancestor revocation/conflict
    # must have blocked the proposal; later history cannot rewrite this receipt.
    excluded = None if event is None else event["event_sha256"]
    blocked = any(e["event_sha256"] != excluded and e["owner_user_id"] == row["owner_user_id"]
           and e["range_start"] <= stop and e["range_end"] >= "2016-08-29"
           and _utc9(e["recorded_at"]) <= cutoff for e in events.values())
    chain = [parent_key]
    if existing is not None:
        chain.append(existing["admission_sha256"])
    for ancestor in chain:
        seen = set()
        while ancestor is not None:
            require(ancestor in admissions and ancestor not in seen, "REFERENCES_POLL_ADMISSION")
            seen.add(ancestor)
            a = admissions[ancestor]
            require(a["owner_user_id"] == row["owner_user_id"] and a["policy_sha256"] == row["policy_sha256"], "REFERENCES_POLL_ADMISSION")
            blocked |= any(e["admission_sha256"] == ancestor and e["evidence_sha256"] != excluded
                and _utc9(e["created_at"]) <= cutoff for e in admission_events)
            ancestor = a["parent_admission_sha256"]
    require(blocked == (row["result_code"] == "INPUT_BLOCKED"), "REFERENCES_POLL_ADMISSION_CAUSE")


def _daily_events_receipts(refs, frontiers, *, protected=None, raw=None, originals=None):
    admissions = refs.keyed("owner_beta_daily_admissions", "admission_sha256")
    events = refs.keyed("owner_beta_daily_source_events", "event_sha256")
    for key, event in events.items():
        require(type(event["owner_user_id"]) is str and UUID.fullmatch(event["owner_user_id"])
                and event["source_kind"] in ("ACTION", "CALENDAR", "BAR", "REFERENCE")
                and event["code"] in ("SOURCE_CONFLICT", "SEALED_SOURCE_CHANGED", "SOURCE_REVOKED",
                    "SOURCE_INTEGRITY", "CALENDAR_CONFLICT", "NONCASH_ACTION")
                and _date(event["range_start"]) <= _date(event["range_end"]), "REFERENCES_SOURCE_EVENT")
        _hash(event["evidence_sha256"])
        _utc9(event["recorded_at"])
        fields = (event[k] for k in ("owner_user_id", "source_kind", "code", "range_start", "range_end", "evidence_sha256"))
        require(_sha(("owner-beta-daily-source-event-v1\n" + "\n".join(fields) + "\n").encode("ascii")) == key,
                "REFERENCES_SOURCE_EVENT")
        if event["admission_sha256"] is not None:
            admission = admissions.get(event["admission_sha256"])
            require(admission is not None and admission["owner_user_id"] == event["owner_user_id"]
                    and admission["range_start"] <= event["range_end"] and admission["range_end"] >= event["range_start"],
                    "REFERENCES_SOURCE_EVENT")
    admission_events = list(refs.rows("owner_beta_daily_admission_events"))
    for row in admission_events:
        _utc9(row["created_at"])
        _hash(row["evidence_sha256"])
        admission = admissions.get(row["admission_sha256"])
        require(admission is not None and admission["owner_user_id"] == row["owner_user_id"], "REFERENCES_ADMISSION_EVENT")
        require(row["kind"] in ("CONFLICT", "REVOKED"), "REFERENCES_ADMISSION_EVENT")
        preimage = "owner-beta-daily-admission-event-v1\n" + row["kind"] + "\n" + row["admission_sha256"] + "\n" + row["evidence_sha256"] + "\n"
        require(_sha(preimage.encode("ascii")) == row["event_sha256"], "REFERENCES_ADMISSION_EVENT")
        if row["kind"] == "CONFLICT":
            cause = events.get(row["evidence_sha256"])
            require(cause is not None and cause["owner_user_id"] == row["owner_user_id"]
                    and admission["range_start"] <= cause["range_end"] and admission["range_end"] >= cause["range_start"],
                    "REFERENCES_ADMISSION_EVENT")
    receipts = refs.keyed("owner_beta_daily_poll_receipts", "receipt_id")
    calendars = None
    for row in receipts.values():
        require(row["stage"] in ("ADMISSION", "SCHEDULE")
                and (row["completed_at"] is None) == (row["result_code"] is None), "REFERENCES_POLL_RECEIPT")
        require(all(type(row[k]) is str and UUID.fullmatch(row[k]) for k in ("receipt_id", "owner_user_id")),
                "REFERENCES_POLL_RECEIPT")
        _hash(row["policy_sha256"])
        _integer(row["activation_epoch"])
        started = _utc9(row["started_at"])
        fields = ("source_frontier_sha256", "admission_sha256", "event_sha256", "completion_binding_sha256")
        for field in fields:
            if row[field] is not None:
                _hash(row[field])
        if row["completed_at"] is None:
            require(all(row[k] is None for k in fields), "REFERENCES_POLL_RECEIPT")
            continue  # A superseded unfinished receipt remains valid history.
        require(_utc9(row["completed_at"]) >= started, "REFERENCES_POLL_RECEIPT")
        result = row["result_code"]
        if row["stage"] == "SCHEDULE":
            require(result in ("CHECKED", "DEFERRED", "BLOCKED") and all(row[k] is None for k in fields),
                    "REFERENCES_POLL_RECEIPT")
            continue
        require(result in ("VERIFIED", "INPUT_PENDING", "INPUT_BLOCKED", "IO_FAILED"), "REFERENCES_POLL_RECEIPT")
        if result == "VERIFIED":
            require(row["admission_sha256"] in admissions and row["source_frontier_sha256"] in frontiers
                    and row["event_sha256"] is None and row["completion_binding_sha256"] is not None, "REFERENCES_POLL_ADMISSION")
            _poll_admission(row, None, frontiers, originals, protected, admissions, events, admission_events, refs)
            continue
        require(row["admission_sha256"] is None, "REFERENCES_POLL_RECEIPT")
        if row["completion_binding_sha256"] is None:
            # SQL finish permits an optional diagnostic hash. It is not a typed
            # original pin; the Rust finish_failure caller presently sends NULL.
            require(row["event_sha256"] is None, "REFERENCES_POLL_RECEIPT")
            continue
        require(result == "INPUT_BLOCKED", "REFERENCES_POLL_RECEIPT")
        event = events.get(row["event_sha256"])
        require(event is not None and event["owner_user_id"] == row["owner_user_id"]
                and event["evidence_sha256"] == row["source_frontier_sha256"]
                and _utc9(event["recorded_at"]) <= _utc9(row["completed_at"]), "REFERENCES_POLL_EVENT")
        payload = {k: event[k] for k in ("source_kind", "range_start", "range_end", "code", "evidence_sha256")}
        if event["code"] != "SOURCE_REVOKED" and row["completion_binding_sha256"] == _sha(b"sql-block-v1" + _pg_jsonb(payload)):
            continue
        if event["source_kind"] == "CALENDAR" and event["code"] in ("CALENDAR_CONFLICT", "SEALED_SOURCE_CHANGED"):
            if calendars is None:
                wanted = {r["source_frontier_sha256"] for r in receipts.values() if r["completion_binding_sha256"] is not None}
                candidates = _poll_calendar_originals(protected, raw, wanted)
                sources = _calendar_sources(protected, refs, raw)
                calendars = candidates, sources
            _poll_calendar(row, event, *calendars, refs)
        elif event["source_kind"] == "BAR" and event["code"] in ("SOURCE_CONFLICT", "SEALED_SOURCE_CHANGED"):
            _poll_admission(row, event, frontiers, originals, protected, admissions, events, admission_events, refs)
        else:
            raise ReferenceError("REFERENCES_POLL_BINDING")
    states = refs.keyed("owner_beta_daily_poll_state", "stage")
    require({r["stage"] for r in receipts.values()} <= set(states), "REFERENCES_POLL_STATE")
    for state in states.values():
        require(state["stage"] in ("ADMISSION", "SCHEDULE"), "REFERENCES_POLL_STATE")
        _utc9(state["next_poll_at"])
        if state["latest_receipt_id"] is None:
            require(all(state[k] is None for k in ("last_completed_at", "last_result_code", "verified_admission_sha256", "source_frontier_sha256")),
                    "REFERENCES_POLL_STATE")
            require(not any(r["stage"] == state["stage"] for r in receipts.values()), "REFERENCES_POLL_STATE")
            continue
        receipt = receipts.get(state["latest_receipt_id"])
        require(receipt is not None and receipt["stage"] == state["stage"], "REFERENCES_POLL_STATE")
        require(all(_utc9(r["started_at"]) <= _utc9(receipt["started_at"])
                    for r in receipts.values() if r["stage"] == state["stage"]), "REFERENCES_POLL_STATE")
        due = datetime.datetime.fromisoformat(receipt["started_at"].replace("Z", "+00:00")) + datetime.timedelta(seconds=60)
        require(_utc9(state["next_poll_at"]) == _utc9(due.isoformat()), "REFERENCES_POLL_STATE")
        # Taking a new slot clears its completion projection in the same SQL
        # transaction. An unfinished latest receipt cannot retain old success.
        if receipt["completed_at"] is None:
            require(all(state[k] is None for k in ("last_completed_at", "last_result_code",
                "verified_admission_sha256", "source_frontier_sha256")), "REFERENCES_POLL_STATE")
        else:
            require(all(state[a] == receipt[b] for a, b in (("last_completed_at", "completed_at"),
                ("last_result_code", "result_code"), ("verified_admission_sha256", "admission_sha256"),
                ("source_frontier_sha256", "source_frontier_sha256"))), "REFERENCES_POLL_STATE")


def _daily_admissions(protected, refs, raw):
    """Close exact admission/manifest/bars/proof byte pins and parent graph.

    This does not replace Rust's normalization, provider reconstruction or
    complete nested Daily evidence traversal; the final closure gate stays shut.
    """
    rows = refs.keyed("owner_beta_daily_admissions", "admission_sha256")
    leaves, originals = set(), {}
    for path in protected.entries:
        if not path.startswith("daily/admitted/"):
            continue
        parts = path[len("daily/admitted/"):].split("/")
        require(re.fullmatch(r"[0-9a-f]{64}", parts[0]) and len(parts) in (1, 2)
                and (len(parts) == 1 or parts[1] in ("admission.json", "manifest.json", "bars.ndjson", "normalization.json")),
                "REFERENCES_DAILY_INVENTORY")
        leaves.add("sha256:" + parts[0])
    require(set(rows) <= leaves, "REFERENCES_DAILY_MISSING")
    for key in sorted(leaves):
        row = rows.get(key)
        prefix = "daily/admitted/" + _hash(key)[7:]
        admission_raw = protected.read(prefix + "/admission.json", 1048576, (10003, 10001, 0o440))
        manifest_raw = protected.read(prefix + "/manifest.json", 16777216, (10003, 10001, 0o440))
        require(_sha(admission_raw) == key, "REFERENCES_DAILY_PIN")
        admission, manifest = _json(admission_raw), _json(manifest_raw, 16777216)
        admission = _object(admission, ("schema", "content_sha256", "manifest_sha256", "policy_sha256", "parent_admission_sha256", "base",
            "verifier_code_commit", "normalizer_version", "evidence_sha256", "counts"))
        manifest = _object(manifest, ("schema", "capability", "audience", "vendor_snapshot", "strict_pit", "base", "parent_admission_sha256",
            "range_start", "range_end", "instruments", "sessions", "daily_sources", "bars", "normalization", "policy_sha256",
            "materializer_version", "code_commit", "content_sha256"))
        require(_wire(admission) == admission_raw and _wire(manifest) == manifest_raw
                and admission["schema"] == "owner-beta-daily-etf11-admission-v1"
                and manifest["schema"] == "owner-beta-daily-etf11-input-v1"
                and manifest["materializer_version"] == "daily-price-only-v1"
                and manifest["capability"] == "PRICE_RETURN_ONLY" and manifest["audience"] == "OWNER_ONLY"
                and manifest["vendor_snapshot"] is True and manifest["strict_pit"] is False
                and admission["base"] == manifest["base"], "REFERENCES_DAILY_CANONICAL")
        for field in ("content_sha256", "policy_sha256", "parent_admission_sha256"):
            require(admission[field] == manifest[field], "REFERENCES_DAILY_PIN")
        require(admission["manifest_sha256"] == _sha(manifest_raw), "REFERENCES_DAILY_PIN")
        if row is not None:
            require(all(row[field] == admission[field] for field in ("content_sha256", "policy_sha256", "parent_admission_sha256", "manifest_sha256"))
                    and manifest["range_start"] == row["range_start"] and manifest["range_end"] == row["range_end"], "REFERENCES_DAILY_PIN")
            evidence = row["evidence_json"]
            require(type(evidence) is dict and evidence.get("manifest") == manifest and evidence.get("admission") == admission
                    and evidence.get("admission_sha256") == key, "REFERENCES_DAILY_EVIDENCE")
        unsigned = {k: v for k, v in manifest.items() if k != "content_sha256"}
        require(_sha(_wire(unsigned)) == manifest["content_sha256"], "REFERENCES_DAILY_CONTENT")
        for field, filename in (("bars", "bars.ndjson"), ("normalization", "normalization.json")):
            descriptor = manifest.get(field)
            require(type(descriptor) is dict and set(descriptor) == {"file", "sha256", "size_bytes", "row_count"}
                    and descriptor["file"] == filename, "REFERENCES_DAILY_DESCRIPTOR")
            protected.hash_file(prefix + "/" + filename, descriptor["sha256"], descriptor["size_bytes"],
                                (10003, 10001, 0o440))
        _daily_nested(protected, prefix, manifest, admission, raw)
        raw.budget.add([admission, manifest])
        originals[key] = {"admission": admission, "manifest": manifest}
    for key, original in originals.items():
        admission, manifest = original["admission"], original["manifest"]
        parent = admission["parent_admission_sha256"]
        if parent is not None:
            ancestor = originals.get(parent)
            require(ancestor is not None and ancestor["admission"]["policy_sha256"] == admission["policy_sha256"], "REFERENCES_DAILY_PARENT")
            old = ancestor["manifest"]
            require(manifest["base"] == old["base"] and manifest["instruments"] == old["instruments"]
                    and old["range_end"] < manifest["range_end"] and len(manifest["sessions"]) == len(old["sessions"]) + 1
                    and len(manifest["daily_sources"]) == len(old["daily_sources"]) + 1
                    and manifest["sessions"][:len(old["sessions"])] == old["sessions"]
                    and manifest["daily_sources"][:len(old["daily_sources"])] == old["daily_sources"],
                    "REFERENCES_DAILY_PARENT")
            if key in rows:
                require(parent in rows and rows[parent]["owner_user_id"] == rows[key]["owner_user_id"], "REFERENCES_DAILY_PARENT")
    return originals


def _simulation_evidence(protected, refs, raw_index):
    """Read the existing R4 seven-member evidence contract from original FDs.

    This validates actual config/receipt/manifest/approval/base bytes and DB pins.
    It does not claim that an absent current R4 configuration proves no legacy
    history, or that these seven files cover every historical V2 dependency.
    """
    present = [path in protected.entries for path in simulation.ARCHIVE_EVIDENCE]
    if not any(present):
        require(not list(refs.rows("owner_beta_simulation_runs")), "REFERENCES_SIMULATION_MISSING")
        _historical_run_pins(protected, refs, None)
        return
    require(all(present), "REFERENCES_SIMULATION_MISSING")
    metadata = {
        "simulation_config/config.json": (0, 10001, 0o440),
        "simulation_receipt/artifact.json": (0, 0, 0o444),
        "simulation_receipt/COMPLETE": (0, 0, 0o444),
        "simulation_manifest/.lagrange-release-manifest": (0, 0, 0o600),
        "simulation_floor/.owner-beta-simulation-reader-floor": (0, 0, 0o600),
        "simulation_reader/owner-beta-simulation-reader-v1.json": (0, 0, 0o644),
        "simulation_approval/" + simulation.APPROVAL_NAME: (0, 0, 0o644),
    }
    evidence = {p: protected.read(p, 256 * 1024, metadata[p]) for p in simulation.ARCHIVE_EVIDENCE}
    require(evidence["simulation_floor/" + simulation.release.FLOOR_NAME] == simulation.release.FLOOR_BYTES
            and evidence["simulation_reader/owner-beta-simulation-reader-v1.json"] == simulation.release.READER_BYTES,
            "REFERENCES_SIMULATION_READER")
    approval_raw = evidence["simulation_approval/" + simulation.APPROVAL_NAME]
    approval = simulation.approved_record(approval_raw)
    config_raw = evidence["simulation_config/config.json"]
    config = simulation.daily.install.json_bytes(config_raw)
    simulation.release.validate_binding(config_raw,
        evidence["simulation_manifest/.lagrange-release-manifest"],
        evidence["simulation_receipt/artifact.json"], evidence["simulation_receipt/COMPLETE"],
        config.get("code_commit"))
    for name in ("simulation_input", "base"):
        # The codec permits these two full roots to alias, but never synthesizes
        # a second original when the caller supplied separate actual roots.
        prefix = name + "/" + approval["candidate_content_sha256"][7:]
        if name == "base" and not any(p.startswith("base/") for p in protected.entries):
            continue
        manifest_raw = protected.read(prefix + "/manifest.json", MAX_DOCUMENT, (10001, 10001, 0o600))
        manifest = simulation.daily.install.json_bytes(manifest_raw, maximum=MAX_DOCUMENT)
        require(manifest_raw == json.dumps(manifest, ensure_ascii=False, separators=(",", ":"),
                                          allow_nan=False).encode("utf-8") + b"\n",
                "REFERENCES_HISTORICAL_CANONICAL")
        require(manifest.get("candidate_content_sha256") == approval["candidate_content_sha256"]
                and manifest.get("manifest_sha256") == approval["artifact_manifest_sha256"],
                "REFERENCES_HISTORICAL_PIN")
        unsigned = {k: v for k, v in manifest.items() if k != "manifest_sha256"}
        require(_sha(json.dumps(unsigned, ensure_ascii=False, separators=(",", ":"),
                                allow_nan=False).encode("utf-8")) == approval["artifact_manifest_sha256"],
                "REFERENCES_HISTORICAL_PIN")
        protected.hash_file(prefix + "/bars.ndjson", approval["bars_sha256"], approval["bars_size_bytes"],
                            (10001, 10001, 0o600))
    pins = {key: approval[key] for key in ("candidate_content_sha256", "artifact_manifest_sha256",
            "stage5_manifest_sha256", "action_manifest_sha256", "bars_sha256")}
    pins["approval_registry_sha256"] = _sha(approval_raw)
    # An archived run binds its own immutable approval tuple. Comparing every
    # historical row with the current installed V3 tuple loses V2 history.
    _historical_run_pins(protected, refs, pins)
    for field in ("price_source", "action_source"):
        source = approval[field]
        _historical_raw_batch(raw_index, field, source["batch_id"], source["file_count"],
                              source["manifest_line_sha256"], source["batch_json_sha256"])


def _historical_raw_batch(raw_index, source_kind, batch_id, count, manifest_line, batch_hash=None):
    """Bind the adopted exact line among every retained equivalent occurrence."""
    require(source_kind in ("price_source", "action_source"), "REFERENCES_HISTORICAL_RAW")
    provider = "kis-daily-range" if source_kind == "price_source" else "kis"
    kind = "bars" if source_kind == "price_source" else "corporate_actions"
    originals = [value for key, value in raw_index.files.items()
                 if key[:3] == (provider, "kr", batch_id)]
    require(len(originals) == count and originals and all(
        value["file"]["kind"] == kind and (batch_hash is None or value["batch_sha256"] == batch_hash)
        and any(line[0] == manifest_line for line in value["manifest_lines"])
        for value in originals), "REFERENCES_HISTORICAL_RAW")
    return originals


LEGACY_APPROVAL_NAME = "kis-historical-price-only-beta-approved-artifacts.json"
LEGACY_APPROVAL_SHA256 = "sha256:4111f51d945a48a7559b22863cc4ed2eae9c760d5ac9288e554aefe5575e3380"
LEGACY_HISTORY_PREFIX = "kis-historical-price-only-beta/v2/candidate-sha256="


def _legacy_approval(data):
    # Same adopted original and source-owned constant as daily-backup.py
    # legacy_approved_record. This cannot approve a new candidate.
    require(_sha(data) == LEGACY_APPROVAL_SHA256, "REFERENCES_LEGACY_APPROVAL")
    value = _json(data, 262144)
    require(value.get("schema_id") == "kis-historical-price-only-beta-approval-registry"
            and value.get("schema_version") == 2 and type(value.get("approved_artifacts")) is list
            and len(value["approved_artifacts"]) == 1, "REFERENCES_LEGACY_APPROVAL")
    return value["approved_artifacts"][0]


def _legacy_history(protected, raw):
    """Original V2 artifact/approval/Raw links, not historical reader proof.

    The later source-selection factory must retain the real installed registry
    under simulation_approval, never manufacture it from this compiled hash.
    Stage5 normalized-session reconstruction and KSD normalization remain open.
    """
    present = any(p.startswith("simulation_input/kis-historical-price-only-beta/")
                  or p.startswith("base/kis-historical-price-only-beta/") for p in protected.entries)
    if not present:
        return  # Absence is not an empty-history proof; describe still fails.
    approval = _legacy_approval(protected.read("simulation_approval/" + LEGACY_APPROVAL_NAME,
                                              262144, (0, 0, 0o644)))
    aliases = [{p[len(prefix):]: {k: v for k, v in entry.items() if k != "path"}
                for p, entry in protected.entries.items() if p.startswith(prefix)}
               for prefix in ("base/", "simulation_input/")]
    require(aliases[0] == aliases[1], "REFERENCES_HISTORICAL_ALIAS")
    manifest = None
    for name in ("base", "simulation_input"):
        prefix = name + "/" + LEGACY_HISTORY_PREFIX + approval["candidate_content_sha256"][7:]
        data = protected.read(prefix + "/manifest.json", MAX_DOCUMENT, (10001, 10001, 0o600))
        value = _json(data)
        require(data == _wire(value) + b"\n" and value.get("candidate_content_sha256") == approval["candidate_content_sha256"]
                and value.get("manifest_sha256") == approval["artifact_manifest_sha256"], "REFERENCES_LEGACY_MANIFEST")
        unsigned = {k: v for k, v in value.items() if k != "manifest_sha256"}
        require(_sha(_wire(unsigned)) == approval["artifact_manifest_sha256"], "REFERENCES_LEGACY_MANIFEST")
        for key in ("audience", "vendor_snapshot", "strict_pit", "capability", "range_start", "range_end", "instruments",
                    "instrument_count", "session_count", "materialization_status", "registration_status", "publication_status"):
            require(value.get(key) == approval[key], "REFERENCES_LEGACY_PIN")
        require(value.get("schema_id") == approval["artifact_schema_id"]
                and value.get("schema_version") == approval["artifact_schema_version"]
                and value.get("row_count") == approval["bar_count"], "REFERENCES_LEGACY_PIN")
        bars = value.get("bars")
        require(type(bars) is dict and bars.get("relative_path") == "bars.ndjson"
                and bars.get("row_count") == approval["bar_count"], "REFERENCES_LEGACY_MANIFEST")
        protected.hash_file(prefix + "/bars.ndjson", _hash(bars.get("sha256")),
                            _integer(bars.get("size_bytes"), 67108864), (10001, 10001, 0o600))
        require(manifest is None or manifest == value, "REFERENCES_HISTORICAL_ALIAS")
        manifest = value
    stage5 = manifest["stage5"]
    require(stage5["manifest_sha256"] == approval["stage5_manifest_sha256"]
            and stage5["file_count"] == len(stage5["files"]) == 187, "REFERENCES_LEGACY_STAGE5")
    originals = {value["file"]["file_name"]: value for identity, value in raw.files.items()
                 if identity[:3] == ("kis-daily-range", "kr", stage5["batch_id"])}
    require(set(originals) == {file["file_name"] for file in stage5["files"]}, "REFERENCES_LEGACY_STAGE5")
    for file in stage5["files"]:
        original = originals[file["file_name"]]["file"]
        require(original["kind"] == file["kind"] == "bars" and original["content_hash"] == file["sha256"]
                and original["size_bytes"] == file["size_bytes"], "REFERENCES_LEGACY_STAGE5")
    ksd = manifest["ksd"]
    require(ksd["manifest_sha256"] == approval["action_manifest_sha256"], "REFERENCES_LEGACY_ACTIONS")
    for key in ("cash_dividend_treatment_id", "ignored_cash_dividend_row_count", "ignored_cash_dividend_rows_sha256",
                "ignored_cash_dividend_source_file_sha256", "ignored_cash_dividend_acquired_at"):
        require(ksd[key] == approval[key], "REFERENCES_LEGACY_ACTIONS")
    actions = [value for identity, value in raw.files.items() if identity[:3] == ("kis", "kr", ksd["batch_id"])]
    require(len(actions) == ksd["file_count"] == 7 and all(value["file"]["kind"] == "corporate_actions" for value in actions)
            and any(value["file"]["content_hash"] == ksd["ignored_cash_dividend_source_file_sha256"] for value in actions),
            "REFERENCES_LEGACY_ACTIONS")


def _daily_config(protected, refs):
    """Current nonsecret Daily config/policy pins, from existing reader rules."""
    path = "config/config.json"
    if path not in protected.entries:
        require(not list(refs.rows("owner_beta_daily_control")), "REFERENCES_DAILY_CONFIG_MISSING")
        return
    data = protected.read(path, daily.CONFIG_LIMIT, (0, 10001, 0o440))
    parsed = daily.json_bytes(data)
    config = daily.validate_config(data, parsed.get("code_commit"))
    policy_raw = protected.read("config/policy.json", daily.POLICY_LIMIT, (0, 10001, 0o440))
    policy = daily.validate_policy(policy_raw)
    require(config["policy_sha256"] == _sha(policy_raw), "REFERENCES_DAILY_CONFIG_PIN")
    for row in refs.rows("owner_beta_daily_control"):
        require(row["owner_user_id"] == config["owner"] and row["policy_sha256"] == _sha(policy_raw)
                and audit.canonical(row["policy_json"]) == audit.canonical(policy)
                and row["entitlement_reference"] == config["entitlement_reference"]
                and row["entitlement_sha256"] == config["entitlement_sha256"]
                and row["first_auto_session"] == config["first_auto_session"], "REFERENCES_DAILY_CONFIG_PIN")
    raw_root = next(root for root in protected.roots if root.name == "raw")
    require(raw_root.uid == config["raw_owner_uid"], "REFERENCES_RAW_OWNER")


def _fixed_floors(protected):
    fixed = (("floor/" + daily.FLOOR_NAME, daily.FLOOR_BYTES),
             ("paper_floor/" + paper.FLOOR_NAME, paper.FLOOR_BYTES),
             ("simulation_floor/" + simulation.release.FLOOR_NAME, simulation.release.FLOOR_BYTES))
    for path, expected in fixed:
        if path in protected.entries:
            require(protected.read(path, len(expected), (0, 0, 0o600)) == expected, "REFERENCES_FLOOR")
    # Absence is not interpreted as a valid empty optional class here.


DATASET_KEYS = frozenset(("id", "dataset_id", "version", "status", "manifest_sha256", "storage_path"))
MAX_DATASET_BYTES = 64 * 1024 * 1024
MAX_DATASET_ROWS = 1_000_000


def _dataset_selectors(references):
    selected = {}
    for table in ("pending_targets", "paper_rebalance_previews"):
        for row in references.rows(table):
            identity, manifest = row["dataset_version_id"], row["dataset_manifest_sha256"]
            version = row.get("dataset_version")
            if identity is None:
                require(manifest is None and version is None, "REFERENCES_DATASET_NULL")
                continue
            require(type(identity) is str and UUID.fullmatch(identity)
                    and type(manifest) is str and re.fullmatch(r"[0-9a-f]{64}", manifest)
                    and (version is None or type(version) is str), "REFERENCES_DATASET_IDENTITY")
            previous = selected.get(identity)
            if previous is not None:
                require(previous[0] == manifest and (previous[1] is None or version is None
                        or previous[1] == version), "REFERENCES_DATASET_CONFLICT")
                version = previous[1] if previous[1] is not None else version
            if previous is None or previous[1] != version:
                references.budget.add(["dataset_requirement", identity, manifest, version])
            selected[identity] = (manifest, version)
    return selected


def dataset_requirements(snapshot, stream):
    """Select IDs from the complete held stream; no new snapshot or SQL client."""
    protected = _Protected(snapshot)
    try:
        references = _References(protected.limits,
            json.loads(audit.PROJECTION.read_bytes())["archive_audit_v1"])
        references.decode(stream)
        selected = _dataset_selectors(references)
        protected.revalidate()
        protected.source_class.verify_unchanged(snapshot)
        return tuple(sorted(selected))
    finally:
        protected.failed = True


def _encoded_dataset_rows(rows):
    previous = None
    size = count = 0
    for row in rows:
        require(type(row) is dict and set(row) == DATASET_KEYS
                and all(type(value) is str for value in row.values()), "REFERENCES_DATASET_ROW")
        identity = row["id"]
        require(UUID.fullmatch(identity) and (previous is None or previous < identity)
                and re.fullmatch(r"[0-9a-f]{64}", row["manifest_sha256"]), "REFERENCES_DATASET_ORDER")
        raw = audit.canonical(row) + b"\n"
        require(len(raw) <= MAX_LINE and count < MAX_DATASET_ROWS
                and len(raw) <= MAX_DATASET_BYTES - size, "REFERENCES_DATASET_LIMIT")
        count += 1
        size += len(raw)
        previous = identity
        yield row, raw


def encode_dataset_rows(rows):
    """Canonical source capture, descriptive only; storage_path is never opened."""
    return b"".join(raw for _, raw in _encoded_dataset_rows(rows))


def _dataset_rows(references, rows):
    selected = _dataset_selectors(references)
    values = []
    for row, _ in _encoded_dataset_rows(rows):
        require(row["id"] in selected, "REFERENCES_DATASET_SET")
        manifest, version = selected[row["id"]]
        require(row["manifest_sha256"] == manifest and (version is None or row["version"] == version)
                and row["storage_path"] == "/data/curated", "REFERENCES_DATASET_BINDING")
        # Charge before copying; no second encoded stream/parsed history lives
        # alongside the existing index and the caller's captured source rows.
        references.budget.add(["source_dataset_row", row["id"], row])
        values.append(dict(row))
    require([row["id"] for row in values] == sorted(selected), "REFERENCES_DATASET_SET")
    return values


def _provisional_root(protected, references, dataset_rows=()):
    """Bind actual original bytes and validated DB references, never closure.

    Used for source candidates and shared vectors. A root calculated here is
    never a restore permit; native independently verifies domain dependencies
    and source/clone equality while the actual original owners stay alive.
    """
    require(references.done and not references.failed, "REFERENCES_STREAM_STATE")
    protected.revalidate()
    for path, entry in protected.entries.items():
        if entry["kind"] == "file":
            protected.hash_file(path)
    root = _FramedRoot()
    for value in files.class_records(protected.manifest_bytes, protected.limits, profile="paper-combined"):
        root.add("source_class", value["class"], value)
    for row in dataset_rows:
        root.add("source_dataset_row", row["id"], row)
    for path in sorted(protected.entries, key=lambda p: p.encode("utf-8")):
        root.add("source_member", path.encode("utf-8").hex(), protected.entries[path])
    for table, rows in sorted(references.tables.items()):
        for key, row in sorted(rows.items()):
            root.add("source_reference", table + ":" + key, row)
    protected.revalidate()
    return root




_HISTORICAL_PIN_FIELDS = ("candidate_content_sha256", "artifact_manifest_sha256",
    "stage5_manifest_sha256", "action_manifest_sha256", "approval_registry_sha256")


def _historical_run_pins(protected, refs, current):
    """Resolve every run's five pins to actual retained V2/V3 originals.

    Actual adopted registry bytes are reopened here, including when no current
    R4 configuration exists. Legacy manifests/bars/Raw
    are separately reopened by _legacy_history; this function never upgrades V2
    to today's registry or treats a rights status as current authorization.
    """
    records = {}
    v3_path = "simulation_approval/" + simulation.APPROVAL_NAME
    v3 = None
    if v3_path in protected.entries:
        original = protected.read(v3_path, 262144, (0, 0, 0o644))
        adopted = simulation.approved_record(original)
        v3 = {key: adopted[key] for key in _HISTORICAL_PIN_FIELDS[:-1]}
        v3.update(approval_registry_sha256=_sha(original), bars_sha256=adopted["bars_sha256"])
        records[v3["approval_registry_sha256"]] = v3
    if current is not None:
        require(v3 == current, "REFERENCES_HISTORICAL_RUN_PIN")
    legacy_path = "simulation_approval/" + LEGACY_APPROVAL_NAME
    if legacy_path in protected.entries:
        original = protected.read(legacy_path, 262144, (0, 0, 0o644))
        legacy = _legacy_approval(original)
        pin = {key: legacy[key] for key in _HISTORICAL_PIN_FIELDS[:-1]}
        pin["approval_registry_sha256"] = _sha(original)
        records[pin["approval_registry_sha256"]] = pin
    for table in ("owner_beta_simulation_runs", "owner_beta_recommendation_runs"):
        for row in refs.rows(table):
            selected = records.get(row["approval_registry_sha256"])
            require(selected is not None and all(row[key] == selected[key]
                    for key in _HISTORICAL_PIN_FIELDS), "REFERENCES_HISTORICAL_RUN_PIN")
            if table == "owner_beta_simulation_runs":
                require(selected is v3 and row["bars_sha256"] == v3["bars_sha256"],
                        "REFERENCES_SIMULATION_PIN")
            job = refs.jobs.get(row["job_id"])
            require(job is not None and job["owner_user_id"] == row["owner_user_id"],
                    "REFERENCES_HISTORY_JOB")
            if table == "owner_beta_simulation_runs" and row["published_attempt_id"] is not None:
                attempt = refs.attempts.get(row["published_attempt_id"])
                require(attempt is not None and attempt["job_id"] == row["job_id"],
                        "REFERENCES_HISTORY_ATTEMPT")


def _journal_children(protected, prefix):
    """A finite already-validated manifest inventory; never a host path scan."""
    return {path[len(prefix) + 1:] for path in protected.entries
            if path.startswith(prefix + "/") and "/" not in path[len(prefix) + 1:]}


def _journal_bytes(protected, path, maximum=journal.MAX_NATIVE_INDEX, mode=0o400):
    raw = protected.read(path, maximum, (0, 0, mode))
    try:
        value = journal._json(raw, maximum)
    except (journal.JournalError, UnicodeError, ValueError, TypeError):
        raise ReferenceError("REFERENCES_JOURNAL_WIRE") from None
    return raw, value


def _journal_validate(function, *args):
    try:
        return function(*args)
    except (journal.JournalError, UnicodeError, ValueError, TypeError, KeyError, OverflowError):
        raise ReferenceError("REFERENCES_JOURNAL_BINDING") from None


def _journal_hash_copy(protected, prefix, original, maximum=journal.MAX_NATIVE_INDEX):
    copy = protected.read(prefix + "/" + _sha(original)[7:] + ".json", maximum, (0, 0, 0o400))
    require(copy == original, "REFERENCES_JOURNAL_ORIGINAL")


def _journal_stage_epoch(intent, stage, previous):
    # Source: journal.Operation._validate_bound_stage; no Operation/custody fake.
    allowed = {
        "STOP": {"INTENT", "FENCED", "STOPPED", "RECONCILED", "OFF_COMMITTED", "FAILED"},
        "BACKUP": {"INTENT", "FENCED", "STOPPED", "RECONCILED", "OFF_COMMITTED", "SNAPSHOT", "FAILED"},
        "RESTORE": {"INTENT", "FENCED", "STOPPED", "RECONCILED", "RESTORE_VERIFIED", "RESTORED_COMMITTED", "FAILED"},
    }.get(intent["purpose"])
    require(allowed is not None and stage["stage"] in allowed, "REFERENCES_JOURNAL_STAGE")
    if stage["stage"] == "FAILED":
        require(previous is not None and stage["control_epoch"] == previous["control_epoch"],
                "REFERENCES_JOURNAL_EPOCH")
        return
    fence = max(intent["expected_epoch"], intent["local_epoch_floor"]) + 1
    expected = intent["expected_epoch"] if stage["stage"] == "INTENT" else fence
    if stage["stage"] in ("OFF_COMMITTED", "SNAPSHOT", "RESTORED_COMMITTED"):
        expected = fence + 1
    require(expected <= I64 and stage["control_epoch"] == expected, "REFERENCES_JOURNAL_EPOCH")


def _journal_action(protected, prefix, intent, root_raw, ordinal, previous_digest, objects, budget):
    require(protected.entries.get(prefix, {}).get("kind") == "directory"
            and protected.entries.get(prefix + "/output", {}).get("kind") == "directory",
            "REFERENCES_JOURNAL_INVENTORY")
    names = _journal_children(protected, prefix)
    raw, _ = _journal_bytes(protected, prefix + "/binding.json", journal.MAX_SMALL)
    action = _journal_validate(journal._validate_action, raw, intent, ordinal, previous_digest)
    expected = {"binding.json", "output"}
    artifact_raw = None
    if action["artifact_sha256"] is not None:
        expected.add("evidence.json")
        artifact_raw, _ = _journal_bytes(protected, prefix + "/evidence.json", journal.MAX_SMALL)
        require(_sha(artifact_raw) == action["artifact_sha256"], "REFERENCES_JOURNAL_ORIGINAL")
    if "import.json" in names:
        expected.add("import.json")
    require(names == expected, "REFERENCES_JOURNAL_INVENTORY")
    output = prefix + "/output"
    children = _journal_children(protected, output)
    require(all(name == "dispatch.json" or re.fullmatch(r"[0-9a-f]{64}\.json", name)
                for name in children), "REFERENCES_JOURNAL_INVENTORY")
    # An undispatched intent/action is retained history. It is never completion.
    if "dispatch.json" not in children:
        require(not children and "import.json" not in names, "REFERENCES_JOURNAL_INCOMPLETE")
        return _sha(raw), None, action["verb"], action["request_key"]
    dispatch_raw, _ = _journal_bytes(protected, output + "/dispatch.json")
    parsed, originals = _journal_validate(journal._validate_v2_dispatch,
        dispatch_raw, root_raw, raw, artifact_raw)
    request_raw = parsed["sql_request_canonical"].encode("ascii")
    _journal_validate(journal._validate_bound_request, request_raw, intent, action, artifact_raw)
    for digest, original, maximum in originals:
        _journal_hash_copy(protected, output, original, maximum)
        require(digest == _sha(original), "REFERENCES_JOURNAL_ORIGINAL")
    if "import.json" in names:
        require(protected.read(prefix + "/import.json", journal.MAX_NATIVE_INDEX, (0, 0, 0o400))
                == dispatch_raw, "REFERENCES_JOURNAL_ORIGINAL")
    if action["stage_sha256"] is not None:
        stage_raw = protected.read(prefix.rsplit("/actions/", 1)[0] + "/"
            + action["stage_sha256"][7:] + ".json", journal.MAX_SMALL, (0, 0, 0o400))
        require(_sha(stage_raw) == action["stage_sha256"], "REFERENCES_JOURNAL_ORIGINAL")
    for name in children - {"dispatch.json"}:
        original, value = _journal_bytes(protected, output + "/" + name)
        digest = "sha256:" + name[:-5]
        require(_sha(original) == digest, "REFERENCES_JOURNAL_ORIGINAL")
        budget.add([digest, output + "/" + name])
        objects.setdefault(digest, []).append(output + "/" + name)
    return _sha(raw), _sha(request_raw), action["verb"], action["request_key"]



def _journal_receipt_links(table, row, request, result, intent, verb):
    """Correlate exact originals using wire.rs validate_original_result fields.

    This verifies SQL receipt identity and result shape. It does not turn a
    historical result/readback into present stop, recovery or restore authority.
    """
    require(type(request) is dict and type(result) is dict
            and set(result) == {"schema", "action", "outcome", "control_epoch", "account_id", "data"}
            and result["schema"] == "owner-beta-paper-action-v1" and result["outcome"] == "OK",
            "REFERENCES_JOURNAL_RESULT")
    kind, args, data = request["kind"], request["args"], result["data"]
    _journal_validate(journal._uuid, result["account_id"])
    require(type(data) is dict and result["action"] ==
            {"BEGIN": "BEGIN_OPERATION", "RECONCILE": "RECONCILE_OPERATION"}.get(kind, kind),
            "REFERENCES_JOURNAL_RESULT")
    require(args.get("request_key") == row["request_key"]
            and (request["account_id"] is None or request["account_id"] == result["account_id"]),
            "REFERENCES_JOURNAL_DATABASE")
    if "generation" in row:
        require(intent["generation"] == row["generation"], "REFERENCES_JOURNAL_DATABASE")
    if "account_id" in row:
        require(result["account_id"] == row["account_id"], "REFERENCES_JOURNAL_DATABASE")
    if table == "owner_beta_paper_operations":
        require(kind == "BEGIN" and verb == "begin", "REFERENCES_JOURNAL_DATABASE")
    if table == "owner_beta_paper_operation_observations":
        require(kind == "RECONCILE" and verb == "reconcile", "REFERENCES_JOURNAL_DATABASE")
    if kind == "BEGIN":
        require(set(data) == {"generation", "operation_event_id", "intent_sha256", "fenced_epoch"}
                and data["generation"] == intent["generation"]
                and data["intent_sha256"] == _sha(args["intent_canonical"].encode("ascii")),
                "REFERENCES_JOURNAL_RESULT")
        epoch = max(args["expected_epoch"], intent["local_epoch_floor"]) + 1
        _journal_validate(journal._uuid, data["operation_event_id"])
        require(type(data["fenced_epoch"]) is int and data["fenced_epoch"] == epoch,
                "REFERENCES_JOURNAL_RESULT")
        if table == "owner_beta_paper_operations":
            require(data["operation_event_id"] == row["fence_event_id"] and epoch == row["fence_epoch"],
                    "REFERENCES_JOURNAL_DATABASE")
    elif kind == "RECONCILE":
        require(set(data) == {"observation_id", "summary", "summary_sha256"}
                and type(data["summary"]) is dict, "REFERENCES_JOURNAL_RESULT")
        summary = data["summary"]
        _journal_validate(journal._uuid, data["observation_id"])
        require(summary.get("observation_id") == data["observation_id"]
                and summary.get("account_id") == result["account_id"]
                and summary.get("generation") == args["generation"]
                and summary.get("inventory_sha256") == args["inventory_sha256"]
                and type(summary.get("control_epoch")) is int
                and summary.get("control_epoch") == args["expected_epoch"]
                and data["summary_sha256"] == _sha(journal._canonical(summary)),
                "REFERENCES_JOURNAL_RESULT")
        epoch = args["expected_epoch"]
        if table == "owner_beta_paper_operation_observations":
            require(data["observation_id"] == row["observation_id"]
                    and summary == row["summary"] and data["summary_sha256"] == row["summary_sha256"],
                    "REFERENCES_JOURNAL_DATABASE")
    elif kind == "CONFIGURE":
        require(set(data) == {"account_id", "event_id", "state", "initial_cash", "currency"}
                and data["account_id"] == result["account_id"] and data["state"] == "OFF"
                and data["initial_cash"] == "10000000.0000" and data["currency"] == "KRW",
                "REFERENCES_JOURNAL_RESULT")
        epoch = 0
        _journal_validate(journal._uuid, data["event_id"])
    elif kind == "TRANSITION":
        transition = args["transition"]
        evidence = _journal_validate(journal._json, args["evidence_canonical"].encode("ascii"), journal.MAX_NATIVE_EMBEDDED)
        if transition == "RECOVERY" and evidence.get("schema") == "owner-beta-paper-recovery-v2":
            require(set(data) == {"event_id", "state", "pending_target_id", "grant_id"}
                    and data["pending_target_id"] == evidence["pending_target_id"],
                    "REFERENCES_JOURNAL_RESULT")
            _journal_validate(journal._uuid, data["grant_id"])
        else:
            require(set(data) == {"event_id", "state"}, "REFERENCES_JOURNAL_RESULT")
        _journal_validate(journal._uuid, data["event_id"])
        require(type(data["state"]) is str, "REFERENCES_JOURNAL_RESULT")
        expected_state = {"ENABLE": "ENABLED", "DRAIN": "DRAINING", "OFF": "OFF",
                          "RESTORED_OFF": "RESTORED_OFF", "RECOVERY": data["state"]}.get(transition)
        require(expected_state is not None and data["state"] == expected_state
                and expected_state in ("OFF", "ENABLED", "DRAINING", "RESTORED_OFF"),
                "REFERENCES_JOURNAL_RESULT")
        epoch = args["expected_epoch"] + (0 if transition == "RECOVERY" else 1)
    else:
        raise ReferenceError("REFERENCES_JOURNAL_RESULT")
    require(type(epoch) is int and 0 <= epoch <= I64
            and type(result["control_epoch"]) is int and result["control_epoch"] == epoch,
            "REFERENCES_JOURNAL_RESULT")
    if table == "owner_beta_paper_events":
        require(row["control_epoch"] == epoch, "REFERENCES_JOURNAL_DATABASE")
        if kind in ("CONFIGURE", "TRANSITION"):
            require(data["event_id"] == row["id"], "REFERENCES_JOURNAL_DATABASE")


def _paper_journal(protected, refs):
    """Traverse actual local journal chains and exact DB receipt originals.

    Native result/readback domain verification remains a typed consuming-reader
    obligation. This closes names, hashes, intent/action/stage/caller links and
    DB exact receipt identity without claiming that a local stage stopped writers.
    """
    prefix = "paper_operations"
    names = _journal_children(protected, prefix)
    database = refs.keyed("owner_beta_paper_operations", "generation")
    if not names:
        require(not database and not any(
            row.get(key) is not None
            for table in ("owner_beta_paper_operations", "owner_beta_paper_operation_observations", "owner_beta_paper_events")
            for row in refs.rows(table)
            for key in ("request_canonical", "request_sha256", "result_canonical", "result_sha256")),
            "REFERENCES_JOURNAL_MISSING")
        return {"operations": {}, "requests": {}, "objects": {}}
    require({"installation-id", "epoch-floor", "operations"} <= names
            <= {"installation-id", "epoch-floor", "operations", "callers"},
            "REFERENCES_JOURNAL_INVENTORY")
    require(protected.entries.get(prefix + "/operations", {}).get("kind") == "directory"
            and ("callers" not in names or protected.entries.get(prefix + "/callers", {}).get("kind") == "directory"),
            "REFERENCES_JOURNAL_INVENTORY")
    identity = protected.read(prefix + "/installation-id", 64, (0, 0, 0o600))
    require(re.fullmatch(rb"[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}\n", identity),
            "REFERENCES_JOURNAL_IDENTITY")
    installation = identity[:-1].decode("ascii")
    _journal_validate(journal._uuid, installation)
    floor_raw = protected.read(prefix + "/epoch-floor", 32, (0, 0, 0o600))
    require(re.fullmatch(rb"(?:0|[1-9][0-9]{0,18})\n", floor_raw), "REFERENCES_JOURNAL_FLOOR")
    floor = _integer(int(floor_raw[:-1]))
    for path, entry in protected.entries.items():
        if path == prefix or path.startswith(prefix + "/"):
            if entry["kind"] == "directory":
                require((entry["uid"], entry["gid"], entry["mode"]) == (0, 0, 0o700),
                        "REFERENCES_JOURNAL_METADATA")
    operations, requests, objects = {}, {}, {}
    callers = []
    bound_generations = set()
    for name in sorted(_journal_children(protected, prefix + "/callers")):
        require(re.fullmatch(r"[0-9a-f]{64}\.json", name), "REFERENCES_JOURNAL_INVENTORY")
        caller_raw, _ = _journal_bytes(protected, prefix + "/callers/" + name, journal.MAX_SMALL)
        caller = _journal_validate(journal._validate_caller, caller_raw, installation)
        refs.budget.add(["caller", name, caller])
        callers.append((name, caller))
        bound_generations.add(caller["generation"])
    for generation in sorted(_journal_children(protected, prefix + "/operations")):
        require(UUID.fullmatch(generation), "REFERENCES_JOURNAL_IDENTITY")
        root = prefix + "/operations/" + generation
        require(protected.entries.get(root, {}).get("kind") == "directory",
                "REFERENCES_JOURNAL_INVENTORY")
        root_raw, _ = _journal_bytes(protected, root + "/intent.json", journal.MAX_SMALL)
        intent = _journal_validate(journal._validate_intent, root_raw)
        require(intent["generation"] == generation and intent["installation_id"] == installation
                and floor >= intent["local_epoch_floor"], "REFERENCES_JOURNAL_IDENTITY")
        children = _journal_children(protected, root)
        require("actions" not in children or protected.entries.get(root + "/actions", {}).get("kind") == "directory",
                "REFERENCES_JOURNAL_INVENTORY")
        require(all(name in ("intent.json", "actions") or journal.STAGE_FILE_RE.fullmatch(name)
                    or re.fullmatch(r"[0-9a-f]{64}\.json", name) for name in children),
                "REFERENCES_JOURNAL_INVENTORY")
        stages = sorted(name for name in children if journal.STAGE_FILE_RE.fullmatch(name))
        previous = previous_sha = None
        for ordinal, name in enumerate(stages):
            require(name == "%08d.json" % ordinal, "REFERENCES_JOURNAL_SEQUENCE")
            stage_raw, _ = _journal_bytes(protected, root + "/" + name, journal.MAX_SMALL)
            stage = _journal_validate(journal._validate_stage, stage_raw, intent, _sha(root_raw), ordinal, previous_sha)
            _journal_validate(journal._advance, None if previous is None else previous["stage"], stage["stage"], intent)
            if generation in bound_generations or "actions" in children:
                _journal_stage_epoch(intent, stage, previous)
            _journal_hash_copy(protected, root, stage_raw, journal.MAX_SMALL)
            require(stage["control_epoch"] <= floor, "REFERENCES_JOURNAL_FLOOR")
            previous, previous_sha = stage, _sha(stage_raw)
        action_previous = None
        reconcile_ordinal = 0
        for ordinal, name in enumerate(sorted(_journal_children(protected, root + "/actions"))):
            require(name == "%08d" % ordinal, "REFERENCES_JOURNAL_SEQUENCE")
            action_previous, request_hash, verb, request_key = _journal_action(protected, root + "/actions/" + name,
                intent, root_raw, ordinal, action_previous, objects, refs.budget)
            if verb == "reconcile":
                reconcile_ordinal += 1
                require(request_key == "%s:reconcile:%d" % (generation, reconcile_ordinal),
                        "REFERENCES_JOURNAL_SEQUENCE")
            if request_hash is not None:
                require(request_hash not in requests, "REFERENCES_JOURNAL_AMBIGUOUS")
                refs.budget.add(["request", request_hash, generation, ordinal, verb])
                requests[request_hash] = (generation, ordinal, verb)
        for name in children:
            if re.fullmatch(r"[0-9a-f]{64}\.json", name):
                raw, _ = _journal_bytes(protected, root + "/" + name)
                digest = "sha256:" + name[:-5]
                require(_sha(raw) == digest, "REFERENCES_JOURNAL_ORIGINAL")
                refs.budget.add([digest, root + "/" + name])
                objects.setdefault(digest, []).append(root + "/" + name)
        row = database.get(generation)
        if row is not None:
            require(row["installation_id"] == installation and row["intent"] == intent
                    and row["intent_sha256"] == _sha(root_raw)
                    and all(row[key] == intent[key] for key in ("account_id", "request_key", "purpose",
                        "scope", "expected_epoch", "local_epoch_floor", "release_manifest_sha256")),
                    "REFERENCES_JOURNAL_DATABASE")
        refs.budget.add(["journal", generation, _sha(root_raw)])
        operations[generation] = (root, _sha(root_raw), intent)
    require(set(database) <= set(operations), "REFERENCES_JOURNAL_MISSING")
    for name, caller in callers:
        operation = operations.get(caller["generation"])
        require(operation is not None and caller["root_intent_sha256"] == operation[1]
                and name == journal._caller_name(installation, caller["account_id"], caller["caller_key"])
                and caller["account_id"] == operation[2]["account_id"]
                and caller["command_canonical"].encode("ascii") == journal._command_projection(operation[2]),
                "REFERENCES_JOURNAL_CALLER")
        _journal_validate(journal._validate_bound_root, operation[2], caller["caller_key"])
    for table in ("owner_beta_paper_operations", "owner_beta_paper_operation_observations", "owner_beta_paper_events"):
        for row in refs.rows(table):
            exact = [row.get(k) for k in ("request_canonical", "request_sha256", "result_canonical", "result_sha256")]
            if all(value is None for value in exact):
                continue  # Frozen all-null old SQL history; never synthesize originals.
            require(all(type(value) is str for value in exact), "REFERENCES_EXACT_RECEIPT")
            parsed = []
            for offset in (0, 2):
                try:
                    raw = exact[offset].encode("ascii")
                except UnicodeError:
                    raise ReferenceError("REFERENCES_EXACT_RECEIPT") from None
                require(_sha(raw) == exact[offset + 1], "REFERENCES_EXACT_RECEIPT")
                parsed.append(_journal_validate(journal._json, raw, journal.MAX_NATIVE_EXACT))
                paths = objects.get(exact[offset + 1])
                require(paths, "REFERENCES_JOURNAL_RECEIPT_MISSING")
                for path in paths:
                    require(protected.read(path, journal.MAX_NATIVE_EXACT, (0, 0, 0o400)) == raw,
                            "REFERENCES_JOURNAL_ORIGINAL")
            require(exact[1] in requests, "REFERENCES_JOURNAL_RECEIPT_MISSING")
            generation, _, verb = requests[exact[1]]
            _journal_receipt_links(table, row, parsed[0], parsed[1], operations[generation][2], verb)
    return {"operations": operations, "requests": requests, "objects": objects}


def _paper_relationships(refs, originals):
    """History joins use each immutable row, never today's active policy."""
    accounts = refs.keyed("accounts", "id")
    configs = refs.keyed("user_strategy_configs", "id")
    events = refs.keyed("owner_beta_paper_events", "id")
    lineages = refs.keyed("owner_beta_paper_lineage", "id")
    targets = refs.keyed("pending_targets", "id")
    controls = refs.keyed("owner_beta_paper_control", "account_id")
    for row in refs.rows("account_strategy_bindings"):
        account, config = accounts.get(row["account_id"]), configs.get(row["strategy_config_id"])
        require(account is not None and config is not None
                and account["owner_user_id"] == row["owner_user_id"] == config["owner_user_id"]
                and config["strategy_id"] == row["strategy_id"] and config["strategy_version"] == row["strategy_version"],
                "REFERENCES_PAPER_BINDING")
    for table in ("owner_beta_paper_control", "owner_beta_paper_events", "owner_beta_paper_lineage",
                  "owner_beta_paper_sessions", "pending_targets", "paper_rebalance_previews"):
        for row in refs.rows(table):
            account = accounts.get(row["account_id"])
            require(account is not None and account["account_type"] == "PAPER"
                    and account["owner_user_id"] == row["owner_user_id"], "REFERENCES_PAPER_OWNER")
            if "strategy_config_id" in row:
                config = configs.get(row["strategy_config_id"])
                require(config is not None and config["owner_user_id"] == row["owner_user_id"],
                        "REFERENCES_PAPER_BINDING")
    for row in refs.rows("owner_beta_paper_control"):
        require(accounts[row["account_id"]]["cost_profile_id"] == row["cost_profile_id"]
                and accounts[row["account_id"]]["cost_profile_version"] == row["cost_profile_version"],
                "REFERENCES_PAPER_COST")
        for name in ("consent_event_id", "activation_event_id"):
            if row[name] is not None:
                event = events.get(row[name])
                require(event is not None and event["account_id"] == row["account_id"]
                        and event["owner_user_id"] == row["owner_user_id"], "REFERENCES_PAPER_EVENT")
    for row in refs.rows("owner_beta_paper_operations"):
        require(row["account_id"] in accounts, "REFERENCES_PAPER_OWNER")
    for row in refs.rows("owner_beta_paper_sessions"):
        target = targets.get(row["pending_target_id"])
        require(target is not None and target["owner_user_id"] == row["owner_user_id"]
                and target["account_id"] == row["account_id"]
                and target["effective_date"] == row["effective_session"], "REFERENCES_PAPER_SESSION")
        for key in ("execution_admission_sha256", "valuation_admission_sha256"):
            if row[key] is not None:
                require(row[key] in originals, "REFERENCES_PAPER_ADMISSION")
    for row in refs.rows("owner_beta_paper_recovery_grants"):
        lineage, target = lineages.get(row["lineage_id"]), targets.get(row["pending_target_id"])
        require(lineage is not None and target is not None and lineage["account_id"] == row["account_id"]
                == target["account_id"] and target["beta_lineage_id"] == row["lineage_id"]
                and lineage["effective_session"] == row["effective_session"], "REFERENCES_PAPER_RECOVERY")
        for key in ("signal_admission_sha256", "execution_admission_sha256"):
            if row[key] is not None:
                require(row[key] in originals, "REFERENCES_PAPER_ADMISSION")
    # Retain revoked/expired rights unchanged. They are archive evidence, never
    # a current resume decision and never required to equal the current config.
    for row in refs.rows("data_entitlements"):
        _hash(row["contract_document_sha256"])
        require(type(row["contract_reference"]) is str and row["contract_reference"],
                "REFERENCES_RIGHTS")


_PENDING_TOKEN = object()


class _PendingSourceClosure:
    """Owned provisional traversal. No serialization, completion flag or digest API.

    This retains the real entered Snapshot and the completed decoder index.
    A/C still have to bind their genuine selected owner, held transaction and
    consuming typed verification. Visiting an original returns data, not proof.
    """
    __slots__ = ("_protected", "_references", "_raw", "_daily", "_frontiers", "_journal",
                 "_closed", "_active")

    def __init__(self, token, protected, references, raw, originals, frontiers, local):
        require(token is _PENDING_TOKEN and type(protected) is _Protected
                and type(references) is _References and references.done and not references.failed,
                "REFERENCES_PENDING_PRIVATE")
        self._protected, self._references, self._raw = protected, references, raw
        self._daily, self._frontiers, self._journal = originals, frontiers, local
        self._closed = self._active = False

    def _check(self):
        require(not self._closed and self._references.done and not self._references.failed,
                "REFERENCES_INVALIDATED")
        try:
            self._protected.revalidate()
            self._protected.source_class.verify_unchanged(self._protected.snapshot)
        except BaseException:
            self.abort()
            raise

    def __reduce__(self):
        raise ReferenceError("REFERENCES_PENDING_PRIVATE")

    def __reduce_ex__(self, _protocol):
        raise ReferenceError("REFERENCES_PENDING_PRIVATE")

    def __copy__(self):
        raise ReferenceError("REFERENCES_PENDING_PRIVATE")

    def __deepcopy__(self, _memo):
        raise ReferenceError("REFERENCES_PENDING_PRIVATE")

    def visit_daily_originals(self, consumer):
        """Stream exact protected originals to a local typed consumer.

        The callback's return value is discarded: a bool/hash/JSON cannot close a
        dependency. Data are bounded per leaf; no aggregate bars history is kept.
        A callback exception or any later source change invalidates this owner.
        """
        self._check()
        require(callable(consumer) and not self._active, "REFERENCES_PENDING_STATE")
        self._active = True
        try:
            for identity in sorted(self._daily):
                self._check()
                prefix = "daily/admitted/" + identity[7:]
                with contextlib.ExitStack() as stack:
                    handles = {}
                    for name in ("admission.json", "manifest.json", "bars.ndjson", "normalization.json"):
                        fd, entry = stack.enter_context(self._protected._member(prefix + "/" + name))
                        handles[name] = fd
                    consumer(identity, handles)
                self._check()
        except (ReferenceError, KeyboardInterrupt, SystemExit):
            self.abort()
            raise
        except BaseException:
            self.abort()
            raise ReferenceError("REFERENCES_CONSUMER") from None
        finally:
            self._active = False

    def visit_archive_members(self, consumer):
        """Visit all actual protected members in the fixed 20-class order.

        The class record is the existing codec record. A file visit borrows an
        actual read-only FD and an independent copy of captured metadata. Empty
        class records describe this captured view only; A's original source
        selection/absence custody and C's consuming proof remain required.
        No callback return value is interpreted as validation or completion.
        """
        self._check()
        require(callable(consumer) and not self._active, "REFERENCES_PENDING_STATE")
        self._active = True
        try:
            protected = self._protected
            for record in files.class_records(protected.manifest_bytes, protected.limits,
                                               profile="paper-combined"):
                consumer("class", record["class"], None, dict(record))
                for path in sorted(protected.entries, key=lambda value: value.encode("utf-8")):
                    entry = protected.entries[path]
                    if path.split("/", 1)[0] != record["class"] or entry["kind"] != "file":
                        continue
                    protected.hash_file(path)
                    with protected._member(path) as (fd, captured):
                        consumer("member", path, fd, dict(captured))
                    protected.hash_file(path)
            self._check()
        except (ReferenceError, KeyboardInterrupt, SystemExit):
            self.abort()
            raise
        except BaseException:
            self.abort()
            raise ReferenceError("REFERENCES_CONSUMER") from None
        finally:
            self._active = False

    def visit_database_references(self, consumer):
        """Pass exact validated decoder rows; never accept a caller row list."""
        self._check()
        require(callable(consumer) and not self._active, "REFERENCES_PENDING_STATE")
        self._active = True
        try:
            for table, rows in sorted(self._references.tables.items()):
                for key, row in sorted(rows.items()):
                    consumer(table, key, audit.canonical(row))
            for table, rows in (("jobs", self._references.jobs), ("job_attempts", self._references.attempts)):
                for key, row in sorted(rows.items()):
                    consumer(table, key, audit.canonical(row))
            self._check()
        except (ReferenceError, KeyboardInterrupt, SystemExit):
            self.abort()
            raise
        except BaseException:
            self.abort()
            raise ReferenceError("REFERENCES_CONSUMER") from None
        finally:
            self._active = False

    def abort(self):
        self._closed = True
        self._protected.failed = True
        self._references.failed = True
        self._references.tables.clear()
        self._references.jobs.clear()
        self._references.attempts.clear()

    def __enter__(self):
        self._check()
        return self

    def __exit__(self, *_):
        try:
            if not self._closed:
                self._check()
        finally:
            self.abort()


def prepare_references(snapshot, stream):
    """Build actual provisional source traversal; never an archive audit result."""
    protected = None
    try:
        protected = _Protected(snapshot)
        projection = json.loads(audit.PROJECTION.read_bytes())["archive_audit_v1"]
        references = _References(protected.limits, projection)
        references.decode(stream)
        raw = _scan_raw(protected, references.budget)
        _source_links(references, raw)
        originals = _daily_admissions(protected, references, raw)
        frontiers = _frontiers(protected, references, raw, originals)
        _calendar_sources(protected, references, raw)
        _daily_events_receipts(references, frontiers, protected=protected, raw=raw, originals=originals)
        _fixed_floors(protected)
        _daily_config(protected, references)
        _simulation_evidence(protected, references, raw)
        _legacy_history(protected, raw)
        _paper_relationships(references, originals)
        local = _paper_journal(protected, references)
        protected.revalidate()
        return _PendingSourceClosure(_PENDING_TOKEN, protected, references, raw, originals, frontiers, local)
    except (ReferenceError, KeyboardInterrupt, SystemExit):
        if protected is not None:
            protected.failed = True
        raise
    except BaseException:
        if protected is not None:
            protected.failed = True
        raise ReferenceError("REFERENCES_REJECTED") from None



def describe(snapshot, stream, dataset_rows=()):
    """Describe actual selected originals; never return a restore permit.

    The host retains snapshot/exporter/selection custody through capture. Native
    independently consumes the typed Daily/Curated/Paper/R4 domains before SQL.
    """
    with prepare_references(snapshot, stream) as pending:
        values = _dataset_rows(pending._references, dataset_rows)
        root = _provisional_root(pending._protected, pending._references, values)
        pending._check()
        return root._description()
