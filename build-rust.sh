#!/bin/bash
# Build the Rust rewrite of Lookout (replaces build.sh once the rewrite is complete).
#   ./build-rust.sh --tests   format check, lints and tests of the whole workspace
#   ./build-rust.sh --probe   build, bundle and sign like the installed app, start it once, and print whether it holds
#                             Accessibility access; the installed app is never touched
#   ./build-rust.sh --app     build, bundle and sign into target/Lookout.app for a live test; the installed app is
#                             never touched
#
# Environment (optional): LOOKOUT_BUNDLE_ID and LOOKOUT_SIGN_ID, default: those of the installed
# ~/Applications/Lookout.app. Accessibility access belongs to bundle id + signing identity, so the probe tells whether
# the Rust build can take over the existing grant.
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALLED="$HOME/Applications/Lookout.app"
LSREGISTER=/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister

identity() {
  BID="${LOOKOUT_BUNDLE_ID:-$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$INSTALLED/Contents/Info.plist" 2>/dev/null || true)}"
  SIGN="${LOOKOUT_SIGN_ID:-$(codesign -dv --verbose=2 "$INSTALLED" 2>&1 | sed -n 's/^Authority=//p' | head -1 || true)}"
  if [ -z "$BID" ] || [ -z "$SIGN" ]; then
    echo "No installed Lookout to copy bundle id and signature from; set LOOKOUT_BUNDLE_ID and LOOKOUT_SIGN_ID." >&2
    exit 1
  fi
  [[ "$(security find-identity -v -p codesigning)" == *"\"$SIGN\""* ]] || {
    echo "Signing identity \"$SIGN\" is not in your keychain." >&2
    exit 1
  }
}

# bundle <path/to/Lookout.app>: wrap target/release/Lookout into a signed app bundle.
bundle() {
  local app="$1"
  mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
  cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleDisplayName</key><string>Lookout</string>
	<key>CFBundleExecutable</key><string>Lookout</string>
	<key>CFBundleIconFile</key><string>AppIcon</string>
	<key>CFBundleIdentifier</key><string>$BID</string>
	<key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
	<key>CFBundleName</key><string>Lookout</string>
	<key>CFBundlePackageType</key><string>APPL</string>
	<key>CFBundleShortVersionString</key><string>4.0.0</string>
	<key>CFBundleVersion</key><string>1</string>
	<key>LSMinimumSystemVersion</key><string>26.0</string>
	<key>LSUIElement</key><true/>
	<key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST
  printf 'APPL????' > "$app/Contents/PkgInfo"
  cp "$DIR/AppIcon.icns" "$app/Contents/Resources/AppIcon.icns"
  cp "$DIR/target/release/Lookout" "$app/Contents/MacOS/Lookout"
  # codesign rejects extended attributes (macOS 27's xattr has no -r).
  find "$app" -exec xattr -c {} + 2>/dev/null || true
  codesign --force --identifier "$BID" -s "$SIGN" "$app"
  codesign --verify --strict "$app"
}

case "${1:-}" in
  --tests)
    cd "$DIR"
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace
    ;;
  --probe)
    identity
    (cd "$DIR" && cargo build --release)
    STAGE="$(mktemp -d)"
    trap '"$LSREGISTER" -u "$STAGE/Lookout.app" 2>/dev/null || true; rm -rf "$STAGE"' EXIT
    bundle "$STAGE/Lookout.app"
    LOG="$HOME/Library/Logs/Lookout-rust-probe.log"
    : > "$LOG"
    # -n: a second instance beside the running Lookout; started by LaunchServices, so macOS checks the bundle's own
    # Accessibility access, not that of the terminal this script runs in.
    open -n -W "$STAGE/Lookout.app" --args --probe
    cat "$LOG"
    ;;
  --app)
    # Build, bundle and sign into target/Lookout.app with the installed app's bundle id and signature, so its
    # Accessibility grant applies. The installed app stays untouched; a live test quits it, starts this one with
    # `open -n`, and starts the installed one again afterwards.
    identity
    (cd "$DIR" && cargo build --release)
    APP="$DIR/target/Lookout.app"
    rm -rf "$APP"
    bundle "$APP"
    echo "$APP"
    ;;
  *)
    echo "usage: $0 --tests | --probe | --app" >&2
    exit 2
    ;;
esac
