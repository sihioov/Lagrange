#!/usr/bin/env python3
"""Explicit, preactivation-only forward binding between immutable releases.

This operation never activates a worker, resets an epoch, edits an installed
configuration, or repairs history. The operator supplies the new first session
in a separately installed release after deployment/calendar validation.
"""
from __future__ import annotations

import contextlib
import datetime as dt
import os
import re


def validate_pair(install, old, new, *, today=None):
    today = today or dt.datetime.now(dt.timezone(dt.timedelta(hours=9))).date()
    allowed = {"code_commit", "first_auto_session"}
    if (set(old) != set(new) or
            {k: v for k, v in old.items() if k not in allowed} !=
            {k: v for k, v in new.items() if k not in allowed} or
            old["code_commit"] == new["code_commit"] or
            not install.is_commit(old["code_commit"]) or
            not install.is_commit(new["code_commit"]) or
            not install.trading_date(old["first_auto_session"]) <= today <
            install.trading_date(new["first_auto_session"])):
        install.reject("DAILY_FIRST_SESSION_BINDING")


def render(install, source, target, values, *, apply):
    old, _ = install.verify_config(source.release, source.commit)
    new, _ = install.verify_config(target.release, target.commit)
    validate_pair(install, old, new)
    if type(apply) is not bool:
        install.reject("DAILY_FIRST_SESSION_INPUT")
    policy = install.read_root_file(
        target.release + "/" + install.CONFIG_DIRECTORY + "/policy.json",
        gid=install.READER_GROUP, mode=0o440, maximum=install.POLICY_LIMIT)
    install.validate_policy(policy)
    template = install.read_root_file(
        target.release + "/deploy/db/owner-beta-daily-first-session.sql",
        maximum=64 * 1024).decode("ascii")
    replacements = {
        "__APPLY__": "true" if apply else "false",
        "__OLD_CONFIG_HEX__": install.canonical(old).hex(),
        "__NEW_CONFIG_HEX__": install.canonical(new).hex(),
        "__POLICY_HEX__": policy.hex(),
        "__CAPACITY__": install.control_number(values["MAX_JOBS_PER_OWNER"], 2**31 - 1),
        "__LEASE__": install.control_number(values["PRICE_BETA_LEASE_SECS"], 2**31 - 1),
        "__BACKOFF__": install.control_number(values["PRICE_BETA_BACKOFF_SECS"], 2**63 - 1),
    }
    for token, value in replacements.items():
        if template.count(token) != 1:
            install.reject("DAILY_FIRST_SESSION_TEMPLATE")
        template = template.replace(token, value)
    if re.search(r"__[A-Z_]+__", template):
        install.reject("DAILY_FIRST_SESSION_TEMPLATE")
    return template


@contextlib.contextmanager
def pristine_guard(install, data_root, config):
    # Keep both original poll locks until the database transaction completes.
    install.check_state(data_root, config)
    with install.root_directory(data_root + "/owner-beta-daily") as parent:
        with contextlib.ExitStack() as stack:
            for name, uid in install.SHARED.items():
                fd = stack.enter_context(install.child_directory(parent, name, uid, 10001, 0o750))
                if os.listdir(fd):
                    install.reject("DAILY_FIRST_SESSION_HISTORY")
            for name, (stage, uid) in install.STAGES.items():
                fd = stack.enter_context(install.child_directory(parent, name, uid, uid, 0o700))
                stack.enter_context(install.state_lock(fd, uid))
                if (set(os.listdir(fd)) != {"poll.lock", "checkpoint.json"} or
                        install.read_file(fd, "checkpoint.json", uid, uid, 0o600) !=
                        install.checkpoint(stage, config)):
                    install.reject("DAILY_FIRST_SESSION_HISTORY")
            yield


def run(ops, operation, snapshot, *, apply):
    source, target = operation.source, operation.target
    ops.require(source.commit != target.commit, "DAILY_FIRST_SESSION_SOURCE")
    ops.stopped(operation.docker, operation.ownership, snapshot)
    source.values, source.overlays = ops.read_context(source.release, source.commit, "disable")
    ops.require(all(source.values[k] == target.values[k] for k in (
        "LAGRANGE_DATA_DIR", "LAGRANGE_ARTIFACTS_DIR", "LAGRANGE_RUNTIME_SECRET_DIR",
        "POSTGRES_DB", "MAX_JOBS_PER_OWNER", "PRICE_BETA_LEASE_SECS", "PRICE_BETA_BACKOFF_SECS")),
        "DAILY_FIRST_SESSION_CONTEXT")
    config, _ = ops.install.verify_config(target.release, target.commit)
    sql = render(ops.install, source, target, target.values, apply=apply)
    ops.install.reader_guard(operation.install_root, target.release, "owner_only", reserve=False)
    ops.require(operation.database.status(target.values) == "DISABLED", "DAILY_FIRST_SESSION_DISABLED")
    with pristine_guard(ops.install, target.values["LAGRANGE_DATA_DIR"], config):
        ops.stopped(operation.docker, operation.ownership, snapshot)
        result = operation.database.execute(target.values, sql, {
            "DAILY_FIRST_SESSION: REBOUND_DISABLED" if apply else "DAILY_FIRST_SESSION: PREFLIGHT"})
        ops.stopped(operation.docker, operation.ownership, snapshot)
    return result.split(": ", 1)[1]
