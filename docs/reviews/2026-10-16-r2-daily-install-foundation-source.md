# R2 Wave72 I6A installation foundation source review

Roadmap R2-04/09. Baseline eb3a938 after complete CI38831683987 acceptance.
Direct coordinator review, not an independent integrated review. Eleven owned
paths: seven new helper/config/fixture files, three Rust tests and one policy CI
step. Existing installer, Compose, image manifest, DB/provider and runtime
sources are unchanged. No operational configuration or activation is implied.

The closed helper rejects duplicate/unknown JSON fields, invalid identities,
unbounded/overflowing budgets, noncanonical paths and nonmatching code/policy.
Entitlement reference also obeys schema0060's1..512character/no-control bound
before staging, retaining the runtime4096UTF8byte maximum. Three negative
variants exercise excessive characters, newline and Unicode control input.
The canonical policy pins the existing approved historical v3 registry/base,
ETF11 universe, price-only/cash-action rules and current fixed limits.
Operational entitlement, first session and budgets remain explicit inputs.
The checked-in Owner/commit/entitlement fixture is synthetic.

Config is copied once to new root:10001 0440 files below0750 protected immutable
release storage. Root-owned ancestors/ACLs, no-follow file descriptors, regular
single-link files, size and before/after identities are checked. Writes are
complete and fsynced. This helper does not switch the installed current link.

State is a protected sibling under the data root, not a child of writable
artifact storage. UID10002 owns candidate/producer state;10003 owns admission/
frontier/admitter state;10001 has no write root. Shared roots0750, private0700,
checkpoint/lock0600. Fresh children remain behind root0700 staging until a
real renameat2 NOREPLACE publishes the complete root. Existing valid advanced
state is reused byte-for-byte; foreign, partial, busy or crash-residue state
fails without reset/repair. Uncertain publication retains protected staging.
Installer advisory locks only coordinate conforming runtime users.

The monotonic root0600 reader floor is reserved before first ON activation.
Once present, even OFF targets must carry the exact reader declaration for
schema60/61/62 and historical+daily input preservation. Missing old-reader or
corrupt-floor paths fail; the floor is not erased after a later failure.
The floor is conservative installation evidence, not proof of DB history.
I6B must wire manifest/commit/config preflight before the current-link switch;
compatible OFF runtime drain/rollback remains a separate integration obligation.

Sixteen written hosted root fixtures exercise real UID writes/reads, protected
ancestors and POSIX ACLs, exact immutable bytes, JSON/limits/link/mode/owner/FIFO
rejection, actual no-replace, advanced reuse, busy locks, crash residue and
reader rollback, plus closed CLI diagnostics. Publication failure injection is
a deterministic error-path test, not a real power-loss claim. Three Rust tests
bind policy/approved registry, initial checkpoint bytes and config/reader schema.
Prior definitions are retained. No Python/source execution, Cargo, compiler,
tests, root, Docker, DB, provider or production operation ran locally.
Only static text/hash/name/whitespace and scoped standalone rustfmt2024.

Exact-candidate hosted compilation/strict lint,3Rust and16root cases plus all
carried Rust/PG/Web/cleanup gates must pass before bounded I6A acceptance.
No source or executable acceptance yet. I6B integration/diagrams, actual approved
base threeUID first/continuation flow, independent integrated review,G4,Owner,
three actual trading days and finalC_R2 remain open.

| File | Lines | SHA256 |
| --- | ---: | --- |
| scripts/ops/lib/owner-beta-daily-release.py | 596 | feb6b08066221b419b4eceb8bb20397efd9fbace04da815520d3081997bb9732 |
| scripts/ops/owner-beta-daily-release-self-test.py | 370 | 466d966f15507d0a82b68a93c346319aee9ce2c6cdfbf2e202e1ce6a94b8556c |
| configs/evidence/owner-beta-daily-price-policy-v1.json | 1 | e9149ef56170877e5e9646f1fff1c370b3a34bf2c8b8ee8272bc30bd5fa8779c |
| configs/evidence/owner-beta-daily-reader-v1.json | 1 | 1675f845fa3a4bd83fd1f425ce76f0a75cbc89decc8e6ab8997cec11112fb246 |
| tests/fixtures/owner-beta-daily-install/config.json | 1 | 62266b8741ba90e9d563982b645bb63cde51956dc4b69f1596e89b22b41ec277 |
| tests/fixtures/owner-beta-daily-install/producer-checkpoint.json | 1 | dada69f9f2f1accd0b152018cdb1dc83b6a512043063fb14fe6b2e57f4150471 |
| tests/fixtures/owner-beta-daily-install/admitter-checkpoint.json | 1 | 0dae0f193f0dad68264aa57e938611a959660cd1ae47565b736a2b2243c307d9 |
| crates/market-data/tests/daily_price_only.rs | 2435 | 2f872d857d424770b5af1e75f25d10f7c2b7b86a1300cf8cf843a63f08733bd8 |
| crates/job-queue/src/owner_beta/runner/daily_local_state.rs | 1005 | d104c2a2a1811d56fc74fe7f8b42c93f034dad5ed367338a669987b5d191279b |
| crates/job-queue/src/owner_beta/runner/daily_ingest.rs | 1261 | 2bb8ccc61fe4cb8a560c8eda81131e8fdb7b02ea82453fc5535d17b9ba414e4e |
| .github/workflows/ci.yml | 236 | 5f73a22dd008ca5a02ab8584f8b8337b7ba8ab9a4da4a13af2f34e76838a126a |
