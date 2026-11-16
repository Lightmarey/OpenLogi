# OpenLogi Frontmost Window — GNOME Shell extension

GNOME (Mutter) does not let ordinary clients see which window is focused on
Wayland, and it implements neither `wlr-foreign-toplevel` nor a focused-window
portal. This minimal extension bridges that gap: it exports the WM_CLASS of the
focused window over D-Bus and emits every native focus change so OpenLogi's
`gnome-shell` backend can drive per-app mouse-profile switching without
polling.

The extension also excludes `openlogi-action-ring` windows from GNOME's window
animations. GNOME's opening animation scales the surface, changing its local
pointer coordinates while it appears. The ring anchors to the first pointer
report, so without this exception it can stay far from the cursor after the
animation ends, especially near screen edges ([#1210](https://github.com/AprilNEA/OpenLogi/issues/1210)).
Other windows retain their normal animation policy; disabling the extension
restores GNOME's original policy. The global animation setting is never changed.

The bridge also reports four held-modifier flags for Crown function selection.
This reads the compositor's current state and works with standard device
permissions on native Wayland.

## D-Bus surface

- name: `org.openlogi.Frontmost`
- path: `/org/openlogi/Frontmost`
- method: `GetFocusedWmClass() -> s` (empty string when nothing is focused)
- signal: `FocusedWmClassChanged(s)` (emitted on each GNOME focus change)
- `GetHeldModifiers() -> (bbbb)` reports Shift, Control, Alt and Super.

## Install

```sh
UUID=openlogi-frontmost@openlogi.dev
DEST="$HOME/.local/share/gnome-shell/extensions/$UUID"
mkdir -p "$DEST"
cp metadata.json ./*.js "$DEST"/
```

On Wayland the shell cannot be reloaded in place, so **log out and back in** to
let GNOME pick up the newly added extension, then enable it:

```sh
gnome-extensions enable "$UUID"
gnome-extensions info "$UUID"   # State should be ACTIVE
```

## Verify

```sh
# Introspect the service:
busctl --user introspect org.openlogi.Frontmost /org/openlogi/Frontmost

# Focus a window, then query it:
gdbus call --session \
  -d org.openlogi.Frontmost \
  -o /org/openlogi/Frontmost \
  -m org.openlogi.Frontmost.GetFocusedWmClass

# Watch focus changes:
gdbus monitor --session \
  -d org.openlogi.Frontmost \
  -o /org/openlogi/Frontmost
```

If the call prints the focused window's WM_CLASS and the monitor reports
`FocusedWmClassChanged` while switching windows, OpenLogi's GNOME backend will
pick it up automatically the next time the hook starts.

With GNOME animations enabled, open the Actions Ring repeatedly over a native
Wayland application, first near the center and then near each edge. Its center
should match the pointer away from edges; near an edge it should move only far
enough inward to keep the ring visible. Hover and click a slot to verify its
hit target matches the drawing. Ordinary application windows should still
animate. Disable the extension to verify that its animation override is removed.

The animation-policy regression tests need Node.js 22 or newer and no running shell:

```sh
node --test crates/openlogi-hook/gnome-shell-extension/tests/ringAnimation.test.mjs
```

Run that command from the repository root. It does not replace the live GNOME
placement check above.

The native modifier regression uses the locked GNOME environment and a private
headless compositor. It injects compositor-owned virtual key events and queries
the real D-Bus method, including both Shift keys and balanced release.

```sh
nix develop path:./crates/openlogi-overlay/gnome-shell-extension/tests
gnome-extensions pack --force --extra-source=ringAnimation.js --out-dir /tmp \
  crates/openlogi-hook/gnome-shell-extension/openlogi-frontmost@openlogi.dev
bash crates/openlogi-hook/gnome-shell-extension/tests/run-modifiers.sh \
  /tmp/openlogi-frontmost@openlogi.dev.shell-extension.zip
```

The Rust bridge regression additionally covers extension restart, an older
extension and the distinction between an unavailable source and released keys.
Build the hook tests in the Rust environment, then run the resulting test binary
on an isolated session bus.

```sh
cargo test -p openlogi-hook --no-run --locked --message-format=json \
  > /tmp/openlogi-hook-test-build.json
nix develop path:./crates/openlogi-overlay/gnome-shell-extension/tests
export OPENLOGI_HOOK_TEST_BINARY="$(jq -r \
  'select(.reason == "compiler-artifact" and .target.name == "openlogi_hook" and .profile.test) | .executable // empty' \
  /tmp/openlogi-hook-test-build.json | tail -n 1)"
OPENLOGI_MODIFIER_ISOLATED_BUS=1 dbus-run-session \
  --config-file="$OPENLOGI_DBUS_SESSION_CONFIG" -- "$OPENLOGI_HOOK_TEST_BINARY" \
  linux::foreground::gnome_shell::tests::gnome_modifier_snapshots_survive_extension_restart_and_older_versions \
  --exact --ignored --nocapture
```

## Notes

- The `shell-version` list in `metadata.json` covers GNOME 45–51. Native modifier
  tests passed on GNOME 50.4 and 51.1. The APIs used here (`Gio.DBusExportedObject`,
  `global.display.focus_window`, `Meta.Window.get_wm_class`) have been stable
  across these versions.
- The extension name/UUID and D-Bus name follow the project's namespace. Keep
  the proxy in `crates/openlogi-hook/src/linux/foreground/gnome_shell.rs` aligned
  when those names change.
- The scoped animation exception uses GNOME Shell's `_shouldAnimateActor` hook
  through its `InjectionManager`. When adding a new shell version, check that
  hook and run the live placement check as well as the policy tests.
