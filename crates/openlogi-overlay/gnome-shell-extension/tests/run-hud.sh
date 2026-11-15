#!/usr/bin/env bash
# Native compositor and native mixer, isolated from the user's audio devices.
set -euo pipefail
if [[ ${OPENLOGI_CROWN_ISOLATED_BUS:-} != 1 ]]; then
  exec dbus-run-session --config-file="${OPENLOGI_DBUS_SESSION_CONFIG:?}" -- env OPENLOGI_CROWN_ISOLATED_BUS=1 bash "$0" "$@"
fi
extension_zip=$1
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
audio_dir=$(mktemp -d /tmp/openlogi-crown-audio.XXXXXX)
audio_pids=()
cleanup() {
  audio_status=$?
  if ((audio_status != 0)); then cat "$audio_dir/pipewire.log" "$audio_dir/pulse.log" >&2; fi
  for audio_pid in "${audio_pids[@]}"; do kill "$audio_pid" 2>/dev/null || true; done
  for audio_pid in "${audio_pids[@]}"; do wait "$audio_pid" 2>/dev/null || true; done
  rm -rf -- "$audio_dir"
}
trap cleanup EXIT
export XDG_RUNTIME_DIR="$audio_dir"
export PIPEWIRE_RUNTIME_DIR="$audio_dir"
export PULSE_SERVER="unix:$audio_dir/pulse-native"
sed '/^context.objects = \[/a\    { factory = metadata args = { metadata.name = default } }' \
  "${OPENLOGI_PIPEWIRE_CONFIG_DIR:?}/pipewire.conf" >"$audio_dir/pipewire.conf"
sed -e "s|\"unix:native\"|\"unix:$audio_dir/pulse-native\"|" \
  -e 's/#pulse.allow-module-loading = true/pulse.allow-module-loading = true/' \
  "${OPENLOGI_PIPEWIRE_CONFIG_DIR:?}/pipewire-pulse.conf" >"$audio_dir/pulse.conf"
pipewire -c "$audio_dir/pipewire.conf" >"$audio_dir/pipewire.log" 2>&1 &
audio_pids+=("$!")
pipewire-pulse -c "$audio_dir/pulse.conf" >"$audio_dir/pulse.log" 2>&1 &
audio_pids+=("$!")
for _ in {1..60}; do
  if pactl info >/dev/null 2>&1; then break; fi
  sleep 0.05
done
pactl info >/dev/null
pactl load-module module-null-sink sink_name=crown-hud-a sink_properties=device.class=sound >/dev/null
pactl load-module module-null-sink sink_name=crown-hud-b sink_properties=device.class=sound >/dev/null
pactl set-default-sink crown-hud-a
# No session manager runs in this audio sandbox. Publish its resolved default
# alongside Pulse's configured default, without probing any physical device.
pw-metadata -n default 0 default.audio.sink '{"name":"crown-hud-a"}' >/dev/null
pactl set-sink-volume crown-hud-a 25%
pactl set-sink-volume crown-hud-b 75%
OPENLOGI_CROWN_TEST_AUDIO=isolated GSETTINGS_BACKEND=memory \
  gnome-shell-test-tool --headless --disable-animations \
  --extension "$extension_zip" "$script_dir/hud.js"
