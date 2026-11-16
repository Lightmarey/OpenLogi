# Craft Crown HUD for GNOME

The companion extension draws read-only Shell actors for Craft Crown functions.
The agent owns input, profiles, action execution and display expiry. The HUD uses
the focused application's monitor and retains application and keyboard focus.

## Installation

Pack and install the extension from the repository root.

```sh
gnome-extensions pack --force --podir=po --extra-source=feedback.js --out-dir /tmp \
  crates/openlogi-overlay/gnome-shell-extension/openlogi-crown@openlogi.dev
gnome-extensions install --force /tmp/openlogi-crown@openlogi.dev.shell-extension.zip
```

Log out and back in after installing new extension code on Wayland, then enable it.

```sh
gnome-extensions enable openlogi-crown@openlogi.dev
gnome-extensions prefs openlogi-crown@openlogi.dev
```

Preferences offer Logitech rectangular, GNOME and OpenLogi skins. All three follow
live system light/dark and Shell scale changes. Bottom margin defaults to 32 logical pixels and
accepts 8–160. Preference labels are available in English and Simplified Chinese.

The volume function reads GNOME's shared output mixer. Its value and fill track
volume, mute and the current output device. Unknown readings display the function
icon. Above-normal volume retains its actual percentage while the fill saturates.
Custom function names and icons retain feedback when their rotation actions still
control system volume.

## Connection and recovery

The overlay connects automatically when the extension becomes available. D-Bus
ownership changes retire the old connection and its presentation lease. Re-enabling
or restarting the extension replays the latest valid snapshot. An expiry received
during reconnection stays hidden. Failed connections and incompatible versions
retry once per second. A healthy connection waits for updates and owner changes.

Check the installed bridge version with the following command. This build uses
bridge version 1.

```sh
gdbus call --session --dest org.openlogi.CrownHud \
  --object-path /org/openlogi/CrownHud --method org.openlogi.CrownHud.GetVersion
```

The caller's unique bus name owns its actor. Explicit Hide and loss of that name
end its lease. All descendants are nonreactive and cannot take focus. The HUD
extension and the application's foreground-detection extension have independent
lifecycles. Crown actions continue while the HUD extension is unavailable.

## Native regression tests

The locked Nix shell supplies GNOME Shell, its test tool, PipeWire, GJS and test
utilities. Run the Rust build in the project's Rust environment first.

```sh
cargo test --locked -p openlogi-overlay --no-run --message-format=json \
  > /tmp/openlogi-crown-test-build.json
nix develop path:./crates/openlogi-overlay/gnome-shell-extension/tests
export OPENLOGI_CROWN_TEST_BINARY="$(jq -r \
  'select(.reason == "compiler-artifact" and .target.name == "openlogi-overlay" and .profile.test) | .executable // empty' \
  /tmp/openlogi-crown-test-build.json | tail -n 1)"
OPENLOGI_CROWN_ISOLATED_BUS=1 dbus-run-session \
  --config-file="$OPENLOGI_DBUS_SESSION_CONFIG" -- "$OPENLOGI_CROWN_TEST_BINARY" \
  crown::gnome::tests::bridge_recovers_late_enable_restart_disable_and_version_mismatch \
  --exact --ignored --nocapture
gnome-extensions pack --force --podir=po --extra-source=feedback.js --out-dir /tmp \
  crates/openlogi-overlay/gnome-shell-extension/openlogi-crown@openlogi.dev
bash crates/openlogi-overlay/gnome-shell-extension/tests/run-hud.sh \
  /tmp/openlogi-crown@openlogi.dev.shell-extension.zip
```

The runner creates an isolated session bus, compositor, settings backend and
PipeWire server with two virtual outputs. It checks three skins, light/dark
changes, 100%/200% scale, placement, actor reuse, function selection, native volume/mute/output
changes, extension restart, lease cleanup and retained application/keyboard focus.
Physical input uses a separate Craft acceptance test.

Set `OPENLOGI_CROWN_TEST_LOGITECH_SCREENSHOT`, `OPENLOGI_CROWN_TEST_SCREENSHOT`,
`OPENLOGI_CROWN_TEST_OPENLOGI_SCREENSHOT` and
`OPENLOGI_CROWN_TEST_PROGRESS_SCREENSHOT` to absolute PNG paths to retain captures.
Each skin also accepts its matching `*_DARK_SCREENSHOT` variable.
`OPENLOGI_CROWN_TEST_HIDPI_SCREENSHOT` captures the GNOME skin at 200% scale.

`tests/prefs.js` checks the actual Adwaita preference controls, bidirectional
settings and Chinese translations. Point `OPENLOGI_CROWN_TEST_EXTENSION_DIR` at an
unpacked extension with compiled schemas and run it through `gjs -m` on an isolated
Xvfb display with `GSETTINGS_BACKEND=memory`.

Native compositor tests cover GNOME 50.4. Craft input and HUD initialization were
also exercised on GNOME 51.1. The declared GNOME 45–49 versions and
other platforms require their own runtime verification.
