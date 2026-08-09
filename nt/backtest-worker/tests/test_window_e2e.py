"""A window that is a strict SUBSET of the dataset, through the real engine.

`test_window.py` pins the row filter and the runner's unit tests pin the
bounds, but neither exercises the middle: a run whose window discards data the
dataset actually holds. The one end-to-end path that carried a window before
this file requested 2020-01-01..2020-12-31, which contains the whole synthetic
dataset -- so the filter was a no-op exactly where it needed proving.

The property under test is not "the dates in the output look right". Fills are
produced by decisions, and a decision made on a pre-window event that fills on
the window's first bar lands an IN-WINDOW date on an OUT-OF-WINDOW decision --
which every date assertion would happily accept. `_materialize_quotes` gets the
filtered rows, but `build_catalog` builds from the FULL curated root and the
session-event streams are loaded from that catalog, so the leak is structurally
possible and has to be measured rather than assumed.
"""
from __future__ import annotations

import json
import uuid
from pathlib import Path

import pytest

from test_worker import build_dataset, make_request

from backtest_worker.worker import Worker


def _dataset_dates(dataset: Path) -> list[str]:
    import pyarrow.parquet as pq

    dates: set[str] = set()
    for path in sorted(dataset.rglob("bars.parquet")):
        table = pq.read_table(path)
        dates.update(table.column("trading_date").cast("string").to_pylist())
    return sorted(dates)


def _run(tmp_path: Path, dataset: Path, window: tuple[str, str] | None, tag: str) -> dict:
    request = make_request(dataset, str(uuid.uuid4()))
    if window is not None:
        request["start_date"], request["end_date"] = window
    scratch = tmp_path / f"scratch-{tag}"
    scratch.mkdir()
    output_dir = tmp_path / f"artifacts-{tag}"
    outcome = Worker(scratch=scratch).run(
        request, output_dir, tmp_path / f"status-{tag}.json"
    )
    assert outcome.state == "SUCCEEDED", f"worker failed: {outcome.error}"
    return json.loads((output_dir / "result.json").read_text(encoding="utf-8"))


@pytest.fixture(scope="module")
def dataset(tmp_path_factory) -> Path:
    return build_dataset(tmp_path_factory.mktemp("window-e2e") / "dataset")


def test_a_sub_window_run_stays_inside_its_window(tmp_path, dataset):
    """Every dated row the run publishes falls inside the requested window.

    The window is taken from the dataset's own trading days -- a middle slice,
    so there is real data discarded on BOTH sides. A window chosen by calendar
    guesswork could silently coincide with the dataset's full extent, which is
    exactly the hole this file exists to close.
    """
    dates = _dataset_dates(dataset)
    assert len(dates) > 30, f"dataset too short to slice meaningfully: {len(dates)}"
    start, end = dates[len(dates) // 3], dates[2 * len(dates) // 3]
    assert start > dates[0] and end < dates[-1], "window must be a strict subset"

    result = _run(tmp_path, dataset, (start, end), "sub")

    for section in ("equity", "fills", "orders"):
        for row in result.get(section, []):
            ts = row.get("ts") or row.get("trading_date") or ""
            day = ts[:10]
            if not day:
                continue
            assert start <= day <= end, (
                f"{section} row dated {day} is outside the requested "
                f"window [{start}, {end}]"
            )

    equity = result.get("equity", [])
    assert equity, "a windowed run must still produce an equity curve"
    assert equity[0]["ts"][:10] == start, (
        f"the equity curve must begin at the window start, not {equity[0]['ts'][:10]}"
    )
    assert equity[-1]["ts"][:10] == end

    benchmark = result.get("benchmark", [])
    if benchmark:
        assert benchmark[0]["ts"][:10] == start, (
            "the benchmark must be based at the window start, or a windowed run "
            "is compared against a return it never had the chance to earn"
        )


def test_a_windowed_run_differs_from_the_full_run(tmp_path, dataset):
    """The window must actually change the result.

    Without this, every assertion above would still pass if the window were
    ignored and the dataset happened to start and end where the window does.
    """
    dates = _dataset_dates(dataset)
    start, end = dates[len(dates) // 3], dates[2 * len(dates) // 3]

    windowed = _run(tmp_path, dataset, (start, end), "cmp-win")
    full = _run(tmp_path, dataset, None, "cmp-full")

    assert len(windowed["equity"]) < len(full["equity"]), (
        "a strict sub-window produced as many equity points as the full run: "
        "the window was not applied"
    )
    assert full["equity"][0]["ts"][:10] == dates[0]


def test_no_decision_is_carried_in_from_before_the_window(tmp_path, dataset):
    """The leak this file was written for.

    `build_catalog` builds from the full curated root and the session-event
    streams are read from that catalog, so events preceding the window can
    reach the strategy even though its quotes were filtered. A decision made on
    one of those events fills on the window's first tradable bar -- an
    in-window DATE carrying an out-of-window DECISION, which no date assertion
    can see.

    A run over a window is compared against a run over a dataset physically cut
    to that window. The second cannot see pre-window events because they do not
    exist. If windowing is honest, the two agree.
    """
    import shutil

    import pyarrow as pa
    import pyarrow.parquet as pq

    dates = _dataset_dates(dataset)
    start, end = dates[len(dates) // 3], dates[2 * len(dates) // 3]

    # A dataset that physically contains only the window.
    cut = tmp_path / "cut"
    shutil.copytree(dataset, cut)
    for path in sorted(cut.rglob("bars.parquet")):
        table = pq.read_table(path)
        col = table.column("trading_date").cast("string").to_pylist()
        keep = [i for i, d in enumerate(col) if start <= d <= end]
        # Typed explicitly: a year partition entirely outside the window keeps
        # nothing, and pyarrow infers `null` for an empty Python list, which
        # `take` has no kernel for.
        pq.write_table(table.take(pa.array(keep, type=pa.int64())), path)

    windowed = _run(tmp_path, dataset, (start, end), "leak-win")
    physically_cut = _run(tmp_path, cut, None, "leak-cut")

    assert [r["ts"] for r in windowed["equity"]] == [
        r["ts"] for r in physically_cut["equity"]
    ], "the windowed run and the physically-cut run disagree on their timeline"

    def _fills(result):
        return [
            (f.get("ts", "")[:10], f.get("instrument_id"), f.get("quantity"), f.get("side"))
            for f in result.get("fills", [])
        ]

    assert _fills(windowed) == _fills(physically_cut), (
        "the windowed run traded differently from a run that physically cannot "
        "see pre-window data -- something before the window reached the strategy"
    )
