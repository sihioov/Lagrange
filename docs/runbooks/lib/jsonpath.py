"""Reads one field out of a JSON object, for the runbook assertions.

Exists because `jq` is not installed on every host that has to run these
runbooks, and a runbook that cannot run on the machine in front of you during
an incident is not a runbook. Python is already a hard dependency here -- the
node itself is Python -- so this adds nothing new.

Supported paths, a deliberate subset of jq's syntax so the assertions read the
same in both shell twins:

    .field                a key
    .a.b                  nested keys
    .a[0]                 an array element
    .a | length           an array's length

Exit codes are the contract:

    0  the path resolved; the value is printed
    1  the path does NOT exist

Non-existence must be an ERROR rather than an empty string. An assertion that
compares "" to "" passes, and a runbook full of assertions against fields that
were renamed away would report success while checking nothing at all.
"""
from __future__ import annotations

import json
import sys


def resolve(document: object, path: str) -> object:
    expr = path.strip()

    want_length = False
    if "|" in expr:
        expr, _, suffix = expr.partition("|")
        if suffix.strip() != "length":
            raise KeyError(f"unsupported operation: {suffix.strip()}")
        want_length = True
        expr = expr.strip()

    node = document
    for raw in expr.lstrip(".").split("."):
        if not raw:
            continue
        key, _, index = raw.partition("[")
        if key:
            if not isinstance(node, dict) or key not in node:
                raise KeyError(path)
            node = node[key]
        if index:
            position = int(index.rstrip("]"))
            if not isinstance(node, list) or position >= len(node):
                raise KeyError(path)
            node = node[position]

    if want_length:
        if not isinstance(node, (list, dict, str)):
            raise KeyError(path)
        return len(node)
    return node


def render(value: object) -> str:
    # Rendered the way the shell twin's `jq -r` would, so an assertion's
    # expected value is written once and works in both.
    if value is True:
        return "true"
    if value is False:
        return "false"
    if value is None:
        return "null"
    if isinstance(value, (dict, list)):
        return json.dumps(value, separators=(",", ":"), sort_keys=True)
    return str(value)


def main(argv: list[str]) -> int:
    if len(argv) != 2:
        print("usage: jsonpath.py <path>  (json on stdin)", file=sys.stderr)
        return 2
    try:
        document = json.loads(sys.stdin.read())
    except json.JSONDecodeError:
        return 1
    try:
        value = resolve(document, argv[1])
    except (KeyError, ValueError, IndexError):
        return 1
    print(render(value))
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
