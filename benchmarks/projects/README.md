# Project index benchmark/smoke tree

This directory holds the tooling for exercising
`crate::projects::index`/`crate::projects::search`/`crate::projects::read`
(`src-tauri/src/projects/{index,search,read}.rs`) against a large, synthetic
project tree, per
`docs/decisions.md` (Task 5): "100,000-file
synthetic tree startup/search behavior."

## What this measures

Real trusted projects can be large (monorepos, vendored dependencies,
generated build output). Task 5's incremental indexer
(`src-tauri/src/projects/index.rs`) is designed around that scale: a bounded
worker-thread walk (the `ignore` crate), fingerprint-based skip-if-unchanged
reindexing, and batched encrypted DB writes. Unit tests in
`src-tauri/src/projects/tests/index.rs` prove each of those behaviors
individually on small, fast, single-purpose fixtures (a handful of files
each). This benchmark/smoke tree instead answers the question those unit
tests deliberately do not: **does the whole pipeline still behave correctly,
and complete in a reasonable time, at the scale it was designed for?**

Concretely, running the paired Rust test (below) against a generated tree
checks:

- a full index pass over ~100,000 files completes and reports a sane
  outcome (no cancellation, no root-identity error, at least one file
  indexed);
- `.gitignore` exclusion (`*.ignored`) still holds at this scale, not only
  in the single-file unit tests;
- filename search (`project.search_files` /
  `crate::projects::search::search_files`) finds real matches against the
  populated index;
- a **second**, incremental pass over the same, unchanged tree indexes
  zero new files (the fingerprint-skip design actually avoids redundant
  re-hashing at scale, not just in a two-file unit test).

This is a correctness-and-completion smoke test, not a calibrated
performance benchmark: per
`docs/contributing.md`'s performance rules,
"reference benchmark numbers must be labeled as external or self-reported;
they cannot satisfy xFalcon acceptance criteria," and the same standard
applies to any timing this script or test prints about itself. Treat the
printed durations as a relative signal across runs on the same machine (e.g.
"did the incremental pass get dramatically cheaper"), not as an absolute
product SLA.

## Generating a tree

`generate-tree.ps1` is PowerShell, not a shell script, because this
repository's Rust toolchain is Windows `cargo.exe` (see
`docs/contributing.md`), and every platform this
benchmark actually targets (a developer machine or CI runner building
`xfalcon-desktop`) has PowerShell available.

```powershell
# Full 100,000-file tree matching the plan's own benchmark scale:
./generate-tree.ps1 -Root C:\bench\xfalcon-project-index -FileCount 100000

# A much smaller tree for a quick local smoke check, plus placeholder
# sensitive-class sample files (never real secrets):
./generate-tree.ps1 -Root C:\bench\smoke -FileCount 2000 -IncludeSensitiveSamples
```

The generated tree mixes plain text files of varying size, a small fraction
of binary files, a `.gitignore` that excludes a real portion of the tree, and
nested directories (`-MaxDepth`/`-DirsPerLevel`) so the walk exercises real
directory fan-out rather than one flat directory. See the script's own
comment-based help (`Get-Help ./generate-tree.ps1 -Full`) for every
parameter.

**This script only writes files under `-Root`.** It does not read, trust, or
register the generated directory with xfalcon-desktop; it is not a
substitute for the app's own "trust a project" flow, and it must never be
pointed at the repository itself or at any directory you do not want filled
with tens of thousands of throwaway files.

## Running the paired test

`src-tauri/src/projects/tests/index.rs` has one `#[ignore]`d test,
`startup_and_search_over_a_100_000_file_synthetic_tree_completes_and_finds_matches`,
that reads the tree's location from an environment variable and runs the
checks listed above against it:

```powershell
$env:XFALCON_PROJECT_INDEX_BENCH_ROOT = "C:\bench\xfalcon-project-index"
cd src-tauri
cargo test --release -- --ignored startup_and_search_over_a_100_000_file_synthetic_tree
```

It is `#[ignore]`d (excluded from the ordinary `cargo test` run) because
generating and walking 100,000 files does not belong in this crate's fast
unit-test loop -- the plan itself frames this scenario as "not necessarily a
fast unit test." `--release` is recommended (not required) for a
representative timing signal; a debug build will complete correctly, just
more slowly.

## Cleaning up

The generated tree is disposable. Delete `-Root` (or pass `-Clean` on a
later `generate-tree.ps1` run against the same path) once you are done with
it; nothing in this repository depends on it persisting between runs.
