"""Validated host-resource observations for the production build gate.

This module deliberately has no command-line entry point and performs no I/O
at import time.  The only live read is made by :func:`read_observation`, which
reads the two procfs files used by the gate.  A full-PSI avg10 of 5 percent is
an intentionally conservative operational threshold for this build workflow;
it is not a kernel-prescribed or universally proven safety limit.
"""

import math
import re


DEFAULT_MIN_MEM_AVAILABLE_KIB = 2097152
MEMINFO_MAX_KIB = (1 << 63) - 1
PSI_MAX_PERCENT = 100.0
PSI_TOTAL_MAX = (1 << 64) - 1
PSI_FULL_AVG10_THRESHOLD = 5.0

OBSERVATION_KEYS = frozenset(
    {
        "MemAvailable",
        "SwapFree",
        "memory_psi_some_avg10",
        "memory_psi_full_avg10",
    }
)
_MEMINFO_KEY_RE = re.compile(r"[A-Za-z][A-Za-z0-9_()]*")
_MEMINFO_VALUE_RE = re.compile(
    r"(?P<value>[0-9]+)(?:[ \t]+(?P<unit>kB))?[ \t]*"
)
_PSI_AVG_RE = re.compile(r"(?:0|[0-9]+)(?:\.[0-9]+)?")
_PSI_TOTAL_RE = re.compile(r"[0-9]+")
_PSI_FIELDS = frozenset({"avg10", "avg60", "avg300", "total"})


def _require_text(value, name):
    if not isinstance(value, str) or not value:
        raise ValueError(f"{name}-invalid")
    return value


def _parse_meminfo(meminfo_text):
    text = _require_text(meminfo_text, "meminfo")
    lines = text.splitlines()
    if not lines or any(line == "" for line in lines):
        raise ValueError("meminfo-lines-invalid")

    values = {}
    for line in lines:
        if ":" not in line:
            raise ValueError("meminfo-line-invalid")
        key, rest = line.split(":", 1)
        if not _MEMINFO_KEY_RE.fullmatch(key):
            raise ValueError("meminfo-key-invalid")
        if key in values:
            raise ValueError("meminfo-duplicate-field")
        match = re.fullmatch(r"[ \t]+" + _MEMINFO_VALUE_RE.pattern, rest)
        if match is None:
            raise ValueError("meminfo-value-invalid")
        raw_value = match.group("value")
        number = int(raw_value)
        if number < 0 or number > MEMINFO_MAX_KIB:
            raise ValueError("meminfo-value-out-of-range")
        unit = match.group("unit")
        if key in ("MemAvailable", "SwapFree") and unit != "kB":
            raise ValueError("meminfo-required-unit-missing")
        values[key] = number

    if "MemAvailable" not in values or "SwapFree" not in values:
        raise ValueError("meminfo-required-field-missing")
    return values


def _parse_psi_value(raw_value, field):
    if field == "total":
        if _PSI_TOTAL_RE.fullmatch(raw_value) is None:
            raise ValueError("psi-total-invalid")
        value = int(raw_value)
        if value < 0 or value > PSI_TOTAL_MAX:
            raise ValueError("psi-total-out-of-range")
        return value
    if _PSI_AVG_RE.fullmatch(raw_value) is None:
        raise ValueError("psi-average-invalid")
    value = float(raw_value)
    if not math.isfinite(value) or value < 0.0 or value > PSI_MAX_PERCENT:
        raise ValueError("psi-average-out-of-range")
    return value


def _parse_pressure(pressure_text):
    text = _require_text(pressure_text, "pressure")
    lines = text.splitlines()
    if len(lines) != 2 or any(line == "" for line in lines):
        raise ValueError("psi-lines-invalid")

    values = {}
    for line in lines:
        fields = line.split()
        if len(fields) != 5 or fields[0] not in ("some", "full"):
            raise ValueError("psi-line-invalid")
        kind = fields[0]
        if kind in values:
            raise ValueError("psi-duplicate-kind")
        parsed = {}
        for token in fields[1:]:
            if token.count("=") != 1:
                raise ValueError("psi-field-invalid")
            field, raw_value = token.split("=", 1)
            if field not in _PSI_FIELDS or field in parsed or not raw_value:
                raise ValueError("psi-field-invalid")
            parsed[field] = _parse_psi_value(raw_value, field)
        if set(parsed) != _PSI_FIELDS:
            raise ValueError("psi-required-field-missing")
        values[kind] = parsed

    if set(values) != {"some", "full"}:
        raise ValueError("psi-required-kind-missing")
    return values


def parse_observation(meminfo_text: str, pressure_text: str) -> dict:
    """Parse and validate one complete procfs resource observation.

    ``MemAvailable`` and ``SwapFree`` are returned in KiB.  PSI averages are
    returned as finite percentage floats.  Every field in both procfs
    contracts is checked so malformed, duplicated, missing, or non-finite
    input cannot become a gate decision.
    """

    meminfo = _parse_meminfo(meminfo_text)
    pressure = _parse_pressure(pressure_text)
    return {
        "MemAvailable": meminfo["MemAvailable"],
        "SwapFree": meminfo["SwapFree"],
        "memory_psi_some_avg10": pressure["some"]["avg10"],
        "memory_psi_full_avg10": pressure["full"]["avg10"],
    }


def read_observation() -> dict:
    """Read and validate the live procfs resource observation."""

    with open("/proc/meminfo", "r", encoding="ascii", newline="") as handle:
        meminfo_text = handle.read()
    with open("/proc/pressure/memory", "r", encoding="ascii", newline="") as handle:
        pressure_text = handle.read()
    return parse_observation(meminfo_text, pressure_text)


def _validate_observation(observation):
    if not isinstance(observation, dict) or set(observation) != OBSERVATION_KEYS:
        raise ValueError("observation-schema-invalid")
    for key in ("MemAvailable", "SwapFree"):
        value = observation[key]
        if type(value) is not int or value < 0 or value > MEMINFO_MAX_KIB:
            raise ValueError("observation-memory-invalid")
    for key in ("memory_psi_some_avg10", "memory_psi_full_avg10"):
        value = observation[key]
        if not isinstance(value, (int, float)) or isinstance(value, bool):
            raise ValueError("observation-psi-invalid")
        if not math.isfinite(float(value)) or not 0.0 <= float(value) <= PSI_MAX_PERCENT:
            raise ValueError("observation-psi-out-of-range")


def failure_reason(observation, min_mem_available_kib=2097152):
    """Return the fixed stop reason for a validated observation, if any."""

    _validate_observation(observation)
    if type(min_mem_available_kib) is not int or not 0 <= min_mem_available_kib <= MEMINFO_MAX_KIB:
        raise ValueError("memory-floor-invalid")
    if observation["MemAvailable"] < min_mem_available_kib:
        return "mem-available-below-floor"
    if observation["memory_psi_full_avg10"] >= PSI_FULL_AVG10_THRESHOLD:
        return "memory-pressure-high"
    return None
