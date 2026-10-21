# Clippy

Clipboard history for macOS, living in the menu bar. Written in Rust (egui). Saves text and images.

## Using it

- **⌘⌥C** (Cmd+Option+C) opens or closes the window from any app.
- Each item shows the icon and name of the app it was copied from. Images show a thumbnail; hover over one for a bigger preview.
- Type to search: items containing all the words, in any order, ignoring case. Type "image" to see only images.
- **↑ / ↓** select, **Enter** or a click copies the item and returns you to the app you were in. Press ⌘V there to paste.
- **Esc** clears the search, and a second Esc closes the window.
- Right-click an item to delete it.
- The menu bar icon has Open Clippy, Clear History and Quit Clippy.

## Install

```sh
./scripts/install.sh
```

This builds Clippy, puts it in `/Applications` (or `~/Applications` if your account can't write there), starts it, and makes it start at every login. Run it again after changing the code to install the new version. To turn off starting at login without uninstalling, use System Settings → General → Login Items & Extensions.

To remove it: `./scripts/install.sh uninstall`. Your history is kept.

The app is about 4 MB. It uses the macOS system fonts and the SQLite that ships with macOS instead of bundling its own.

While developing: `cargo run` and `cargo test`. `CLIPPY_DB=/tmp/try.db cargo run` uses a separate history file, so your real history is untouched. `./scripts/bundle.sh` only builds `target/Clippy.app`. `cargo test -- --ignored` saves pictures of the window in light and dark mode to `target/ui-preview/`.

## Privacy and storage

- History stays on this Mac in `~/Library/Application Support/clippy/history.db`, readable only by your user. Nothing is sent over the network.
- The newest 1,000 text items and 50 images are kept. Copying something again moves it to the top.
- Not saved: copies that password managers mark as secret or temporary (the nspasteboard.org markers), files copied in Finder, text over 1 MB, and images over 25 MB.
- Newer macOS versions may ask whether Clippy may paste from other apps. Choose **Allow**, or Clippy cannot see what you copy. The setting is in System Settings → Privacy & Security.

## Known behavior

- The window shows once at launch. Esc hides it.
- The window has no title bar or close button. Drag the search bar or the bottom bar to move it, and use Esc or ⌘⌥C to close it.
- While Clippy runs, other apps no longer receive ⌘⌥C. In Finder that is "Copy as Pathname", and in Safari and Chrome it opens the developer tools. To use another shortcut, change `hotkey()` and `HOTKEY_LABEL` in `src/main.rs`.
- A copy that holds both text and a picture, such as cells copied from a spreadsheet, is saved as text.
- Images are put back on the clipboard as PNG, which nearly every app accepts.
- macOS does not say which app put text on the clipboard, so Clippy credits the app that was in front when the copy happened, unless the app names itself on the clipboard. Text from your iPhone (Universal Clipboard) or from scripts such as `pbcopy` therefore shows whichever app was in front.
- macOS remembers permissions per app signature. The default ad hoc signature changes with every build, so macOS may ask again after a rebuild. Set `CLIPPY_SIGN_IDENTITY` to a self-signed certificate from Keychain Access before running `scripts/bundle.sh` to avoid that.
