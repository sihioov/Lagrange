#!/usr/bin/bash -p
# --inspect --expected-commit SHA --grant-id UUID --approval-input PATH
#   --approval-sha256 SHA256: existing pinned grant evidence, read-only DB check.
# Inspection is not installation, revocation, activation, or a market connection.
set -eu

wrapper_path=${BASH_SOURCE[0]}
wrapper_dir=${wrapper_path%/*}
if [ "$wrapper_dir" = "$wrapper_path" ]; then
  wrapper_dir=.
fi
physical_dir=$(CDPATH= cd -P -- "$wrapper_dir" && pwd -P) || exit 1
exec /usr/bin/python3 -I "$physical_dir/lib/market-stream-grant.py" "$@"
