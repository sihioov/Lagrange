"""CLI entry for the backtest worker (plan Todo 20).

    python -m backtest_worker run --request request.json \
        --output-dir artifacts --status-path run_status.json [--scratch dir]

A queue consumer (Todo 21) drives `Worker().run(...)` in-process or shells out
to this entrypoint; the isolated NT child is `python -m backtest_worker.simulate`
(never invoked directly by callers).
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

from .worker import Worker


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Lagrange Station NT backtest worker")
    sub = parser.add_subparsers(dest="command", required=True)

    run = sub.add_parser("run", help="run one isolated backtest and normalize its results")
    run.add_argument("--request", required=True, type=Path)
    run.add_argument("--output-dir", required=True, type=Path)
    run.add_argument("--status-path", required=True, type=Path)
    run.add_argument("--scratch", type=Path, default=None)
    run.add_argument("--keep-run-dir", action="store_true")
    args = parser.parse_args(argv)

    request = json.loads(args.request.read_text(encoding="utf-8"))
    outcome = Worker(scratch=args.scratch, keep_run_dir=args.keep_run_dir).run(
        request, output_dir=args.output_dir, status_path=args.status_path
    )
    print(json.dumps(outcome.to_dict(), sort_keys=True))
    return 0 if outcome.state == "SUCCEEDED" else 1


if __name__ == "__main__":
    sys.exit(main())
