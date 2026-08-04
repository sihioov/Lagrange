#!/usr/bin/env bash
# check-pins.sh — POSIX twin of scripts/check-pins.ps1 for CI / clean containers.
# Approved pins (draft 2026-08-04 line 40): Rust 1.97.1, CPython 3.12, Node >=24 <25,
# nautilus_trader==1.231.0, polars>=0.54,<0.55.
# Two-sided drift detection: pin FILE vs approved constant, and installed toolchain vs pin FILE.
# Exit 0 when every pin holds; exit 1 NAMING each drift.
# Run from anywhere in the repo; root is resolved as the parent of scripts/.
set -u
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
drifts=()

APPROVED_RUST='1.97.1'
APPROVED_PY='3.12'
APPROVED_NODE='>=24 <25'

# --- Rust --------------------------------------------------------------------
if [ -f "$root/rust-toolchain.toml" ]; then
  pin="$(sed -n 's/.*channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$root/rust-toolchain.toml" | head -n1)"
  if [ -z "$pin" ]; then
    drifts+=("rust-toolchain.toml: no channel= pin found")
  else
    [ "$pin" = "$APPROVED_RUST" ] || drifts+=("rust-toolchain.toml: approved rust pin is $APPROVED_RUST but channel is $pin")
    out="$(rustc --version 2>&1 || true)"
    actual="$(printf '%s' "$out" | sed -n 's/^rustc \([0-9.]*\).*/\1/p')"
    if [ -z "$actual" ]; then
      out="$(rustc +stable --version 2>&1 || true)"
      actual="$(printf '%s' "$out" | sed -n 's/^rustc \([0-9.]*\).*/\1/p')"
    fi
    if [ -z "$actual" ]; then
      drifts+=("rustc: could not read installed version (raw: $out)")
    elif [ "$actual" != "$pin" ]; then
      drifts+=("rustc: pin $pin (rust-toolchain.toml) but installed $actual")
    fi
  fi
else
  drifts+=("rust-toolchain.toml: missing")
fi

# --- Python ------------------------------------------------------------------
if [ -f "$root/.python-version" ]; then
  pin="$(head -n1 "$root/.python-version" | tr -d '[:space:]')"
  [ "$pin" = "$APPROVED_PY" ] || drifts+=(".python-version: approved python pin is $APPROVED_PY but file says $pin")
  out="$(python --version 2>&1 || true)"
  actual="$(printf '%s' "$out" | sed -n 's/^Python \([0-9.]*\).*/\1/p')"
  if [ -z "$actual" ]; then
    drifts+=("python: could not read installed version (raw: $out)")
  elif [ "$actual" != "$pin" ] && [ "${actual#"$pin".}" = "$actual" ]; then
    drifts+=("python: pin $pin (.python-version) but installed $actual")
  fi
else
  drifts+=(".python-version: missing")
fi

# --- Node --------------------------------------------------------------------
if [ -f "$root/package.json" ]; then
# engines.node parsed with sed: static check must not depend on node or on host path translation (host vs container/WSL).
  range="$(sed -n 's/.*"node"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$root/package.json" | head -n1)"
  if [ -z "$range" ]; then
    drifts+=("package.json: engines.node missing (approved: '$APPROVED_NODE')")
  elif [ "$range" != "$APPROVED_NODE" ]; then
    drifts+=("package.json: approved engines.node is '$APPROVED_NODE' but found '$range'")
  fi
  out="$(node --version 2>&1 || true)"
  major="$(printf '%s' "$out" | sed -n 's/^v\([0-9]*\)\..*/\1/p')"
  if [ -z "$major" ]; then
    drifts+=("node: could not read installed version (raw: $out)")
  elif [ -n "$range" ]; then
    min="$(printf '%s' "$range" | sed -n 's/.*>=\([0-9]*\).*/\1/p')"
    max="$(printf '%s' "$range" | sed -n 's/.*<\([0-9]*\).*/\1/p')"
    if { [ -n "$min" ] && [ "$major" -lt "$min" ]; } || { [ -n "$max" ] && [ "$major" -ge "$max" ]; }; then
      drifts+=("node: engines '$range' (package.json) but installed $major.x")
    fi
  fi
else
  drifts+=("package.json: missing (no node pin)")
fi

# --- NautilusTrader / Polars pins (nt project) --------------------------------
if [ -f "$root/nt/pyproject.toml" ]; then
  content="$(cat "$root/nt/pyproject.toml")"
  grep -Eq 'nautilus[_\-]trader[[:space:]]*==[[:space:]]*1\.231\.0' <<<"$content" \
    || drifts+=("nt/pyproject.toml: nautilus_trader not pinned to 1.231.0")
  grep -Eq 'polars[[:space:]]*>=[[:space:]]*0\.54[^[:space:]]*,[[:space:]]*<[[:space:]]*0\.55' <<<"$content" \
    || drifts+=("nt/pyproject.toml: polars not pinned to >=0.54,<0.55")
else
  drifts+=("nt/pyproject.toml: missing (no NT pin)")
fi

if [ "${#drifts[@]}" -gt 0 ]; then
  echo "PIN DRIFT DETECTED:"
  for d in "${drifts[@]}"; do echo "  - $d"; done
  exit 1
fi
echo "ALL PINS OK (rustc/python/node/NT)"
exit 0
