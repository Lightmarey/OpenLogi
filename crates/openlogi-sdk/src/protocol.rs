use std::collections::BTreeSet;

use openlogi_core::application_control::MAX_FUNCTIONS;
pub use openlogi_core::application_control::{
    ControlCapabilities, ControlFeatures, ControlId, ControlKind, ControlLabel, ProviderId,
};
pub use openlogi_core::binding::ActionRingIcon as ToolIcon;
pub use openlogi_core::parameter::{ParameterState, ParameterValue};
pub use openlogi_core::rotary::{RotationResolution, TactileMode};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Public application protocol version, independent of the agent's internal IPC.
pub const VERSION: u32 = 1;
/// Maximum encoded JSON line, excluding its newline.
pub const MAX_FRAME_BYTES: usize = 65_536;

/// Protocol-level validation failure.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ProtocolError {
    /// Both peers must implement this protocol version.
    #[error("application protocol {received} is incompatible with {VERSION}")]
    Version { received: u32 },
    /// Each context offers a nonempty, bounded tool set.
    #[error("an application context must contain 1–8 tools")]
    ToolCount,
    /// Tool identities remain unique within a context.
    #[error("application tool IDs must be unique")]
    DuplicateTool,
    /// Messages are bounded before entering a transport queue.
    #[error("application message exceeds the maximum frame size")]
    FrameSize,
}

/// Apply the protocol handshake's version policy.
pub fn require_version(received: u32) -> Result<(), ProtocolError> {
    if received == VERSION {
        Ok(())
    } else {
        Err(ProtocolError::Version { received })
    }
}

/// Registration identity chosen explicitly in an application profile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    /// Stable configuration identity.
    pub id: ProviderId,
    /// Literal display name shown by the configuration picker.
    pub name: ControlLabel,
}

/// An application-owned tool, independent of the originating device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    /// Stable tool identity used for selection and value updates.
    pub id: ControlId,
    /// Literal tool name.
    pub name: ControlLabel,
    /// Shared embedded icon, requiring no external assets.
    pub icon: ToolIcon,
    /// Tactile preference resolved by the physical adapter.
    pub tactile: TactileMode,
    /// Initial real parameter value, or `None` for a command tool.
    pub parameter: Option<ParameterState>,
}

/// A validated, ordered context catalog. Dynamic data remains connection-scoped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<Tool>", into = "Vec<Tool>")]
pub struct ToolSet(Vec<Tool>);

impl ToolSet {
    /// Validate tool count and identities before publishing a context.
    pub fn new(tools: Vec<Tool>) -> Result<Self, ProtocolError> {
        if tools.is_empty() || tools.len() > MAX_FUNCTIONS {
            return Err(ProtocolError::ToolCount);
        }
        if tools
            .iter()
            .map(|tool| &tool.id)
            .collect::<BTreeSet<_>>()
            .len()
            != tools.len()
        {
            return Err(ProtocolError::DuplicateTool);
        }
        Ok(Self(tools))
    }

    /// Tools in function-selection order.
    #[must_use]
    pub fn tools(&self) -> &[Tool] {
        &self.0
    }
}
impl TryFrom<Vec<Tool>> for ToolSet {
    type Error = ProtocolError;
    fn try_from(tools: Vec<Tool>) -> Result<Self, Self::Error> {
        Self::new(tools)
    }
}
impl From<ToolSet> for Vec<Tool> {
    fn from(tools: ToolSet) -> Self {
        tools.0
    }
}

/// Identity of one activation. It is opaque and unique within an agent lifetime.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(pub u64);

/// A source identity and the real capabilities of its current adapter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlSource {
    /// Opaque source identity. It contains no device serial or configuration key.
    pub id: ControlId,
    /// Real capabilities reported by the source's adapter.
    pub capabilities: ControlCapabilities,
}

/// Physical input reserved by the host for cycling tools.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionInput {
    Press,
    TouchTap,
    None,
}

/// One active source and application context.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Activation {
    /// Fresh identity required by every parameter update.
    pub session: SessionId,
    /// Provider-authored, strictly increasing context revision.
    pub context_revision: u64,
    /// Actual source capabilities.
    pub source: ControlSource,
    /// Initial tool selected by the host.
    pub selected: ControlId,
    /// Input the host reserves for selection.
    pub selection: SelectionInput,
}

/// Native rotation report phase. Start and stop are ordered edges.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RotationPhase {
    Start,
    Move,
    Stop,
    Inactive,
    Unknown(u8),
}

/// Calibrated motion and the underlying adapter report.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rotation {
    /// Signed native-wheel increments after source calibration and user sensitivity.
    pub amount: f64,
    /// Signed raw slots. Coalesced frames retain their complete sum.
    pub raw: i64,
    /// Signed physical detents. May remain zero during sub-detent movement.
    pub detents: i64,
    /// Device-reported slot-to-native calibration.
    pub resolution: RotationResolution,
    /// Confirmed physical mode used to interpret this report.
    pub tactile: TactileMode,
    /// Native start, movement or stop phase.
    pub phase: RotationPhase,
    /// Real mechanical press state during this movement.
    pub pressed: bool,
    /// Physical contact state during the report, when the source has touch sensing.
    pub touched: bool,
    /// Most recent instantaneous raw slot speed, in slots per second.
    pub speed: i32,
}

/// Ordered input delivered to the selected tool.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlInput {
    /// A physical contact edge.
    Touch { down: bool },
    /// A mechanical press edge, with host-selection reservation explicitly marked.
    Press { down: bool, reserved: bool },
    /// A clean touch tap. Rotation consumes the tap intent in the device adapter.
    Tap { reserved: bool },
    /// One native report, or a coalesced run of compatible movement reports.
    Rotation(Rotation),
}

/// Application-to-agent messages.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClientMessage {
    /// Must be the first message on a connection.
    Register { version: u32, provider: Provider },
    /// Replace the current document/tool context. `None` withdraws its tools.
    Context {
        revision: u64,
        tools: Option<ToolSet>,
    },
    /// Publish a real value for one currently active tool.
    Parameter {
        session: SessionId,
        context_revision: u64,
        tool: ControlId,
        value: ParameterState,
    },
}

impl ClientMessage {
    /// Construct the only supported registration handshake.
    #[must_use]
    pub const fn register(provider: Provider) -> Self {
        Self::Register {
            version: VERSION,
            provider,
        }
    }
}

/// Why an activation ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeactivationReason {
    FocusChanged,
    ContextReplaced,
    CaptureEnded,
    Disconnected,
    SlowClient,
    Shutdown,
}

/// Structured error category, stable across message wording changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    Version,
    InvalidMessage,
    DuplicateProvider,
    StaleContext,
    StaleSession,
    UnknownTool,
}

/// Agent-to-application messages.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ServerMessage {
    /// Registration succeeded. The connection identity is opaque.
    Ready { version: u32, connection: u64 },
    /// A source entered this application's current context.
    Activated(Activation),
    /// An activation ended. All its outstanding updates are stale.
    Deactivated {
        session: SessionId,
        reason: DeactivationReason,
    },
    /// Host selection moved to another tool.
    Selected { session: SessionId, tool: ControlId },
    /// An ordered input range. A coalesced frame spans first through last.
    Input {
        session: SessionId,
        first: u64,
        last: u64,
        time_us: u64,
        tool: ControlId,
        event: ControlInput,
    },
    /// A failed request. It never partially replaces the current context.
    Error { code: ErrorCode, message: String },
}

impl ServerMessage {
    /// Construct the current-version handshake acknowledgement.
    #[must_use]
    pub const fn ready(connection: u64) -> Self {
        Self::Ready {
            version: VERSION,
            connection,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incoming_catalogs_and_values_share_the_validated_core_models() {
        ToolSet::new(Vec::new()).unwrap_err();
        let tool = Tool {
            id: ControlId::new("zoom".into()).unwrap(),
            name: ControlLabel::new("Zoom").unwrap(),
            icon: ToolIcon::Gauge,
            tactile: TactileMode::Smooth,
            parameter: None,
        };
        ToolSet::new(vec![tool.clone(), tool.clone()]).unwrap_err();
        ToolSet::new(vec![tool.clone(); MAX_FUNCTIONS + 1]).unwrap_err();
        let message = ClientMessage::Context {
            revision: 1,
            tools: Some(ToolSet::new(vec![tool]).unwrap()),
        };
        let encoded = serde_json::to_string(&message).unwrap();
        assert_eq!(
            serde_json::from_str::<ClientMessage>(&encoded).unwrap(),
            message
        );
        serde_json::from_str::<ClientMessage>(r#"{"type":"context","revision":1,"tools":[]}"#)
            .unwrap_err();
        serde_json::from_str::<ParameterValue>(r#"{"value":2,"minimum":1,"maximum":1,"text":"2"}"#)
            .unwrap_err();
        require_version(VERSION + 1).unwrap_err();
    }
}
