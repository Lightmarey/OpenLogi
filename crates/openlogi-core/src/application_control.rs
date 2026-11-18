//! Device-independent application control references and identifier validation.

use nutype::nutype;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Maximum number of functions offered by a rotary control or application.
pub const MAX_FUNCTIONS: usize = 8;

/// Device-independent category of a physical control source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlKind {
    /// A relative rotary encoder or wheel.
    Rotary,
}

bitflags::bitflags! {
    /// Independently discoverable physical features of a control source.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
    pub struct ControlFeatures: u8 {
        /// Physical contact edges are available.
        const TOUCH = 1;
        /// Mechanical press edges are available.
        const PRESS = 2;
        /// Reports include physical detents.
        const DETENTS = 4;
        /// Hardware supports motorized smooth/ratchet selection.
        const TACTILE = 8;
    }
}

/// Real source capabilities, reported by a device adapter before its first input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlCapabilities {
    /// Semantic control category.
    pub kind: ControlKind,
    /// Features discovered by the adapter.
    pub features: ControlFeatures,
    /// Actual slot-to-native calibration reported by the device.
    pub resolution: crate::rotary::RotationResolution,
}

/// An invalid persisted provider or protocol control identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
#[error("identifier must contain 1–64 ASCII letters, digits, '-' or '_'")]
pub struct IdentifierError;

/// A stable application provider reference. It remains valid while offline.
#[nutype(
    validate(with = validate_identifier, error = IdentifierError),
    derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, AsRef, TryFrom, Into, Serialize, Deserialize)
)]
pub struct ProviderId(String);

impl ProviderId {
    /// Validate a provider's stable configuration identity.
    pub fn new(id: String) -> Result<Self, IdentifierError> {
        Self::try_new(id)
    }
}

/// A tool or control identity independent of any physical device.
#[nutype(
    validate(with = validate_identifier, error = IdentifierError),
    derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, AsRef, TryFrom, Into, Serialize, Deserialize)
)]
pub struct ControlId(String);

impl ControlId {
    /// Validate a stable tool or control identity.
    pub fn new(id: String) -> Result<Self, IdentifierError> {
        Self::try_new(id)
    }
}

/// A literal, bounded name shared by application tools and native functions.
#[nutype(
    sanitize(trim),
    validate(with = validate_label, error = LabelError),
    derive(Clone, Debug, PartialEq, Eq, AsRef, TryFrom, Into, Serialize, Deserialize)
)]
pub struct ControlLabel(String);

/// A label cannot be empty or exceed the HUD's available text budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
#[error("control name must contain 1–64 characters")]
pub struct LabelError;

impl ControlLabel {
    /// Trim and validate a literal display name.
    pub fn new(name: &str) -> Result<Self, LabelError> {
        Self::try_new(name)
    }
}

fn validate_label(name: &str) -> Result<(), LabelError> {
    if name.is_empty() || name.chars().count() > 64 {
        Err(LabelError)
    } else {
        Ok(())
    }
}

/// Validate an identity shared by persisted functions and application controls.
///
/// All persisted control and protocol identifiers use this same lexical policy.
pub fn validate_identifier(id: &str) -> Result<(), IdentifierError> {
    if !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        Ok(())
    } else {
        Err(IdentifierError)
    }
}

/// Provider presence for a read-only configuration picker.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ProviderDescriptor {
    /// Stable identity saved by an application profile.
    pub id: ProviderId,
    /// Literal application-provided display name.
    pub name: String,
}

impl ProviderDescriptor {
    /// Describe a registered provider using an already-validated name.
    #[must_use]
    pub const fn new(id: ProviderId, name: String) -> Self {
        Self { id, name }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Serialize, Deserialize)]
    struct Profile {
        provider: ProviderId,
    }

    #[test]
    fn provider_references_are_validated_on_load_and_survive_offline() {
        let id = ProviderId::new("example-parameters".into()).unwrap();
        assert_eq!(id.as_ref(), "example-parameters");
        ProviderId::new(String::new()).unwrap_err();
        ProviderId::new("bad provider".into()).unwrap_err();
        let text = toml::to_string(&Profile {
            provider: id.clone(),
        })
        .unwrap();
        assert_eq!(toml::from_str::<Profile>(&text).unwrap().provider, id);
        toml::from_str::<Profile>("provider = 'bad provider'").unwrap_err();
    }
}
