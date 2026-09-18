# Build-cache smoke fixture

This is a deliberately small two-binary Rust workspace. `fixture-lib` uses
the pinned `itoa` registry crate from the repository lockfile; both binaries
use that local crate, the application build script, embedded data, and a
compile-time commit marker. The compile `RUN` prints a phase marker before
each verbose Cargo invocation so the smoke harness can distinguish reuse
inside that one `RUN` from reuse in a later Docker build.

The Dockerfile uses only `build-cache-fixture-*` cache IDs. It is never a
production cache or image contract. The smoke harness uses temporary copies,
temporary image tags, and a fresh cache-mount namespace for its cold case.
Its disposable external-dependency scenario adds the root-pinned `cfg-if`
1.0.4 record and a `cfg_if` macro to a temporary Git branch; the committed
fixture itself intentionally starts with only `itoa` so that scenario proves
new dependency and lockfile compilation while retaining `itoa` reuse.
