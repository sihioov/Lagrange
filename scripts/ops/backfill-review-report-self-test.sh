#!/usr/bin/env bash
# Focused local test for the non-approving backfill review report.
set -euo pipefail

script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
report="$script_dir/backfill-review-report.sh"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/lagrange-backfill-review.XXXXXX")
trap 'rm -rf -- "$tmp"' EXIT

[ -x "$report" ] || { echo 'BACKFILL_REVIEW_SELF_TEST: report is not executable' >&2; exit 1; }
bash -n "$report"
plan=$(
  "$report" --start 2026-08-18 --end 2026-08-18 \
    --state-file "$tmp/state.tsv" --data-root "$tmp/data" --plan
)
grep -Fq 'PLAN_ONLY: no production file read, write, or external service action made' <<<"$plan"

python3 - "$tmp" <<'PY'
from __future__ import annotations

import hashlib
import json
import pathlib
import sys

root = pathlib.Path(sys.argv[1])
data = root / "data"
for provider in ("kis", "kis-normalized"):
    manifest = data / "raw" / "manifests" / f"provider={provider}" / "market=kr" / "manifest.jsonl"
    manifest.parent.mkdir(parents=True, exist_ok=True)
    manifest.write_text(
        json.dumps({"provider": provider, "market": "kr", "date": "2026-08-18"}) + "\n",
        encoding="utf-8",
    )

identity = "0123456789abcdef" * 4
(root / "state.tsv").write_text(
    "\n".join(
        (
            f"LAGRANGE_BACKFILL_STATE_V3\t{identity}",
            f"2026-08-18\tRUNNING\t{identity}",
            f"2026-08-18\tPUBLISHED\t{identity}",
        )
    )
    + "\n",
    encoding="ascii",
)

relative = "bars/market=kr/symbol=069500.KRX/year=2026/version=1/bars.parquet"
artifact = data / "curated" / relative
artifact.parent.mkdir(parents=True, exist_ok=True)
artifact.write_bytes(b"PAR1fixturePAR1")
artifact_hash = hashlib.sha256(artifact.read_bytes()).hexdigest()

manifest = data / "curated" / "datasets" / "krx_eod_bars" / "version=1" / "manifest.json"
manifest.parent.mkdir(parents=True, exist_ok=True)
body = {
    "dataset_id": "krx_eod_bars",
    "version": 1,
    "capability": "PRICE_RETURN_ONLY",
    "created_at": "2026-08-18T00:00:00Z",
    "source_batches": [
        {
            "batch_id": "00000000-0000-4000-8000-000000000001",
            "bars_file": "bars.json",
            "bars_hash": "sha256:" + "a" * 64,
            "actions_file": "corporate-actions.json",
            "actions_hash": "sha256:" + "b" * 64,
        }
    ],
    "artifacts": [
        {
            "path": relative,
            "sha256": "sha256:" + artifact_hash,
            "size_bytes": artifact.stat().st_size,
            "schema": "bars-v1",
        }
    ],
    "bar_count": 1,
    "action_count": 0,
}
canonical = json.dumps(body, ensure_ascii=False, separators=(",", ":"))
body["content_hash"] = "sha256:" + hashlib.sha256(canonical.encode()).hexdigest()
manifest.write_text(json.dumps(body, separators=(",", ":")), encoding="utf-8")
PY

complete=$(
  "$report" --start 2026-08-18 --end 2026-08-18 \
    --state-file "$tmp/state.tsv" --data-root "$tmp/data" --check
)
grep -Fq 'BACKFILL_REVIEW: CURATED_CANDIDATE_FOUND_UNAPPROVED' <<<"$complete"
grep -Fq 'DB_READY=NOT_CHECKED' <<<"$complete"
if grep -Eq 'READY registration succeeded|DATASET_READY|BACKFILL_REVIEW: PASS' <<<"$complete"; then
  echo 'BACKFILL_REVIEW_SELF_TEST: report made an approval/readiness claim' >&2
  exit 1
fi

sed -i $'s/2026-08-18\tPUBLISHED\t/2026-08-18\tRUNNING\t/' "$tmp/state.tsv"
if "$report" --start 2026-08-18 --end 2026-08-18 \
  --state-file "$tmp/state.tsv" --data-root "$tmp/data" --check >"$tmp/incomplete.out" 2>&1; then
  echo 'BACKFILL_REVIEW_SELF_TEST: incomplete state unexpectedly passed' >&2
  exit 1
fi
grep -Fq 'BACKFILL_REVIEW: BLOCKED' "$tmp/incomplete.out"

echo 'BACKFILL_REVIEW_SELF_TEST: PASS (local-only; no DB/provider/service action)'
