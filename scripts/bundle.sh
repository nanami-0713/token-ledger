#!/bin/sh
# Assemble dist/TokenLedger.app: a real macOS app, double-clickable, its own
# Dock presence, not a terminal orphan. Run: sh scripts/bundle.sh
set -eu
APP=TokenLedger
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
~/.cargo/bin/cargo build --release
BUNDLE="$ROOT/dist/$APP.app"
rm -rf "$BUNDLE"
mkdir -p "$BUNDLE/Contents/MacOS" "$BUNDLE/Contents/Resources"
cp target/release/token-ledger "$BUNDLE/Contents/MacOS/$APP"
if [ -f "$ROOT/assets/icon.icns" ]; then
  cp "$ROOT/assets/icon.icns" "$BUNDLE/Contents/Resources/$APP.icns"
fi
cat > "$BUNDLE/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key><string>TokenLedger</string>
    <key>CFBundleIdentifier</key><string>com.nanami.token-ledger</string>
    <key>CFBundleName</key><string>TokenLedger</string>
    <key>CFBundleDisplayName</key><string>TokenLedger</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>0.2.0</string>
    <key>CFBundleVersion</key><string>0.2.0</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>LSMinimumSystemVersion</key><string>13.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict>
</plist>
PLIST
touch "$BUNDLE"
echo "bundled: $BUNDLE"
echo "open it:  open $BUNDLE"
