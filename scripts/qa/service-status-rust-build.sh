#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$repo_root/scripts/qa/service-status-qa-lib.sh"
qa_root="${SERVICE_STATUS_QA_ROOT:-}"
action="${1:-start}"
service_status_validate_qa_root "$qa_root"

for command in systemctl; do
  command -v "$command" >/dev/null || { echo "required QA command is unavailable: $command" >&2; exit 2; }
done
if [[ "$action" == wait ]]; then
  unit_file="$qa_root/rust-build/unit-name.txt"
  result_file="$qa_root/rust-build/exit-status.txt"
  metrics_file="$qa_root/rust-build/unit-metrics.log"
  if [[ ! -f "$unit_file" ]]; then
    echo "No WP-5 Rust build unit is recorded for this QA root." >&2
    exit 2
  fi
  IFS= read -r unit <"$unit_file"
  if [[ ! "$unit" =~ ^lagrange-service-status-wp5-rust-build-[0-9]+\.service$ ]]; then
    echo "The recorded WP-5 Rust build unit name is invalid." >&2
    exit 2
  fi
  lock_owner="rust-example-build:$unit"
  recorded_owner="$(sed -n 's/^owner=//p' "$qa_root/slot.lock/owner.txt" 2>/dev/null || true)"
  if [[ "$recorded_owner" != "$lock_owner" ]]; then
    echo "The serial QA slot lock does not match the recorded Rust example build unit." >&2
    exit 2
  fi
  for ((attempt = 0; attempt < 1080; attempt += 1)); do
    active_state="$(systemctl --user show "$unit" --property=ActiveState --value 2>/dev/null || true)"
    if [[ -s "$result_file" && ( "$active_state" == inactive || -z "$active_state" || "$active_state" == not-found ) ]]; then
      IFS= read -r exit_status <"$result_file"
      service_status_capture_unit rust-example-build "$unit" "$metrics_file"
      if [[ "$exit_status" == 0 ]]; then
        service_status_release_slot_lock "$qa_root" "$lock_owner"
        printf 'Rust QA example compile completed successfully: %s; evidence=%s\n' "$unit" "$metrics_file"
        exit 0
      fi
      service_status_release_slot_lock "$qa_root" "$lock_owner"
      printf 'The bounded Rust example build failed: unit=%s exit=%s log=%s metrics=%s\n' \
        "$unit" "$exit_status" "$qa_root/rust-build/cargo-build.log" "$metrics_file" >&2
      if [[ "$exit_status" =~ ^[0-9]+$ ]] && (( exit_status > 0 && exit_status <= 255 )); then
        exit "$exit_status"
      fi
      exit 125
    fi
    if [[ "$active_state" == failed ]]; then
      original_exit="$(systemctl --user show "$unit" --property=ExecMainStatus --value 2>/dev/null || true)"
      service_status_capture_unit rust-example-build "$unit" "$metrics_file"
      service_status_release_slot_lock "$qa_root" "$lock_owner"
      printf 'The bounded Rust example build unit failed: unit=%s log=%s metrics=%s\n' \
        "$unit" "$qa_root/rust-build/cargo-build.log" "$metrics_file" >&2
      if [[ "$original_exit" =~ ^[0-9]+$ ]] && (( original_exit > 0 && original_exit <= 255 )); then
        exit "$original_exit"
      fi
      exit 125
    fi
    if [[ "$active_state" == inactive || "$active_state" == not-found || -z "$active_state" ]]; then
      service_status_capture_unit rust-example-build "$unit" "$metrics_file"
      service_status_release_slot_lock "$qa_root" "$lock_owner"
      printf 'The bounded Rust example build exited without a result receipt: unit=%s state=%s log=%s metrics=%s\n' \
        "$unit" "${active_state:-missing}" "$qa_root/rust-build/cargo-build.log" "$metrics_file" >&2
      exit 1
    fi
    if [[ "$active_state" != active && "$active_state" != activating && "$active_state" != deactivating ]]; then
      echo "The bounded WP-5 Rust build unit is missing or has an unexpected state." >&2
      exit 1
    fi
    service_status_capture_unit rust-example-build-sample "$unit" "$metrics_file"
    sleep 1
  done
  echo "Timed out waiting for the bounded WP-5 Rust build unit." >&2
  exit 1
fi
if [[ "$action" != start ]]; then
  echo "Choose start or wait for the WP-5 Rust compile." >&2
  exit 2
fi
for command in cargo docker ionice pgrep systemd-run systemctl; do
  command -v "$command" >/dev/null || { echo "required QA command is unavailable: $command" >&2; exit 2; }
done
service_status_assert_no_runtime_sets "$qa_root"
service_status_preflight "$qa_root" rust-example-build 1792
owned_containers="$(docker ps --all --quiet --filter label=com.docker.compose.project=lagrange-service-status-wp5)"
if [[ -n "$owned_containers" ]]; then
  echo "The WP-5 QA Compose project exists; stop and remove its owned containers before compiling." >&2
  exit 2
fi

mkdir -m 700 -p "$qa_root/rust-build" "$qa_root/rust-build/pids"
unit="lagrange-service-status-wp5-rust-build-$$.service"
log_file="$qa_root/rust-build/cargo-build.log"
pid_file="$qa_root/rust-build/pids/cargo-build.pid"
result_file="$qa_root/rust-build/exit-status.txt"
ownership_file="$qa_root/rust-build/ownership.txt"
unit_load_state="$(systemctl --user show "$unit" --property=LoadState --value 2>/dev/null || true)"
if [[ -n "$unit_load_state" && "$unit_load_state" != not-found ]]; then
  echo "The bounded Rust build unit name already exists." >&2
  exit 2
fi
rm -f "$pid_file" "$result_file"
lock_owner="rust-example-build:$unit"
service_status_acquire_slot_lock "$qa_root" "$lock_owner"

if systemd-run --user --quiet --expand-environment=no --unit="$unit" \
  --property=Type=exec \
  --property="ExecStartPre=/usr/bin/bash $repo_root/scripts/qa/service-status-unit-receipt.sh start $qa_root/rust-build/lifecycle-receipt.tsv 1610612736 $unit" \
  --property="ExecStopPost=/usr/bin/bash $repo_root/scripts/qa/service-status-unit-receipt.sh end $qa_root/rust-build/lifecycle-receipt.tsv 1610612736 $unit" \
  --property=TimeoutStopSec=30s \
  --property=WorkingDirectory="$repo_root" \
  --property=MemoryMax=1536M --property=MemorySwapMax=0 --property=MemoryAccounting=yes \
  --property=CPUWeight=10 --property=OOMPolicy=kill --property=RuntimeMaxSec=90min \
  --property="StandardOutput=append:$log_file" \
  --property="StandardError=append:$log_file" \
  -- env PATH="$PATH" \
    CARGO_BUILD_JOBS=1 \
    CARGO_INCREMENTAL=0 \
    CARGO_PROFILE_DEV_DEBUG=0 \
    CARGO_PROFILE_TEST_DEBUG=0 \
    bash -c 'pid_file="$1"; result_file="$2"; printf "%s\n" "$$" >"$pid_file"; shift 2; "$@"; result=$?; printf "%s\n" "$result" >"$result_file"; exit "$result"' \
    service-status-rust-build "$pid_file" "$result_file" \
    ionice -c 3 -n 7 cargo build -p api-server --example service-status-browser-harness --offline; then
  :
else
  result=$?
  service_status_capture_unit rust-example-build "$unit" "$qa_root/rust-build/unit-metrics.log"
  printf 'Rust example build did not start or was capped: unit=%s exit=%s log=%s metrics=%s\n' \
    "$unit" "$result" "$log_file" "$qa_root/rust-build/unit-metrics.log" >&2
  service_status_release_slot_lock "$qa_root" "$lock_owner"
  exit "$result"
fi
printf '%s\n' "$unit" >"$qa_root/rust-build/unit-name.txt"
for ((attempt = 0; attempt < 30; attempt += 1)); do
  if [[ -s "$pid_file" ]]; then break; fi
  if ! systemctl --user is-active --quiet "$unit"; then
    echo "The bounded QA build unit exited before recording its PID." >&2
    service_status_capture_unit rust-example-build "$unit" "$qa_root/rust-build/unit-metrics.log"
    service_status_release_slot_lock "$qa_root" "$lock_owner"
    exit 1
  fi
  sleep 1
done
if [[ ! -s "$pid_file" ]]; then
  echo "The bounded QA build unit did not record its PID." >&2
  service_status_capture_unit rust-example-build "$unit" "$qa_root/rust-build/unit-metrics.log"
  if systemctl --user is-active --quiet "$unit"; then
    printf 'The unit may still be active; its serial QA slot lock remains held for review: unit=%s\n' "$unit" >&2
  else
    service_status_release_slot_lock "$qa_root" "$lock_owner"
  fi
  exit 1
fi
build_pid="$(cat "$pid_file")"
if [[ ! "$build_pid" =~ ^[1-9][0-9]*$ ]]; then
  echo "The bounded QA build unit recorded an invalid PID." >&2
  exit 1
fi
printf 'unit=%s\npid=%s\npid_file=%s\nresult_file=%s\nlog=%s\ncap=1536M\n' \
  "$unit" "$build_pid" "$pid_file" "$result_file" "$log_file" >"$ownership_file"
printf 'Started bounded QA build unit %s with PID %s\n' "$unit" "$build_pid"
