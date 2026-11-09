"""Protected Paper source-reference traversal (incomplete closure, fail closed).

Only ``describe(snapshot, stream)`` is a candidate integration entry point. It
accepts an entered, source-owned paper-combined Snapshot and owns the real audit
stream decoder. Neither a manifest dictionary nor a decoder result is authority.

The bounded Raw original/copy graph, protected descriptor reads, database
reference spool and framed accumulator are implemented here. Full Daily/R4 and
Paper dependency closure is deliberately NOT implemented yet. ``describe`` raises
REFERENCES_CLOSURE_INCOMPLETE after the implemented checks, including on empty
input. It cannot currently return a source_files group or an archive audit.

Internal framing (not a new public schema): source_class / class-name / the
existing class_records row; source_member / UTF8(path).hex() / exact codec entry;
source_reference / table + ':' + key / exact normalized reference row. Labels and
keys sort by UTF8 bytes; all three components use the audit decoder's decimal
byte-length framing. Actual member bytes are rehashed through retained FDs before
these records can be added. The unfinished closure gate must not be removed until
all inventory and typed dependencies below are implemented and Root-tested.
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
# This is an explicit implementation inventory, not a caller-configurable policy.
INCOMPLETE = (
    "DAILY_FRONTIER_NORMALIZATION_HISTORY",
    "HISTORICAL_V2_V3_CURATED_DEPENDENCIES",
    "PAPER_CLASS_SELECTION_AND_JOURNAL_CLOSURE",
    "PAPER_CONTRACT_COST_ENTITLEMENT_RELEASE_CAPABILITY",
    "SIMULATION_ALL_HISTORY_CONFIG_RECEIPT_DEPENDENCIES",
)


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
        calendar_dates = []
        for claim in session["calendar"]:
            calendar = _object(claim, ("civil_date", "session_type", "exchange", "timezone", "source_file",
                                       "normalized_document_sha256", "legacy_projection"))
            require(calendar["exchange"] == "KRX" and calendar["timezone"] == "Asia/Seoul"
                    and calendar["session_type"] in ("TRADING", "CLOSED"), "REFERENCES_DAILY_CALENDAR")
            file = _file_evidence(calendar["source_file"], raw)
            require(file["kind"] == "calendar", "REFERENCES_DAILY_CALENDAR")
            target = file["copied_from"][-1]["batch_date"] if file["copied_from"] else file["batch_date"]
            body = _raw_json(protected.read(raw.reference(file)["path"], 67108864), 67108864)
            days = _calendar_provider_days(body, target)
            require(calendar["civil_date"] in days and days[calendar["civil_date"]]["session_type"] == calendar["session_type"],
                    "REFERENCES_DAILY_CALENDAR")
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
        require(all(source[a] == file[b] for a, b in pairs) and source["requested_target"] == file["batch_date"]
                and _utc9(source["source_retrieved_at"]) == file["retrieved_at"]
                and source["copy_chain"] == file["copied_from"], "REFERENCES_CALENDAR_SOURCE")
        preimage = ("owner-beta-daily-calendar-source-v1\n" + source["owner_user_id"] + "\n"
                    + source["policy_sha256"] + "\ndaily-raw-reconstruction-v1\n").encode("ascii") + _wire(file)
        require(_sha(preimage) == key, "REFERENCES_CALENDAR_IDENTITY")
        original = raw.reference(file)
        document = _raw_json(protected.read(original["path"], 67108864), 67108864)
        expected = _calendar_provider_days(document, target)
        require(claimed.get(key) == expected, "REFERENCES_CALENDAR_DAYS")
    return sources


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


def _daily_events_receipts(refs, frontiers):
    admissions = refs.keyed("owner_beta_daily_admissions", "admission_sha256")
    events = refs.keyed("owner_beta_daily_source_events", "event_sha256")
    for key, event in events.items():
        fields = (event[k] for k in ("owner_user_id", "source_kind", "code", "range_start", "range_end", "evidence_sha256"))
        require(_sha(("owner-beta-daily-source-event-v1\n" + "\n".join(fields) + "\n").encode("ascii")) == key,
                "REFERENCES_SOURCE_EVENT")
        if event["admission_sha256"] is not None:
            admission = admissions.get(event["admission_sha256"])
            require(admission is not None and admission["owner_user_id"] == event["owner_user_id"]
                    and admission["range_start"] <= event["range_end"] and admission["range_end"] >= event["range_start"],
                    "REFERENCES_SOURCE_EVENT")
    for row in refs.rows("owner_beta_daily_admission_events"):
        admission = admissions.get(row["admission_sha256"])
        require(admission is not None and admission["owner_user_id"] == row["owner_user_id"], "REFERENCES_ADMISSION_EVENT")
        require(row["kind"] in ("CONFLICT", "REVOKED"), "REFERENCES_ADMISSION_EVENT")
        preimage = "owner-beta-daily-admission-event-v1\n" + row["kind"] + "\n" + row["admission_sha256"] + "\n" + row["evidence_sha256"] + "\n"
        require(_sha(preimage.encode("ascii")) == row["event_sha256"], "REFERENCES_ADMISSION_EVENT")
        if row["kind"] == "CONFLICT":
            require(row["evidence_sha256"] in events, "REFERENCES_ADMISSION_EVENT")
    receipts = refs.keyed("owner_beta_daily_poll_receipts", "receipt_id")
    for row in receipts.values():
        require((row["completed_at"] is None) == (row["result_code"] is None), "REFERENCES_POLL_RECEIPT")
        if row["result_code"] == "VERIFIED":
            require(row["admission_sha256"] is not None and row["source_frontier_sha256"] in frontiers,
                    "REFERENCES_POLL_ADMISSION")
        if row["completed_at"] is not None:
            require(row["completed_at"] >= row["started_at"], "REFERENCES_POLL_RECEIPT")
        if row["admission_sha256"] is not None:
            admission = admissions.get(row["admission_sha256"])
            require(admission is not None and admission["owner_user_id"] == row["owner_user_id"]
                    and admission["policy_sha256"] == row["policy_sha256"], "REFERENCES_POLL_ADMISSION")
            if row["result_code"] == "VERIFIED":
                require(row["source_frontier_sha256"] == admission["evidence_json"]["source_frontier_sha256"]
                        and row["source_frontier_sha256"] in frontiers
                        and row["completion_binding_sha256"] == _sha(b"sql-admission-v1" + _pg_jsonb(admission["evidence_json"])),
                        "REFERENCES_POLL_BINDING")
        if row["event_sha256"] is not None:
            require(row["event_sha256"] in events and events[row["event_sha256"]]["owner_user_id"] == row["owner_user_id"],
                    "REFERENCES_POLL_EVENT")
    for state in refs.rows("owner_beta_daily_poll_state"):
        if state["latest_receipt_id"] is None:
            require(all(state[k] is None for k in ("last_completed_at", "last_result_code", "verified_admission_sha256", "source_frontier_sha256")),
                    "REFERENCES_POLL_STATE")
            continue
        receipt = receipts.get(state["latest_receipt_id"])
        require(receipt is not None and receipt["stage"] == state["stage"], "REFERENCES_POLL_STATE")
        # An unfinished latest poll deliberately retains the previous completion.
        if receipt["completed_at"] is not None:
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
    for row in refs.rows("owner_beta_simulation_runs"):
        require(all(row[key] == expected for key, expected in pins.items()), "REFERENCES_SIMULATION_PIN")
    for field in ("price_source", "action_source"):
        source = approval[field]
        originals = [value for key, value in raw_index.files.items() if key[2] == source["batch_id"]]
        require(len(originals) == source["file_count"] and all(
            value["batch_sha256"] == source["batch_json_sha256"]
            and value["line_sha256"] == source["manifest_line_sha256"] for value in originals),
            "REFERENCES_HISTORICAL_RAW")


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
                 if identity[:3] == ("kis", "kr", stage5["batch_id"])}
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


def _provisional_root(protected, references):
    """Bind actual original bytes and validated DB references, never closure.

    Used for internal vectors and the unfinished describe path. A root calculated
    here is insufficient for export; unknown inventory/dependency closure is a
    mandatory separate gate, not a serialized success flag.
    """
    require(references.done and not references.failed, "REFERENCES_STREAM_STATE")
    protected.revalidate()
    for path, entry in protected.entries.items():
        if entry["kind"] == "file":
            protected.hash_file(path)
    root = _FramedRoot()
    for value in files.class_records(protected.manifest_bytes, protected.limits, profile="paper-combined"):
        root.add("source_class", value["class"], value)
    for path in sorted(protected.entries, key=lambda p: p.encode("utf-8")):
        root.add("source_member", path.encode("utf-8").hex(), protected.entries[path])
    for table, rows in sorted(references.tables.items()):
        for key, row in sorted(rows.items()):
            root.add("source_reference", table + ":" + key, row)
    protected.revalidate()
    return root


def describe(snapshot, stream):
    """Run implemented traversal; fail closed until the full closure is supplied.

    The caller must still own transport termination and the same held exported
    snapshot/DrainedHandle. This function performs no lifecycle or SQL operation.
    No supplied dictionary/boolean/digest can bypass the incomplete-closure gate.
    """
    protected = None
    try:
        protected = _Protected(snapshot)
        projection = json.loads(audit.PROJECTION.read_bytes())["archive_audit_v1"]
        references = _References(protected.limits, projection)
        references.decode(stream)
        protected.revalidate()
        raw = _scan_raw(protected, references.budget)
        _source_links(references, raw)
        originals = _daily_admissions(protected, references, raw)
        frontiers = _frontiers(protected, references, raw, originals)
        _calendar_sources(protected, references, raw)
        _daily_events_receipts(references, frontiers)
        _fixed_floors(protected)
        _daily_config(protected, references)
        _simulation_evidence(protected, references, raw)
        _legacy_history(protected, raw)
        _provisional_root(protected, references)
        protected.finish()
        # Deliberately no serialization or returned partial hash: these missing
        # dependencies are functional work, not an operator approval switch.
        raise ReferenceError("REFERENCES_CLOSURE_INCOMPLETE")
    except (ReferenceError, KeyboardInterrupt, SystemExit):
        raise
    except BaseException:
        raise ReferenceError("REFERENCES_REJECTED") from None
    finally:
        if protected is not None:
            protected.failed = True
