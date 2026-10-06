#!/bin/bash
# Build Lookout.
#   ./build.sh            build Lookout and install it to $LOOKOUT_APP_DIR (default: ~/Applications)
#   ./build.sh --tests    build and run the tests (assert-based, -Onone)
#
# Environment (all optional):
#   LOOKOUT_APP_DIR    where Lookout.app goes (default: $HOME/Applications)
#   LOOKOUT_BUNDLE_ID  bundle id (default: the installed app's, else io.github.kevintheo-ai.lookout)
#   LOOKOUT_SIGN_ID    code signing identity from your keychain (default: the installed app's, else ad-hoc)
#
# Accessibility access is tied to the signature. A named identity keeps it across rebuilds; an ad-hoc signature
# changes with every build, and macOS then needs the access granted again. The installed app is replaced only after
# the new one is built and signed.
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
APP_DIR="${LOOKOUT_APP_DIR:-$HOME/Applications}"
APP="$APP_DIR/Lookout.app"
# Without a fixed target, swiftc on macOS 27 picks a newer one than the SDK allows, and LaunchServices then refuses to
# start the app (error -10825). The tests are not affected; they do not start through LaunchServices.
TARGET="$(uname -m)-apple-macos26.0"
FRAMEWORKS=(-framework Cocoa -framework ApplicationServices -framework CoreGraphics)

if ! command -v swiftc >/dev/null; then
  echo "swiftc not found. Install the Xcode Command Line Tools: xcode-select --install" >&2
  exit 1
fi

has_identity() {
  # Read the whole list first: piped into `grep -q`, an early match can end in SIGPIPE, which pipefail reports as failure.
  [[ "$(security find-identity -v -p codesigning)" == *"\"$1\""* ]]
}

build_app() {
  local installed_id="" installed_sign="" bid sign
  if [ -d "$APP" ]; then
    installed_id="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$APP/Contents/Info.plist" 2>/dev/null || true)"
    installed_sign="$(codesign -dv --verbose=2 "$APP" 2>&1 | sed -n 's/^Authority=//p' | head -1 || true)"
  fi
  bid="${LOOKOUT_BUNDLE_ID:-${installed_id:-io.github.kevintheo-ai.lookout}}"

  # Decide the signature before building, so a missing identity never touches the installed app.
  if [ -n "${LOOKOUT_SIGN_ID:-}" ]; then
    sign="$LOOKOUT_SIGN_ID"
    has_identity "$sign" || { echo "Signing identity \"$sign\" (LOOKOUT_SIGN_ID) is not in your keychain." >&2; exit 1; }
  elif [ -n "$installed_sign" ]; then
    sign="$installed_sign"
    if ! has_identity "$sign"; then
      echo "The installed Lookout is signed with \"$sign\", which is not in your keychain." >&2
      echo "Nothing was changed. Set LOOKOUT_SIGN_ID, or remove $APP to start over with ad-hoc signing." >&2
      exit 1
    fi
  else
    sign="-"
  fi

  STAGE="$(mktemp -d)"
  trap 'rm -rf "$STAGE"' EXIT
  local new="$STAGE/Lookout.app"
  mkdir -p "$new/Contents/MacOS" "$new/Contents/Resources"
  cat > "$new/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDisplayName</key><string>Lookout</string>
	<key>CFBundleExecutable</key><string>Lookout</string>
	<key>CFBundleIconFile</key><string>AppIcon</string>
	<key>CFBundleIdentifier</key><string>$bid</string>
	<key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
	<key>CFBundleName</key><string>Lookout</string>
	<key>CFBundlePackageType</key><string>APPL</string>
	<key>CFBundleShortVersionString</key><string>3.0</string>
	<key>CFBundleVersion</key><string>1</string>
	<key>LSMinimumSystemVersion</key><string>26.0</string>
	<key>LSUIElement</key><true/>
	<key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
  printf 'APPL????' > "$new/Contents/PkgInfo"
  cp "$DIR/AppIcon.icns" "$new/Contents/Resources/AppIcon.icns"
  swiftc -target "$TARGET" -O -o "$new/Contents/MacOS/Lookout" "$DIR"/app/*.swift "${FRAMEWORKS[@]}"
  # Clear extended attributes that codesign rejects (macOS 27's xattr has no -r); codesign below is the real check.
  find "$new" -exec xattr -c {} + 2>/dev/null || true
  codesign --force --identifier "$bid" -s "$sign" "$new"
  codesign --verify --strict "$new"

  # Swap only now: keep the old app until the new one is in place.
  mkdir -p "$APP_DIR"
  if [ -d "$APP" ]; then mv "$APP" "$STAGE/previous.app"; fi
  if ! mv "$new" "$APP"; then
    if [ -d "$STAGE/previous.app" ]; then
      if ! mv "$STAGE/previous.app" "$APP"; then
        trap - EXIT   # keep the only copy of the previous app
        echo "Could not install $APP, nor put the previous app back; it is kept at $STAGE/previous.app." >&2
        exit 1
      fi
      echo "Could not install $APP; the previous app was restored." >&2
    else
      echo "Could not install $APP." >&2
    fi
    exit 1
  fi

  if [ "$sign" != "-" ] && [ "$sign" = "$installed_sign" ] && [ "$bid" = "$installed_id" ]; then
    echo "Installed $APP (signed with \"$sign\", $bid). Accessibility access stays granted; restart Lookout."
  else
    # The earlier grant does not match this build (ad hoc, or a new identity or bundle id): clear the stale entry, so
    # the new one can be granted cleanly.
    tccutil reset Accessibility "$bid" >/dev/null 2>&1 || true
    if [ "$sign" = "-" ]; then echo "Installed $APP (ad-hoc signed, $bid)."
    else echo "Installed $APP (signed with \"$sign\", $bid)."; fi
    echo "Grant Accessibility access: System Settings > Privacy & Security > Accessibility > Lookout."
  fi
}

build_tests() {
  local bin="$DIR/.build/LayoutTests"
  mkdir -p "$DIR/.build"
  # -Onone: under -O the compiler drops the asserts.
  swiftc -target "$TARGET" -Onone -o "$bin" \
    "$DIR/app/Common.swift" "$DIR/app/Watcher.swift" "$DIR/app/Mapper.swift" "$DIR/app/Layout.swift" \
    "$DIR/app/Overlay.swift" "$DIR/tests/LayoutTests.swift" "${FRAMEWORKS[@]}"
  # Own log file: otherwise the tests would write their sample lines into ~/Library/Logs/Lookout.log.
  LOOKOUT_LOG="$DIR/.build/tests.log" "$bin" "$@"
}

case "${1:-}" in
  "" | --lookout) build_app ;;
  --tests) shift; build_tests "$@" ;;
  *) echo "Unknown option: $1 (allowed: --tests)" >&2; exit 2 ;;
esac
