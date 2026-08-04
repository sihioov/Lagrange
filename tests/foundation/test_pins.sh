#!/usr/bin/env bash
set -euo pipefail

# Given: a repository that declares the approved toolchain pins.
# When: the cross-platform pin check runs.
# Then: it validates the checked-in pins without requiring installed toolchains.
"$(dirname "$0")/../../scripts/check-pins.sh" --manifest-only

# Given: the documented workspace topology.
# When: the foundation validator runs.
# Then: every required workspace boundary is present.
"$(dirname "$0")/../../scripts/validate-foundation.sh"
