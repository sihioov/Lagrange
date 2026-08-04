# ADR-0001: Python polars pin correction (0.54.x is unsatisfiable on PyPI)

- **Status:** Accepted
- **Date:** 2026-08-05
- **Deciders:** Lagrange Station implementer (Todo 1 worker), orchestrator verification
- **Replaces:** approved toolchain line "Polars 0.54.x" (`.omo/drafts/lagrange-station-implementation.md`, line 40, dated 2026-08-04) **for the Python side only**

## Context

The approved toolchain decision pinned "Polars 0.54.x" as a single line applying to
both the Rust research crates (`crates/factor-engine`, `crates/selector`) and the
Python/NautilusTrader project (`nt/`). During Todo 1 bootstrap, `uv lock --project nt`
failed to resolve `polars>=0.54,<0.55`:

```
x No solution found when resolving dependencies:
  Because only the following versions of polars are available:
          polars<0.54
          polars>0.55
      and your project depends on polars>=0.54,<0.55, we can conclude that
      your project's requirements are unsatisfiable.
```

### Evidence (verified 2026-08-05 against live package indexes)

1. **PyPI `polars` has no 0.54.x release.** `pip index versions polars` reports
   `polars (1.43.2)` with available versions `1.43.2 ... 1.0.0, 0.20.31, 0.20.30, ...
   0.7.x`. The version line jumps from `0.20.31` to `1.0.0`; the range `[0.54, 0.55)`
   is empty.
2. **`nautilus_trader==1.231.0` does not depend on polars.** Its `requires_dist`
   (PyPI JSON, `https://pypi.org/pypi/nautilus-trader/1.231.0/json`) is:
   `click<9.0.0,>=8.4.1`, `fsspec==2026.2.0`, `msgspec<1.0.0,>=0.21.1`,
   `numpy>=1.26.4`, `pandas<4.0.0,>=2.3.3`, `portion>=2.6.1`, `pyarrow>=25.0.0`,
   `pytz>=2026.2`, `tqdm<5.0.0,>=4.68.4`, `tzdata>=2026.3` (plus extras; `uvloop`
   only off-win32). NT's Parquet/catalog stack is **pandas + pyarrow**, not polars.
3. **crates.io `polars` 0.54.x exists.** `cargo search polars` reports latest
   `0.55.1`; the approved Rust line `0.54.x` is a valid, satisfiable dependency line
   for the Rust crates.

The approved decision text itself anticipates this class of correction: "the current
compatible stable baselines independently verified from official release/package
metadata on 2026-08-04 ... [pin changes are] reversible ... through an explicit
compatibility upgrade ADR and golden regression" (draft line 40).

## Decision

- **Remove the Python polars pin from `nt/pyproject.toml`.** Keep
  `nautilus_trader==1.231.0`, the `pytest` dev dependency group, and
  `[tool.uv] package = false`.
- The Python-side data stack is provided by NT 1.231.0's own pinned dependencies
  (pandas + pyarrow). No Python polars dependency is required by the documented
  nt/ work (custom catalog data and strategies are consumed through NT APIs).
- **Rust polars 0.54.x remains approved** for `crates/factor-engine` and
  `crates/selector` and stays untouched. It will be pinned by `Cargo.lock` when
  those crates first depend on it (Todos 15/16).
- `scripts/check-pins.*` no longer assert a Python polars pin; the
  `nautilus_trader==1.231.0` assertion is retained.

## Consequences

- **Positive:** `uv lock --project nt` becomes satisfiable and produces
  `nt/uv.lock`; `uv run --project nt pytest -q` can genuinely pass; the Todo 1
  uv gates exit 0 instead of reporting BLOCKED_ENVIRONMENT.
- **Negative:** a Python polars dependency, if ever needed by a later todo, must be
  introduced with its own ADR (0.20.x or 1.x) and pinned via `uv.lock`.
- **Required reading:** workers for Todos 13+ (custom NT catalog data) must read
  this ADR before assuming a Python polars line exists; `Cargo.toml` header comment
  and `scripts/check-pins.*` cite this ADR so the Rust/Python split is not re-litigated.
- **Rust polars 0.54.x** for `crates/factor-engine`/`crates/selector` is unaffected
  and is asserted via `Cargo.lock` when first consumed.
