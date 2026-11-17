#!/usr/bin/env bash
#
# Verifies a built macOS artifact will install without a Gatekeeper block.
#
# A release can reach users broken in ways the build log looks happy about: an
# ad-hoc signature when the certificate failed to import, a missing secure
# timestamp, or a notarization ticket that was never stapled. A missing staple
# is the nastiest, because a machine that can reach Apple's servers verifies
# online and looks fine -- the block only appears for the offline or
# rate-limited user.
#
# A disk image needs both layers checked. `tauri-action` notarizes and staples
# the .app, then wraps it in a DMG that is signed but not itself notarized;
# Gatekeeper assesses the downloaded image before anything is mounted, so that
# DMG is refused even though the app inside it is perfect. So for a .dmg this
# checks the image, then mounts it and checks the app.
#
# Note that hardened runtime is asserted only on the app. The flag lives in an
# executable's code directory; a disk image is a container and never carries
# it, so demanding it of a DMG fails a correct artifact.
#
#   scripts/verify-macos-release.sh path/to/xFalcon_0.5.2_universal.dmg
#   scripts/verify-macos-release.sh "path/to/xFalcon.app"
#   scripts/verify-macos-release.sh "path/to/xFalcon.app.tar.gz"
#
# Requires macOS. Exits non-zero if any check fails.

set -uo pipefail

if [ "$(uname -s)" != "Darwin" ]; then
  echo "error: macOS only (Gatekeeper and stapler are not available here)" >&2
  exit 2
fi

TARGET="${1:-}"
if [ -z "$TARGET" ] || [ ! -e "$TARGET" ]; then
  echo "usage: $0 <path to .app, .dmg or .app.tar.gz>" >&2
  exit 2
fi

failed=0
MOUNTPOINT=""
EXTRACT_DIR=""

note() { printf '\n== %s\n' "$1"; }
fail() { printf '   FAIL: %s\n' "$1"; failed=1; }
pass() { printf '   ok: %s\n' "$1"; }

cleanup() {
  if [ -n "$MOUNTPOINT" ]; then hdiutil detach "$MOUNTPOINT" -quiet 2>/dev/null; fi
  if [ -n "$EXTRACT_DIR" ]; then rm -rf "$EXTRACT_DIR"; fi
  return 0
}
trap cleanup EXIT

# $1 path, $2 human label, $3 "runtime" to require the hardened runtime flag
check_signature() {
  local path="$1" label="$2" want_runtime="${3:-}"
  local details
  details=$(codesign --display --verbose=4 "$path" 2>&1 || true)

  if codesign --verify --deep --strict "$path" 2>/dev/null; then
    pass "$label signature is intact"
  else
    fail "$label signature does not verify (a 'detritus not allowed' xattr means you are checking an installed copy, not the release asset)"
  fi

  # Notarization requires a Developer ID certificate. An Apple Development
  # certificate signs cleanly, but Apple refuses to notarize it and Gatekeeper
  # blocks it everywhere except a registered device.
  if grep -q 'Authority=Developer ID Application' <<<"$details"; then
    pass "$label signed with a Developer ID Application certificate"
  else
    fail "$label not signed with a Developer ID Application certificate"
  fi

  # Without a secure timestamp a signature stops validating when the
  # certificate expires, rather than staying valid for what was signed while it
  # was live.
  if grep -q '^Timestamp=' <<<"$details"; then
    pass "$label has a secure timestamp"
  else
    fail "$label has no secure timestamp"
  fi

  if [ "$want_runtime" = "runtime" ]; then
    if grep -Eq 'flags=0x[0-9a-f]*\(.*runtime' <<<"$details"; then
      pass "$label has the hardened runtime enabled"
    else
      fail "$label is missing the hardened runtime (notarization will be refused)"
    fi
  fi
}

# $1 path, $2 label, $3 spctl assess args
check_gatekeeper() {
  local path="$1" label="$2"; shift 2
  if xcrun stapler validate "$path" >/dev/null 2>&1; then
    pass "$label has a stapled ticket (offline machines accept it)"
  else
    fail "$label has no stapled ticket -- offline users will be blocked"
  fi
  local verdict
  verdict=$(spctl --assess "$@" --verbose=4 "$path" 2>&1 || true)
  printf '   %s\n' "$(tr '\n' ' ' <<<"$verdict" | sed 's/  */ /g')"
  if grep -q ': accepted' <<<"$verdict"; then
    pass "Gatekeeper accepts $label"
  else
    fail "Gatekeeper rejects $label"
  fi
}

case "$TARGET" in
  *.app.tar.gz)
    note "Application inside the updater archive"
    EXTRACT_DIR=$(mktemp -d) || exit 1
    tar -xzf "$TARGET" -C "$EXTRACT_DIR" || exit 1
    shopt -s nullglob
    APPS=("$EXTRACT_DIR"/*.app)
    if [ "${#APPS[@]}" -ne 1 ]; then
      fail "updater archive must contain exactly one application bundle"
    else
      check_signature "${APPS[0]}" "updater app" runtime
      check_gatekeeper "${APPS[0]}" "the updater app" --type execute
    fi
    ;;
  *.app)
    note "Application bundle"
    check_signature "$TARGET" "app" runtime
    # `--type install` is for installer packages; assessing an application
    # under it reports a spurious rejection for a perfectly good bundle.
    check_gatekeeper "$TARGET" "the app" --type execute
    ;;
  *.dmg)
    note "Disk image"
    check_signature "$TARGET" "disk image"
    check_gatekeeper "$TARGET" "the disk image" --type open --context context:primary-signature

    note "Application inside the disk image"
    MOUNTPOINT=$(hdiutil attach -nobrowse -readonly -plist "$TARGET" 2>/dev/null \
      | grep -A1 '<key>mount-point</key>' | grep '<string>' \
      | sed -E 's#.*<string>(.*)</string>.*#\1#' | head -1)
    if [ -z "$MOUNTPOINT" ] || [ ! -d "$MOUNTPOINT" ]; then
      fail "could not mount the disk image to inspect the app inside"
    else
      APP=$(find "$MOUNTPOINT" -maxdepth 1 -name '*.app' -print -quit)
      if [ -z "$APP" ]; then
        fail "no .app found inside the disk image"
      else
        printf '   %s\n' "$(basename "$APP")"
        check_signature "$APP" "app" runtime
        check_gatekeeper "$APP" "the app" --type execute
      fi
    fi
    ;;
  *)
    echo "error: expected a .app, .dmg or .app.tar.gz, got: $TARGET" >&2
    exit 2
    ;;
esac

printf '\n'
if [ "$failed" -ne 0 ]; then
  echo "RESULT: not safe to publish -- see the failures above."
  exit 1
fi
echo "RESULT: $TARGET will install without a Gatekeeper prompt."
