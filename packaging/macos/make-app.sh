#!/usr/bin/env bash
# Wraps the host binary into Everywhere.app: a normal Mac app (no terminal),
# with the usage text macOS shows when asking for Screen Recording.
#
#   packaging/macos/make-app.sh [binary] [output dir]
#
# The app is ad-hoc signed. Without an Apple Developer ID, macOS asks the
# user to confirm the first launch (see docs/install-mac.md).
set -euo pipefail
cd "$(dirname "$0")/../.."
BIN="${1:-host/target/release/everywhere-host}"
OUT_DIR="${2:-dist}"
VERSION="$(grep -m1 '^version' host/Cargo.toml | cut -d'"' -f2)"
APP="$OUT_DIR/Everywhere.app"

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/Everywhere"
chmod +x "$APP/Contents/MacOS/Everywhere"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>Everywhere</string>
  <key>CFBundleDisplayName</key><string>Everywhere</string>
  <key>CFBundleIdentifier</key><string>io.github.mohamedzh4.everywhere</string>
  <key>CFBundleExecutable</key><string>Everywhere</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VERSION}</string>
  <key>CFBundleVersion</key><string>${VERSION}</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSScreenCaptureUsageDescription</key>
  <string>Everywhere shows this Mac's screen in your own browser when you connect from another computer.</string>
</dict>
</plist>
PLIST
codesign --force --sign - "$APP"
echo "$APP"
