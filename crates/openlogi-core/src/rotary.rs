//! Shared rotary-control units and settings. Device adapters supply calibration.

use serde::{Deserialize, Serialize};

use crate::binding::Action;
use crate::scroll::ScrollDelta;

/// Requested tactile behavior of a motorized rotary control.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TactileMode {
    /// Use the mode found before software took control.
    #[default]
    Inherit,
    /// Continuous rotation without detents.
    Smooth,
    /// Rotation with physical detents.
    Ratchet,
}

impl TactileMode {
    /// Configuration choices in inspector order.
    pub const ALL: [Self; 3] = [Self::Inherit, Self::Smooth, Self::Ratchet];

    /// Localization key shared by rotary inspectors.
    #[must_use]
    pub const fn translation_key(self) -> &'static str {
        match self {
            Self::Inherit => "keyboard.crown_feel_inherit",
            Self::Smooth => "keyboard.crown_feel_smooth",
            Self::Ratchet => "keyboard.crown_feel_ratchet",
        }
    }
}

/// Native output steps and captured increments per physical revolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RotationResolution {
    /// Native output steps per revolution.
    pub native_res: u16,
    /// Captured sensor increments per revolution.
    pub diverted_res: u16,
}

impl Default for RotationResolution {
    fn default() -> Self {
        Self::UNKNOWN
    }
}

impl RotationResolution {
    /// Unknown calibration retains one output unit per input increment.
    pub const UNKNOWN: Self = Self::new(0, 0);

    /// Calibration measured by a device adapter.
    #[must_use]
    pub const fn new(native_res: u16, diverted_res: u16) -> Self {
        Self {
            native_res,
            diverted_res,
        }
    }

    /// Output units per captured increment, without guessing missing calibration.
    #[must_use]
    pub fn native_per_increment(self) -> f64 {
        if self.native_res == 0 || self.diverted_res == 0 {
            return 1.0;
        }
        f64::from(self.native_res) / f64::from(self.diverted_res)
    }
}

/// Invalid rotary sensitivity supplied by configuration or an editor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("rotary sensitivity must be between 1 and 1000 percent")]
pub struct RotarySensitivityError;

/// Validated rotation speed, expressed as a percentage of native speed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u16", into = "u16")]
pub struct RotarySensitivity(u16);

impl RotarySensitivity {
    /// Lowest supported speed percentage.
    pub const MIN: u16 = 1;
    /// Highest supported speed percentage.
    pub const MAX: u16 = 1000;

    /// Validate a persisted percentage.
    pub fn new(value: u16) -> Result<Self, RotarySensitivityError> {
        if (Self::MIN..=Self::MAX).contains(&value) {
            Ok(Self(value))
        } else {
            Err(RotarySensitivityError)
        }
    }

    /// Configured percentage.
    #[must_use]
    pub const fn percent(self) -> u16 {
        self.0
    }

    /// Multiplier relative to native speed.
    #[must_use]
    pub fn multiplier(self) -> f64 {
        f64::from(self.0) / 100.0
    }
}

impl Default for RotarySensitivity {
    fn default() -> Self {
        Self(100)
    }
}

impl TryFrom<u16> for RotarySensitivity {
    type Error = RotarySensitivityError;
    fn try_from(value: u16) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<RotarySensitivity> for u16 {
    fn from(value: RotarySensitivity) -> Self {
        value.0
    }
}

/// Host-independent settings shared by rotary inputs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[non_exhaustive]
pub struct RotarySettings {
    /// Speed relative to calibrated native output.
    pub sensitivity: RotarySensitivity,
    /// Reverse the physical direction before resolving bindings.
    pub inverted: bool,
}

impl RotarySettings {
    /// Configure speed while retaining direction.
    #[must_use]
    pub const fn with_sensitivity(mut self, sensitivity: RotarySensitivity) -> Self {
        self.sensitivity = sensitivity;
        self
    }

    /// Configure direction while retaining speed.
    #[must_use]
    pub const fn with_inverted(mut self, inverted: bool) -> Self {
        self.inverted = inverted;
        self
    }
}

/// Axis and sign of an action that produces continuous scrolling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollDirection {
    /// Positive vertical output.
    Up,
    /// Negative vertical output.
    Down,
    /// Negative horizontal output.
    Left,
    /// Positive horizontal output.
    Right,
}

impl ScrollDirection {
    /// Classify the shared action vocabulary once for all rotary adapters.
    #[must_use]
    pub fn from_action(action: &Action) -> Option<Self> {
        match action {
            Action::ScrollUp => Some(Self::Up),
            Action::ScrollDown => Some(Self::Down),
            Action::HorizontalScrollLeft => Some(Self::Left),
            Action::HorizontalScrollRight => Some(Self::Right),
            _ => None,
        }
    }

    /// Preserve fractional positive distance in the configured axis and sign.
    #[must_use]
    pub const fn delta(self, distance: f64) -> ScrollDelta {
        match self {
            Self::Up => ScrollDelta::wheel_ticks(0.0, distance),
            Self::Down => ScrollDelta::wheel_ticks(0.0, -distance),
            Self::Left => ScrollDelta::wheel_ticks(-distance, 0.0),
            Self::Right => ScrollDelta::wheel_ticks(distance, 0.0),
        }
    }
}

/// Convert a directional scroll action and positive distance into typed output.
#[must_use]
pub fn scroll_for_action(action: &Action, distance: f64) -> Option<ScrollDelta> {
    ScrollDirection::from_action(action).map(|direction| direction.delta(distance))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibration_and_direction_preserve_fractional_motion() {
        let distance = RotationResolution::new(20, 120).native_per_increment();
        assert_eq!(
            scroll_for_action(&Action::ScrollUp, distance).unwrap().y(),
            1.0 / 6.0
        );
        assert_eq!(
            scroll_for_action(&Action::HorizontalScrollLeft, distance)
                .unwrap()
                .x(),
            -1.0 / 6.0
        );
        assert!(scroll_for_action(&Action::Copy, distance).is_none());
        assert_eq!(RotationResolution::UNKNOWN.native_per_increment(), 1.0);
    }

    #[test]
    fn persisted_sensitivity_is_bounded() {
        for value in [0, 1001, u16::MAX] {
            RotarySensitivity::new(value).unwrap_err();
        }
        assert_eq!(RotarySensitivity::default().multiplier(), 1.0);
        assert_eq!(RotarySensitivity::new(150).unwrap().multiplier(), 1.5);
    }
}
