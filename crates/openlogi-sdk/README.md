# OpenLogi application SDK

Applications publish rotary tools and authoritative parameter values over a
versioned local JSON-lines protocol. The agent owns source selection, physical
input and device adaptation. The first adapter is the Craft Crown.

## Run the example

```sh
cargo run -p openlogi-sdk --example parameters
```

The terminal preview exposes **Preview zoom** and **Preview item**. Rotation
changes its actual preview state. An unreserved press advances the item. Release edges leave its state unchanged.
The application publishes its resulting value to every active source and retains
its state when the agent restarts.

Open the Craft keyboard's **Keys** panel, select **Crown**, choose the terminal's
application profile and enable Crown function mode. Select `example-preview` as
the application provider and save. Choose the touch-tap selection trigger to use
mechanical presses for the example's item command. Focus that terminal while
using the Crown. Saved local functions resume when the application disconnects.

Development builds can use `OPENLOGI_PROFILE=dev`. The Rust client and agent use
the same existing profile paths. An isolated endpoint, state file and action
limit can also be passed as positional arguments to the example.

```sh
cargo run -p openlogi-sdk --example parameters -- /tmp/preview-sdk.sock /tmp/preview-state.json
```

The mock agent serves the same registration directory and native application
endpoint for GUI development. It supplies scripted device inventory.

## Protocol version 1

Each UTF-8 line contains one JSON message, bounded to 65,536 bytes excluding its
newline. Registration and writes have bounded deadlines. The Rust client owns
the handshake and closes a failed send's connection before it can be reused.

| Direction | Message | Required contents |
|---|---|---|
| Application to agent | `register` | `version`, provider `id` and `name` |
| Agent to application | `ready` | `version`, opaque `connection` |
| Application to agent | `context` | increasing `revision`, a `tools` catalog or `null` |
| Agent to application | `activated` | fresh `session`, `context_revision`, opaque source capabilities, selected tool and reserved selection input |
| Agent to application | `deactivated` | ended `session` and `reason` |
| Agent to application | `selected` | `session` and new tool |
| Agent to application | `input` | session, sequence range, monotonic microseconds, selected tool and real input |
| Application to agent | `parameter` | session, context revision, tool and validated parameter state |
| Agent to application | `error` | stable error `code` and a diagnostic `message` |

Provider and tool IDs contain 1–64 ASCII letters, digits, hyphens or underscores.
Names and parameter text fit 64 characters. Each context has 1–8 uniquely named
tool IDs. Tools specify a shared icon, tactile preference and optional initial
parameter. A parameter contains its real integer value, minimum, maximum and
formatted text. Fixed-point units support fractional application values.

The public Rust types define the exact JSON representation. Build without default
features for the host-free protocol models. The default `client` feature adds
the local endpoint, framing and async Rust client.

## Source and session lifetime

Source IDs contain no hardware serials or configuration keys. Capabilities come
from the adapter's actual discovery. Motion preserves raw slots, detents,
calibration, confirmed tactile mode, phase, speed, touch and mechanical press.
Its signed `amount` also includes user sensitivity and direction settings.

Only an explicitly selected provider can replace an application's local tools.
An activation starts when its adapter admits input. Focus changes, context
replacement, capture teardown and connection loss end that activation. A new
session is required even when the provider ID and tool names stay the same.
Applications release any output they own when deactivated or disconnected.

Touch and press edges stay ordered. Inputs reserved for host selection carry
`reserved = true`. A clean touch tap is consumed by physical rotation before it
reaches the application. Selection messages share the same queue as input.

Adjacent movement-phase rotation reports can merge within the same activation,
tool, movement direction, press/contact state, calibration and tactile mode. `first` and `last`
retain the complete input sequence range. Raw motion and calibrated amount are
summed. Speed and monotonic time reflect the latest report. Start/stop phases,
edges and selection changes remain distinct. A full bounded queue ends the
provider's lease and restores saved local functions.

Parameter updates must name a live session owned by that exact connection and
context revision. Late updates receive a stale-session/context error. Dynamic
tools and values belong to the connection lease and are never saved to config.
A new activation reuses the context's latest authoritative values. Applications
publish fresh parameter readings when activated and whenever their native values change.

## Transport and verification

Unix uses a user-only socket beside the profile's configuration file. Windows
uses a user/profile-derived named pipe and the creating user's default access
control. The SDK protocol version is independent of the internal GUI IPC.

```sh
cargo test -p openlogi-sdk --all-targets
cargo test -p openlogi-agent-core applications
cargo test -p openlogi-agent-core crown_application
cargo build -p openlogi-sdk --example parameters
OPENLOGI_SDK_EXAMPLE="$PWD/target/debug/examples/parameters" \
  cargo test -p openlogi-agent-core sdk_parameter_example -- --ignored --nocapture
```

The process test exercises real preview state, command execution, read-only HUD
feedback, endpoint restart, automatic client reconnection and lease cleanup.
The Logitech Craft WebSocket and manifest adapter is planned as a later PR.
