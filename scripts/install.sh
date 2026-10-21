#!/bin/zsh
# Installs Clippy in Applications and makes it start every time you log in.
#
#   ./scripts/install.sh             install, or update to the current code
#   ./scripts/install.sh uninstall   quit Clippy, stop it starting at login, and remove the app
#
# Your clipboard history in ~/Library/Application Support/clippy is kept either way.
set -euo pipefail
cd "$(dirname "$0")/.."

# /Applications needs an admin account; otherwise the app goes in your own Applications folder.
applications=/Applications
[[ -w $applications ]] || applications=$HOME/Applications
app=$applications/Clippy.app
# A launch agent is how macOS starts programs at login; the label matches the bundle identifier.
agent=$HOME/Library/LaunchAgents/local.clippy.plist
session=gui/$(id -u)

stop_clippy() {
  launchctl bootout "$session" "$agent" 2>/dev/null || true
  pkill -x Clippy 2>/dev/null || true
}

if [[ ${1:-} == uninstall ]]; then
  stop_clippy
  rm -f "$agent"
  rm -rf "$app"
  echo "Clippy is removed. Your history is still in ~/Library/Application Support/clippy."
  exit 0
fi

./scripts/bundle.sh
stop_clippy
mkdir -p "$applications" "${agent:h}"
rm -rf "$app"
cp -R target/Clippy.app "$app"

cat > "$agent" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>local.clippy</string>
    <key>ProgramArguments</key>
    <array><string>$app/Contents/MacOS/Clippy</string></array>
    <key>RunAtLoad</key><true/>
    <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
EOF
plutil -lint -s "$agent"

# Loading the agent also starts Clippy now, without waiting for the next login.
launchctl bootstrap "$session" "$agent"
echo "Clippy is installed in $applications and will start at every login. Press ⌘⌥C to open it."
