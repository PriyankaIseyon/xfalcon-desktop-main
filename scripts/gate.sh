#!/usr/bin/env bash
#
# The quality gate that used to run in GitHub Actions.
#
# `ci` and `chat-slice-e2e` are disabled in Actions (Windows bills at 2x and
# macOS at ~10x against the org's included minutes, and both fired on every PR
# *and* every merge to main -- see the workflow files, which are kept in the
# repo so re-enabling is a `gh workflow enable` away). `desktop-build` stays
# active because releases still need real macOS and Windows runners.
#
# So this script is now the gate. It mirrors, in order, the two steps that
# `ci.yml` ran on each of macos-latest and windows-latest:
#
#   Frontend quality gates:  format:check, lint, npm test
#   Rust quality and policy: fmt --check, clippy -D warnings, test, deny
#
# plus `chat-slice-e2e.yml`'s `unix-rust` job, which is the same
# `cargo test --workspace --locked` this already runs.
#
# What this CANNOT replace: `ci.yml` ran the Rust gate on macOS as well as
# Windows, and clippy's `cfg(not(windows))` arms only compile on a non-Windows
# host. A Windows-only local run cannot see a `needless_return` -- or a broken
# `#[cfg(not(windows))]` code path -- in a macOS-only branch. That gap is real
# and it has already shipped a macOS-only startup bug once. Re-enable `ci` (or
# run this script on a Mac) before releasing anything that touches
# platform-conditional code.
#
# Usage:
#   scripts/gate.sh              # frontend + rust, the ci.yml equivalent
#   scripts/gate.sh --rust       # rust only
#   scripts/gate.sh --frontend   # frontend only
#   scripts/gate.sh --e2e        # additionally run the Playwright suites
#
set -uo pipefail

cd "$(dirname "$0")/.."

run_frontend=1
run_rust=1
run_e2e=0
for arg in "$@"; do
  case "$arg" in
    --rust) run_frontend=0 ;;
    --frontend) run_rust=0 ;;
    --e2e) run_e2e=1 ;;
    -h|--help) sed -n '2,40p' "$0" | sed 's/^# \?//'; exit 0 ;;
    *) echo "gate: unknown argument '$arg'" >&2; exit 2 ;;
  esac
done

# On WSL the Rust toolchain must be the Windows one: the MSVC linker cannot
# write through a `\\wsl$` UNC path, so a Linux-side `cargo build` fails at
# link time on this checkout.
#
# Three traps sit behind that sentence, and together they used to make this
# gate report a Rust pass it never ran:
#
#   1. `~/.cargo/bin/cargo.exe` is a symlink to rustup's shim. WSL interop
#      cannot exec a Windows symlink -- it fails "Permission denied" -- and
#      `command -v cargo.exe` does not find it on PATH either. The toolchain's
#      own binary is a real file and runs.
#   2. A UNC working directory breaks incremental compilation: "could not
#      create session directory lock file: Incorrect function". `pushd` maps a
#      drive letter, which needs cmd.exe rather than a direct exec.
#   3. The target directory has to live on a local drive for the same linker
#      reason. `CARGO_TARGET_DIR` from the environment wins if it is set.
cargo_bin=cargo
on_wsl=0
windows_cargo_exe=""
if grep -qi microsoft /proc/version 2>/dev/null; then
  on_wsl=1
  for candidate in "$HOME"/.rustup/toolchains/*-pc-windows-msvc/bin/cargo.exe \
                   /mnt/c/Users/*/.rustup/toolchains/*-pc-windows-msvc/bin/cargo.exe; do
    if [ -f "$candidate" ] && [ ! -L "$candidate" ]; then
      windows_cargo_exe=$candidate
      break
    fi
  done
  if [ -z "$windows_cargo_exe" ]; then
    echo "gate: no Windows Rust toolchain found under .rustup/toolchains." >&2
    echo "gate: install one with 'rustup toolchain install stable-x86_64-pc-windows-msvc'." >&2
    echo "gate: the Rust gate cannot run on the Linux toolchain -- the MSVC linker" >&2
    echo "gate: cannot write through a UNC path. Refusing to report a pass it did not run." >&2
    exit 2
  fi
fi

# Runs the Windows toolchain from a mapped drive, so incremental compilation
# has a working directory it can lock.
#
# The script is written with a quoted-value heredoc rather than `printf`: a
# `.bat` is dense with `%VAR%`, and every one of them is a `printf` conversion
# waiting to eat an exit code. This gate already reported a pass over a failing
# clippy once for exactly that reason.
windows_cargo() {
  local script status target_line
  if [ -n "${CARGO_TARGET_DIR:-}" ]; then
    target_line="set CARGO_TARGET_DIR=${CARGO_TARGET_DIR}"
  else
    target_line='set CARGO_TARGET_DIR=%LOCALAPPDATA%\xfalcon-gate-target'
  fi
  script=$(mktemp /tmp/gate-cargo-XXXXXX.bat)
  {
    echo '@echo off'
    echo "$target_line"
    # Build scripts shell out to `rustc`, which is beside cargo and not on the
    # inherited PATH.
    echo "set PATH=$(wslpath -w "$(dirname "$windows_cargo_exe")");%PATH%"
    echo "pushd $(wslpath -w "$PWD/src-tauri")"
    echo "\"$(wslpath -w "$windows_cargo_exe")\" $*"
    echo 'set CODE=%ERRORLEVEL%'
    echo 'popd'
    echo 'exit /b %CODE%'
  } | sed 's/$/\r/' >"$script"
  # From /mnt/c, so cmd.exe does not start on a UNC path and warn about it.
  (cd /mnt/c && cmd.exe /c "$(wslpath -w "$script")")
  status=$?
  rm -f "$script"
  return $status
}

# One name for both hosts. Both run from `src-tauri`, so the call sites below
# carry no `--manifest-path`.
cargo_run() {
  if ((on_wsl)); then
    windows_cargo "$@"
  else
    (cd src-tauri && "$cargo_bin" "$@")
  fi
}

failed=()
step() {
  local label=$1
  shift
  printf '\n\033[1m=== %s ===\033[0m\n' "$label"
  if "$@"; then
    printf '\033[32mok\033[0m  %s\n' "$label"
  else
    printf '\033[31mFAIL\033[0m %s\n' "$label"
    failed+=("$label")
  fi
}

if ((run_frontend)); then
  step "prettier --check"      npm run --silent format:check
  # `tsc -b`, which walks the project references and so covers the test and
  # benchmark sources. `npm run build` runs the same script, and without this
  # step a type error in a test file passes the gate and breaks the build.
  step "tsc -b (typecheck)"    npm run --silent typecheck
  step "eslint --max-warnings=0" npm run --silent lint
  step "style contract" npm run --silent check:styles
  step "tooltip contract" npm run --silent check:tooltips
  # The closure ledger has to be able to lose a row loudly -- seventeen
  # primitive rows once vanished in an unrelated edit and nothing noticed.
  step "ux matrix ledger" npm run --silent check:matrix
  # Two workers, not the default (one per core): each jsdom worker holds about a
  # gigabyte, and with rust-analyzer open the default run was killed for memory
  # (exit 137) on a 16 GB WSL VM. The extra args reach `vitest run`, the last
  # command of the `test` script.
  step "vitest + font inventory" npm run --silent test -- --maxWorkers=2
fi

if ((run_rust)); then
  step "cargo fmt --check"  cargo_run fmt --all -- --check
  step "cargo clippy -D warnings" \
    cargo_run clippy --workspace --all-targets -- -D warnings
  step "cargo test --locked" \
    cargo_run test --workspace --locked
  # `deny` reads `Cargo.lock` and an advisory database. It never links, so it
  # does not need the MSVC toolchain -- and on WSL it must not use it, because
  # cargo-deny fetches the database with the git CLI and the Windows side of
  # this machine has no git. The host toolchain has one.
  #
  # cargo-deny is a separate install; a missing binary is a skipped policy
  # check, not a passing one, so say so loudly rather than exiting 0.
  if (cd src-tauri && cargo deny --version >/dev/null 2>&1); then
    step "cargo deny check" bash -c 'cd src-tauri && cargo deny check'
  else
    printf '\n\033[33mSKIP\033[0m cargo deny check -- not installed '
    printf '(cargo install cargo-deny --version 0.19.4 --locked)\n'
    failed+=("cargo deny check (not installed)")
  fi
fi

if ((run_e2e)); then
  step "playwright chat e2e" npm run --silent test:e2e:chat
  step "playwright visual"   npm run --silent test:visual
fi

printf '\n'
if ((${#failed[@]})); then
  printf '\033[31m%d gate(s) failed:\033[0m\n' "${#failed[@]}"
  printf '  - %s\n' "${failed[@]}"
  exit 1
fi
printf '\033[32mall gates passed\033[0m\n'
