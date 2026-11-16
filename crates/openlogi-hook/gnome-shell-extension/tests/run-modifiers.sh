#!/usr/bin/env bash
# Native compositor-owned key state on a private session bus and runtime.
set -euo pipefail
if [[ ${OPENLOGI_MODIFIER_ISOLATED_BUS:-} != 1 ]]; then
  exec dbus-run-session --config-file="${OPENLOGI_DBUS_SESSION_CONFIG:?}" -- \
    env OPENLOGI_MODIFIER_ISOLATED_BUS=1 bash "$0" "$@"
fi
extension_zip=${1:?Provide the desktop bridge extension archive}
shift
script_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
modifier_runtime=$(mktemp -d /tmp/openlogi-modifier-test.XXXXXX)
trap 'rm -rf -- "$modifier_runtime"' EXIT
export XDG_RUNTIME_DIR="$modifier_runtime"
gnome-shell-test-tool --headless --disable-animations --extension "$extension_zip" \
  "$@" "$script_dir/modifiers.js"
