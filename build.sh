#!/bin/bash
# Build Lookout.
#   ./build.sh            build Lookout and install it to $LOOKOUT_APP_DIR (default: ~/Applications)
#   ./build.sh --tests    format check, lints and tests of the whole workspace
#   ./build.sh --app      build and sign into target/Lookout.app for a live test beside the installed app, which stays
#                         untouched (same bundle id and signature, so its Accessibility access applies)
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
VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' "$DIR/Cargo.toml" | head -1)"

if ! command -v cargo >/dev/null; then
  echo "cargo not found. Install Rust (https://rustup.rs, or brew install rust) and the Xcode Command Line Tools" >&2
  echo "(xcode-select --install)." >&2
  exit 1
fi

has_identity() {
  # Read the whole list first: piped into `grep -q`, an early match can end in SIGPIPE, which pipefail reports as failure.
  [[ "$(security find-identity -v -p codesigning)" == *"\"$1\""* ]]
}

# Bundle id and signature, decided before building, so a missing identity never touches the installed app.
identity() {
  INSTALLED_ID=""
  INSTALLED_SIGN=""
  if [ -d "$APP" ]; then
    INSTALLED_ID="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$APP/Contents/Info.plist" 2>/dev/null || true)"
    INSTALLED_SIGN="$(codesign -dv --verbose=2 "$APP" 2>&1 | sed -n 's/^Authority=//p' | head -1 || true)"
  fi
  BID="${LOOKOUT_BUNDLE_ID:-${INSTALLED_ID:-io.github.kevintheo-ai.lookout}}"
  if [ -n "${LOOKOUT_SIGN_ID:-}" ]; then
    SIGN="$LOOKOUT_SIGN_ID"
    has_identity "$SIGN" || { echo "Signing identity \"$SIGN\" (LOOKOUT_SIGN_ID) is not in your keychain." >&2; exit 1; }
  elif [ -n "$INSTALLED_SIGN" ]; then
    SIGN="$INSTALLED_SIGN"
    if ! has_identity "$SIGN"; then
      echo "The installed Lookout is signed with \"$SIGN\", which is not in your keychain." >&2
      echo "Nothing was changed. Set LOOKOUT_SIGN_ID, or remove $APP to start over with ad-hoc signing." >&2
      exit 1
    fi
  else
    SIGN="-"
  fi
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
	<key>CFBundleShortVersionString</key><string>$VERSION</string>
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
  # Clear extended attributes that codesign rejects (macOS 27's xattr has no -r); codesign below is the real check.
  find "$app" -exec xattr -c {} + 2>/dev/null || true
  codesign --force --identifier "$BID" -s "$SIGN" "$app"
  codesign --verify --strict "$app"
}

install_app() {
  identity
  (cd "$DIR" && cargo build --release)
  STAGE="$(mktemp -d)"
  trap 'rm -rf "$STAGE"' EXIT
  local new="$STAGE/Lookout.app"
  bundle "$new"

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

  if [ "$SIGN" != "-" ] && [ "$SIGN" = "$INSTALLED_SIGN" ] && [ "$BID" = "$INSTALLED_ID" ]; then
    echo "Installed $APP (signed with \"$SIGN\", $BID). Accessibility access stays granted; restart Lookout."
  else
    # The earlier grant does not match this build (ad hoc, or a new identity or bundle id): clear the stale entry, so
    # the new one can be granted cleanly.
    tccutil reset Accessibility "$BID" >/dev/null 2>&1 || true
    if [ "$SIGN" = "-" ]; then echo "Installed $APP (ad-hoc signed, $BID)."
    else echo "Installed $APP (signed with \"$SIGN\", $BID)."; fi
    echo "Grant Accessibility access: System Settings > Privacy & Security > Accessibility > Lookout."
  fi
}

case "${1:-}" in
  "" | --lookout) install_app ;;
  --tests)
    cd "$DIR"
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace
    ;;
  --app)
    # A live test quits the installed app, starts this one with `open -n`, and starts the installed one again
    # afterwards.
    identity
    (cd "$DIR" && cargo build --release)
    rm -rf "$DIR/target/Lookout.app"
    bundle "$DIR/target/Lookout.app"
    echo "$DIR/target/Lookout.app"
    ;;
  *) echo "Unknown option: $1 (allowed: --tests, --app)" >&2; exit 2 ;;
esac
