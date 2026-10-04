#!/usr/bin/bash -p
set -eu

wrapper_path=${BASH_SOURCE[0]}
wrapper_dir=${wrapper_path%/*}
if [ "$wrapper_dir" = "$wrapper_path" ]; then
  wrapper_dir=.
fi
physical_dir=$(CDPATH= cd -P -- "$wrapper_dir" && pwd -P) || exit 1
exec /usr/bin/python3 -I "$physical_dir/lib/market-stream-grant.py" "$@"
