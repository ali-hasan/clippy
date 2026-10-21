#!/bin/zsh
# Builds target/Clippy.app, a menu-bar-only app that can be copied to /Applications.
# macOS remembers permissions per signature: set CLIPPY_SIGN_IDENTITY to the name of a
# code-signing certificate from Keychain Access so rebuilds keep them. Without it the app is
# signed ad hoc, and macOS may ask for clipboard access again after each rebuild.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release

version=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
app=target/Clippy.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp target/release/clippy "$app/Contents/MacOS/Clippy"

# LSUIElement keeps the app out of the Dock and the ⌘Tab switcher. The bundle identifier must
# match CLIPPY in src/clipboard.rs.
cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Clippy</string>
    <key>CFBundleDisplayName</key><string>Clippy</string>
    <key>CFBundleIdentifier</key><string>local.clippy</string>
    <key>CFBundleExecutable</key><string>Clippy</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleVersion</key><string>$version</string>
    <key>CFBundleShortVersionString</key><string>$version</string>
    <key>LSMinimumSystemVersion</key><string>13.0</string>
    <key>LSUIElement</key><true/>
    <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF

codesign --force --sign "${CLIPPY_SIGN_IDENTITY:--}" "$app"
echo "Built $app"
