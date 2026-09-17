#!/usr/bin/env python3
"""Offline contract tests for build-resource-policy.py."""

import importlib.util
import io
import os
from contextlib import redirect_stderr, redirect_stdout


ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "../.."))
MODULE_PATH = os.path.join(ROOT, "scripts/ops/lib/build-resource-policy.py")


def load_module():
    spec = importlib.util.spec_from_file_location("build_resource_policy_self_test", MODULE_PATH)
    if spec is None or spec.loader is None:
        raise AssertionError("module-spec-missing")
    module = importlib.util.module_from_spec(spec)
    stdout = io.StringIO()
    stderr = io.StringIO()
    with redirect_stdout(stdout), redirect_stderr(stderr):
        spec.loader.exec_module(module)
    if stdout.getvalue() or stderr.getvalue():
        raise AssertionError("module-import-had-side-effects")
    return module


MEMINFO = """MemTotal:       8388608 kB
MemAvailable:   4194304 kB
SwapFree:             0 kB
HugePages_Total:      0
"""
PRESSURE = """some avg10=0.00 avg60=0.00 avg300=0.00 total=1
full avg10=0.00 avg60=0.00 avg300=0.00 total=1
"""


def reject(module, meminfo=MEMINFO, pressure=PRESSURE):
    try:
        module.parse_observation(meminfo, pressure)
    except (TypeError, ValueError):
        return
    raise AssertionError("malformed-observation-accepted")


def main():
    module = load_module()
    observation = module.parse_observation(MEMINFO, PRESSURE)
    if set(observation) != {
        "MemAvailable",
        "SwapFree",
        "memory_psi_some_avg10",
        "memory_psi_full_avg10",
    }:
        raise AssertionError("observation-keys")
    if observation != {
        "MemAvailable": 4194304,
        "SwapFree": 0,
        "memory_psi_some_avg10": 0.0,
        "memory_psi_full_avg10": 0.0,
    }:
        raise AssertionError("observation-values")
    if module.failure_reason(observation) is not None:
        raise AssertionError("plenty-ram-swap-zero-low-psi-failed")

    low_ram = module.parse_observation(
        "MemAvailable: 2097151 kB\nSwapFree: 8388608 kB\n", PRESSURE
    )
    if module.failure_reason(low_ram) != "mem-available-below-floor":
        raise AssertionError("low-ram-did-not-fail")

    high_pressure = module.parse_observation(
        MEMINFO,
        "some avg10=0.00 avg60=0.00 avg300=0.00 total=1\n"
        "full avg10=5.00 avg60=0.00 avg300=0.00 total=1\n",
    )
    if module.failure_reason(high_pressure) != "memory-pressure-high":
        raise AssertionError("five-percent-pressure-did-not-fail")
    below_pressure = module.parse_observation(
        MEMINFO,
        "some avg10=100.00 avg60=0.00 avg300=0.00 total=1\n"
        "full avg10=4.99 avg60=0.00 avg300=0.00 total=1\n",
    )
    if module.failure_reason(below_pressure) is not None:
        raise AssertionError("below-five-percent-pressure-failed")

    final_sample = module.parse_observation(
        "MemAvailable: 8722008 kB\nSwapFree: 499484 kB\n", PRESSURE
    )
    if module.failure_reason(final_sample) is not None:
        raise AssertionError("representative-final-sample-failed")

    for bad_pressure in (
        "some avg10=0.00 avg60=0.00 avg300=0.00 total=1\n"
        "full avg10=NaN avg60=0.00 avg300=0.00 total=1\n",
        "some avg10=0.00 avg60=0.00 total=1\n"
        "full avg10=0.00 avg60=0.00 avg300=0.00 total=1\n",
        "some avg10=-0.01 avg60=0.00 avg300=0.00 total=1\n"
        "full avg10=0.00 avg60=0.00 avg300=0.00 total=1\n",
        "some avg10=0.00 avg60=0.00 avg300=0.00 total=1\n"
        "full avg10=100.01 avg60=0.00 avg300=0.00 total=1\n",
        "some avg10=0.00 avg10=0.00 avg300=0.00 total=1\n"
        "full avg10=0.00 avg60=0.00 avg300=0.00 total=1\n",
    ):
        reject(module, pressure=bad_pressure)

    reject(module, meminfo="SwapFree: 0 kB\n", pressure=PRESSURE)
    reject(module, meminfo="MemAvailable: 1 kB\n", pressure=PRESSURE)
    reject(module, meminfo="MemAvailable: NaN kB\nSwapFree: 0 kB\n", pressure=PRESSURE)
    reject(module, meminfo="MemAvailable: 1 kB\nMemAvailable: 2 kB\nSwapFree: 0 kB\n", pressure=PRESSURE)
    reject(module, meminfo="MemAvailable: 1\nSwapFree: 0 kB\n", pressure=PRESSURE)

    try:
        module.failure_reason({"MemAvailable": 1})
    except (TypeError, ValueError):
        pass
    else:
        raise AssertionError("invalid-observation-accepted")

    print("BUILD_RESOURCE_POLICY_SELF_TEST: PASS (deterministic procfs fixtures; no live host assumptions)")


if __name__ == "__main__":
    main()
