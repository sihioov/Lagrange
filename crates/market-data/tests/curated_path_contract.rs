//! Repo-wide guard against the `curated/curated` defect family.
//!
//! `CurateStore::new(root)` takes the `data/` root and appends `curated`
//! itself (`crates/market-data/src/curate.rs`, `CurateStore::curated_dir`).
//! Handing it a path that already ends in `curated` produces
//! `<root>/curated/curated/...`, which never exists — so the reader built on
//! it silently finds nothing, and any test that depends on that reader
//! silently passes. The family has been fixed four times (`8e77c7a`,
//! `733aca2`) and two of those four were invisible to a plain string search:
//! three api-server fixtures *wrote* through the same doubled join, so the
//! suite stayed green in both directions, and `factor_series.rs` built the
//! doubled path in two hops (`join("data/phase0/curated")` then
//! `join("curated/bars/...")`).
//!
//! Neither half below catches all four, so both run:
//!
//! * [`no_curate_store_is_built_from_a_curated_suffixed_path`] reads every
//!   Rust source in the repository and rejects a `CurateStore::new` whose
//!   argument expression contains a `curated`-tailed string literal. This is
//!   the direct one-hop form.
//! * [`the_generated_curated_tree_has_no_nested_curated_directory`] walks a
//!   real generated curated zone and rejects a `curated` directory nested
//!   directly inside another. This is the half that catches a doubled path
//!   assembled over more than one hop, or one created by a fixture writer
//!   rather than read — neither of which any source-text rule can see.
//!
//! The guard lives in `market-data` because `CurateStore` — the contract being
//! guarded — is defined here, and because the source walk reaches the whole
//! repository regardless of which package hosts it. It runs in the ordinary
//! `cargo test --workspace` lane; it needs no new CI job and no new tooling.
//!
//! The source scan follows the `include_str!` contract-test idiom already used
//! by `data-pipelines/collectors/src/worker.rs`
//! (`mod price_recovery_contract_tests`), widened from one file to the tree.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The construction whose argument must never already end in `curated`.
///
/// Matching the suffix also covers the qualified `market_data::CurateStore`
/// and `crate::CurateStore` spellings.
const CONSTRUCTION: &str = "CurateStore::new(";

/// Directory names never scanned: build output and vendored trees.
const SKIPPED_DIRECTORIES: [&str; 4] = [".git", "target", "node_modules", ".venv"];

/// A scan that stops finding sources is a scan that always passes. The
/// repository held 420 Rust sources when this guard was written; the floor is
/// low enough to survive ordinary deletions and high enough that a broken
/// walk cannot pass.
const MINIMUM_RUST_SOURCES: usize = 200;

/// How many lines after the construction the negative existence assertion may
/// appear in for a deliberate `curated/curated` construction to be allowed.
/// The widest legitimate gap today is eight lines.
const NEGATIVE_ASSERTION_WINDOW_LINES: usize = 15;

/// The only legitimate constructions of a doubled curated path: the guard
/// tests that build it precisely to assert it does NOT exist.
///
/// Recorded as `(path, binding)` pairs and asserted to still be *found*, so a
/// scanner that has silently stopped matching cannot pass by finding nothing.
/// This is not the allow mechanism — see [`asserts_absence`] for that. Naming
/// a file here grants it nothing.
const EXPECTED_NEGATIVE_ASSERTIONS: [(&str, &str); 3] = [
    ("crates/job-queue/src/paper_execution.rs", "doubled"),
    ("crates/job-queue/src/paper_valuation.rs", "doubled"),
    ("crates/job-queue/tests/paper_preview.rs", "doubled_path"),
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root above crates/market-data")
        .to_path_buf()
}

fn rust_sources(directory: &Path, found: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()));
    for entry in entries {
        let entry =
            entry.unwrap_or_else(|error| panic!("entry in {}: {error}", directory.display()));
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        if kind.is_dir() {
            if SKIPPED_DIRECTORIES
                .iter()
                .any(|skipped| OsStr::new(skipped) == entry.file_name())
            {
                continue;
            }
            rust_sources(&path, found);
        } else if path.extension() == Some(OsStr::new("rs")) {
            found.push(path);
        }
    }
}

/// The byte index just past a string literal that starts at `quote`.
fn end_of_string_literal(bytes: &[u8], quote: usize) -> Option<usize> {
    let mut index = quote + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'"' => return Some(index + 1),
            _ => index += 1,
        }
    }
    None
}

/// The byte index just past a `'`-introduced token: a char literal, or a
/// lifetime, which is not a literal and consumes only the tick.
fn end_of_tick(bytes: &[u8], tick: usize) -> usize {
    if bytes.get(tick + 1) == Some(&b'\\') {
        let mut index = tick + 2;
        while index < bytes.len() {
            if bytes[index] == b'\'' {
                return index + 1;
            }
            index += 1;
        }
        return bytes.len();
    }
    if bytes.get(tick + 2) == Some(&b'\'') {
        return tick + 3;
    }
    tick + 1
}

/// The argument expression between `open` — the byte just after the `(` of a
/// [`CONSTRUCTION`] — and its matching `)`.
///
/// String and char literals are stepped over so brackets inside them do not
/// count. `None` means the source is unbalanced from here, which the caller
/// reports rather than skipping.
fn balanced_argument(source: &str, open: usize) -> Option<&str> {
    let bytes = source.as_bytes();
    let mut depth = 1usize;
    let mut index = open;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => index = end_of_string_literal(bytes, index)?,
            b'\'' => index = end_of_tick(bytes, index),
            b'(' | b'[' | b'{' => {
                depth += 1;
                index += 1;
            }
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&source[open..index]);
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

/// The first string literal in `expression` whose last path segment is
/// `curated`.
///
/// Only literals count. `CurateStore::new(&config.curated_root)`
/// (`data-pipelines/collectors/src/worker.rs`) passes a runtime value whose
/// *name* says curated but whose value is the `data/` root — a bare substring
/// rule would reject it wrongly. Matching the tail rather than the whole
/// literal catches the compound one-hop form `join("data/phase0/curated")`
/// as well as `join("curated")`.
fn names_a_curated_tail(expression: &str) -> Option<&str> {
    let bytes = expression.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'"' {
            index += 1;
            continue;
        }
        let end = end_of_string_literal(bytes, index)?;
        let literal = &expression[index + 1..end - 1];
        if literal == "curated" || literal.ends_with("/curated") || literal.ends_with("\\curated") {
            return Some(literal);
        }
        index = end;
    }
    None
}

/// The `let` binding the construction at `occurrence` is assigned to.
///
/// The statement may begin several lines above the construction — in
/// `crates/job-queue/tests/paper_preview.rs` the `let` is on the previous
/// line — so this scans back to the end of the previous statement or block
/// rather than to the start of the line. A construction that is not bound by
/// a plain `let` yields `None` and is therefore never allowed.
fn binding_name(source: &str, occurrence: usize) -> Option<&str> {
    let start = source[..occurrence]
        .rfind([';', '{', '}'])
        .map_or(0, |index| index + 1);
    let statement = source[start..occurrence].trim_start();
    let declared = statement.strip_prefix("let ")?.trim_start();
    let declared = declared
        .strip_prefix("mut ")
        .unwrap_or(declared)
        .trim_start();
    let name = declared
        .split(|character: char| !character.is_alphanumeric() && character != '_')
        .next()
        .filter(|name| !name.is_empty())?;
    // The binding must be assigned from this very expression, not merely
    // declared earlier in the statement.
    declared[name.len()..]
        .trim_start()
        .starts_with('=')
        .then_some(name)
}

/// Whether the construction at `occurrence` is immediately followed by an
/// assertion that the path it built does NOT exist.
///
/// This is the whole allow mechanism. It is not a file list: the code must
/// state, about the exact binding it just created, that the doubled path is
/// absent. Code that means to *read* through a doubled path cannot make that
/// claim, because the claim is the negation of what such code needs.
fn asserts_absence(source: &str, occurrence: usize, binding: &str) -> bool {
    let line_start = source[..occurrence]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let window = source[line_start..]
        .lines()
        .take(NEGATIVE_ASSERTION_WINDOW_LINES)
        .collect::<Vec<_>>()
        .join("\n");
    let compact: String = window.chars().filter(|c| !c.is_whitespace()).collect();
    compact.contains(&format!("assert!(!{binding}.exists()"))
}

fn line_of(source: &str, offset: usize) -> usize {
    source[..offset].matches('\n').count() + 1
}

/// Half A — the direct one-hop form, found in the source text.
#[test]
fn no_curate_store_is_built_from_a_curated_suffixed_path() {
    let root = repo_root();
    let mut sources = Vec::new();
    rust_sources(&root, &mut sources);
    sources.sort();
    assert!(
        sources.len() >= MINIMUM_RUST_SOURCES,
        "the walk under {} found only {} Rust sources, fewer than the {} floor: \
         the walk is broken, and a broken walk passes this guard vacuously",
        root.display(),
        sources.len(),
        MINIMUM_RUST_SOURCES
    );

    let mut violations = Vec::new();
    let mut allowed = BTreeSet::new();
    for path in &sources {
        let source = std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let relative = path
            .strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        for (occurrence, _) in source.match_indices(CONSTRUCTION) {
            let open = occurrence + CONSTRUCTION.len();
            let argument = balanced_argument(&source, open).unwrap_or_else(|| {
                panic!(
                    "{}:{}: unbalanced `{CONSTRUCTION}` argument",
                    relative,
                    line_of(&source, occurrence)
                )
            });
            let Some(literal) = names_a_curated_tail(argument) else {
                continue;
            };
            match binding_name(&source, occurrence)
                .filter(|binding| asserts_absence(&source, occurrence, binding))
            {
                Some(binding) => {
                    allowed.insert((relative.clone(), binding.to_owned()));
                }
                None => violations.push(format!(
                    "  {}:{}\n    argument: {}\n    curated-tailed literal: {:?}",
                    relative,
                    line_of(&source, occurrence),
                    argument.split_whitespace().collect::<Vec<_>>().join(" "),
                    literal
                )),
            }
        }
    }

    assert!(
        violations.is_empty(),
        "`{CONSTRUCTION}` appends `curated` itself \
         (crates/market-data/src/curate.rs, CurateStore::curated_dir), so an \
         argument that already ends in `curated` builds \
         `<root>/curated/curated/...`, which never exists. Pass the `data/` \
         root instead. {} site(s):\n{}\n\nIf a site builds the doubled path \
         deliberately in order to assert it is absent, bind it with `let` and \
         assert `assert!(!<binding>.exists())` within {} lines — that is the \
         only allowance, and no file is exempt.",
        violations.len(),
        violations.join("\n"),
        NEGATIVE_ASSERTION_WINDOW_LINES
    );

    for (path, binding) in EXPECTED_NEGATIVE_ASSERTIONS {
        assert!(
            allowed.contains(&(path.to_owned(), binding.to_owned())),
            "the scan no longer sees the deliberate `curated/curated` negative \
             assertion on `{binding}` in {path}. Either this scanner has \
             regressed and now matches nothing — in which case it would pass a \
             real reintroduction — or that guard test was removed. If it was \
             removed on purpose, drop the entry from \
             EXPECTED_NEGATIVE_ASSERTIONS; do not loosen the scan. Found: {:?}",
            allowed
        );
    }
}

#[derive(Default)]
struct CuratedTreeScan {
    bar_files: usize,
    nested: Vec<PathBuf>,
}

fn scan_curated_tree(directory: &Path, scan: &mut CuratedTreeScan) {
    let entries = std::fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()));
    let parent_is_curated = directory.file_name() == Some(OsStr::new("curated"));
    for entry in entries {
        let entry =
            entry.unwrap_or_else(|error| panic!("entry in {}: {error}", directory.display()));
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        if kind.is_dir() {
            if parent_is_curated && entry.file_name() == OsStr::new("curated") {
                scan.nested.push(path.clone());
            }
            scan_curated_tree(&path, scan);
        } else if entry.file_name() == OsStr::new("bars.parquet") {
            scan.bar_files += 1;
        }
    }
}

/// Half B — the filesystem invariant, which sees a doubled path however many
/// hops built it and whoever created it.
///
/// `crates/job-queue/src/factor_series.rs` built `curated/curated` in two
/// hops, so no source-text rule found it; three api-server fixtures *wrote*
/// through a doubled join, so the reader agreed with the writer and the suite
/// stayed green. A real generated tree answers both: the authoritative writer
/// is `scripts/ci/prepare_phase0.py`, and it never nests `curated`.
#[test]
fn the_generated_curated_tree_has_no_nested_curated_directory() {
    let repo = repo_root();
    let tracked = repo.join("data/phase0");
    // `data/phase0` is an untracked build artifact. CI materializes it before
    // the Rust lanes run (`.github/workflows/ci.yml`), so this branch is the
    // one CI takes and the skip below never fires there.
    let (root, _generated) = if tracked.join("curated/bars/market=kr").is_dir() {
        (tracked, None)
    } else {
        // The generator rejects any destination outside the repository, and
        // any destination that is not absent or empty
        // (`scripts/ci/prepare_phase0.py`, `destination`/`prepare`), so build
        // in a fresh repository child and let the TempDir remove it.
        let temporary = match tempfile::Builder::new()
            .prefix(".curated-path-contract-phase0-")
            .tempdir_in(&repo)
        {
            Ok(temporary) => temporary,
            Err(error) => {
                eprintln!(
                    "SKIPPED the_generated_curated_tree_has_no_nested_curated_directory: \
                     no curated tree at {} and no temporary directory could be created \
                     inside {} to generate one: {error}",
                    tracked.display(),
                    repo.display()
                );
                return;
            }
        };
        let destination = temporary.path().join("phase0");
        let interpreter = std::env::var_os("PYTHON").unwrap_or_else(|| "python".into());
        let script = repo.join("scripts/ci/prepare_phase0.py");
        let output = Command::new(&interpreter)
            .current_dir(&repo)
            .arg(&script)
            .arg("--root")
            .arg(&destination)
            .output();
        let output = match output {
            Ok(output) => output,
            Err(error) => {
                eprintln!(
                    "SKIPPED the_generated_curated_tree_has_no_nested_curated_directory: \
                     no curated tree at {} and the generator could not be launched — \
                     spawning {:?} {} failed: {error}. Set PYTHON to an interpreter \
                     that has pyarrow, or run `python {} --root data/phase0`.",
                    tracked.display(),
                    interpreter,
                    script.display(),
                    script.display()
                );
                return;
            }
        };
        if !output.status.success() {
            eprintln!(
                "SKIPPED the_generated_curated_tree_has_no_nested_curated_directory: \
                 no curated tree at {} and the generator failed (status {:?}). \
                 stderr: {}",
                tracked.display(),
                output.status.code(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
            return;
        }
        (destination, Some(temporary))
    };

    let mut scan = CuratedTreeScan::default();
    scan_curated_tree(&root, &mut scan);

    assert!(
        scan.bar_files > 0,
        "the curated tree at {} holds no bars.parquet, so this invariant would \
         hold over an empty directory and prove nothing",
        root.display()
    );
    assert!(
        scan.nested.is_empty(),
        "the curated tree at {} nests `curated` inside `curated`. \
         `CurateStore::new(root)` appends `curated` itself, so a doubled \
         directory means some writer was handed an already-`curated` path — and \
         every reader of that path finds nothing while reporting success. {} \
         path(s):\n{}",
        root.display(),
        scan.nested.len(),
        scan.nested
            .iter()
            .map(|path| format!("  {}", path.display()))
            .collect::<Vec<_>>()
            .join("\n")
    );
}
