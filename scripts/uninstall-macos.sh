#!/usr/bin/env bash
#
# Uninstalls xFalcon Desktop on macOS, leaving no local data behind.
#
# A DMG install has no uninstaller hook, so this script does what the Windows
# NSIS uninstaller does: it quits the app, runs the app's own
# --purge-local-data mode (data folders, WebView storage, caches, temp files and
# keychain entries), then removes the app bundle.
#
#   scripts/uninstall-macos.sh                       # /Applications/xFalcon.app
#   scripts/uninstall-macos.sh "/path/to/xFalcon.app"
#
# The script refuses anything that is not an xFalcon .app bundle. It exits with
# the purge's exit code when that code is non-zero (2: something remains,
# 1: error). The app bundle is removed either way.

set -euo pipefail

APP="${1:-/Applications/xFalcon.app}"
APP="${APP%/}"
BINARY_NAME="xfalcon-desktop"
BUNDLE_ID="ai.xfalcon.desktop"

case "$APP" in
  *.app) ;;
  *)
    echo "refusing: not a .app bundle: $APP" >&2
    exit 1
    ;;
esac

if [ ! -d "$APP" ] || [ ! -f "$APP/Contents/MacOS/$BINARY_NAME" ]; then
  echo "refusing: $APP is not an xFalcon app (missing Contents/MacOS/$BINARY_NAME)" >&2
  exit 1
fi

bundle_id="$(defaults read "$APP/Contents/Info" CFBundleIdentifier 2>/dev/null || true)"
if [ "$bundle_id" != "$BUNDLE_ID" ]; then
  echo "refusing: $APP has bundle identifier '$bundle_id', expected '$BUNDLE_ID'" >&2
  exit 1
fi

if pgrep -x "$BINARY_NAME" >/dev/null 2>&1; then
  echo "quitting xFalcon..."
  osascript -e 'tell application "xFalcon" to quit' >/dev/null 2>&1 || true
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    pgrep -x "$BINARY_NAME" >/dev/null 2>&1 || break
    sleep 1
  done
  if pgrep -x "$BINARY_NAME" >/dev/null 2>&1; then
    echo "xFalcon did not quit; stopping it"
    pkill -x "$BINARY_NAME" || true
    for _ in 1 2 3 4 5; do
      pgrep -x "$BINARY_NAME" >/dev/null 2>&1 || break
      sleep 1
    done
  fi
  if pgrep -x "$BINARY_NAME" >/dev/null 2>&1; then
    echo "warning: xFalcon is still running; some local data may be left behind" >&2
  fi
fi

echo "removing local data..."
purge_code=0
# --retry-until-clean repeats the purge while files remain, for a process that
# is still exiting and holds files for a moment after the app quits.
"$APP/Contents/MacOS/$BINARY_NAME" --purge-local-data --retry-until-clean || purge_code=$?

if [ "$purge_code" -eq 0 ]; then
  echo "local data removed"
elif [ "$purge_code" -eq 2 ]; then
  echo "warning: some local data could not be removed (exit code 2)" >&2
else
  echo "warning: local data purge failed (exit code $purge_code)" >&2
fi

echo "removing $APP..."
rm -rf -- "$APP"
echo "xFalcon uninstalled"

exit "$purge_code"
