//! Authoritative parameter feedback shared by native controls and applications.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Direction of one relative parameter adjustment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Adjustment {
    /// Increase the parameter by its native step.
    Increase,
    /// Decrease the parameter by its native step.
    Decrease,
}

/// Current availability of an authoritative parameter producer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParameterState {
    /// A successfully measured or application-published value.
    Available(ParameterValue),
    /// The producer is offline or its latest read failed.
    Unavailable,
}

/// Invalid application or native parameter feedback.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Error)]
pub enum ParameterError {
    /// A visual range must have a positive span.
    #[error("parameter minimum must be less than maximum")]
    Range,
    /// The authoritative formatted label must fit its presenter.
    #[error("parameter text must contain 1–64 characters")]
    Text,
}

/// A real value and its display range. Applications may use fixed-point units.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawValue")]
pub struct ParameterValue {
    value: i32,
    minimum: i32,
    maximum: i32,
    text: String,
}

#[derive(Deserialize)]
struct RawValue {
    value: i32,
    minimum: i32,
    maximum: i32,
    text: String,
}

impl TryFrom<RawValue> for ParameterValue {
    type Error = ParameterError;
    fn try_from(raw: RawValue) -> Result<Self, Self::Error> {
        Self::new(raw.value, raw.minimum, raw.maximum, raw.text)
    }
}

impl ParameterValue {
    /// Validate a measured value. Out-of-range values retain their real text.
    pub fn new(
        value: i32,
        minimum: i32,
        maximum: i32,
        text: String,
    ) -> Result<Self, ParameterError> {
        if minimum >= maximum {
            return Err(ParameterError::Range);
        }
        if text.trim().is_empty() || text.chars().count() > 64 {
            return Err(ParameterError::Text);
        }
        Ok(Self {
            value,
            minimum,
            maximum,
            text,
        })
    }

    /// Measured value in the provider's units.
    #[must_use]
    pub const fn value(&self) -> i32 {
        self.value
    }

    /// Lower end of the provider's visual range.
    #[must_use]
    pub const fn minimum(&self) -> i32 {
        self.minimum
    }

    /// Upper end of the provider's visual range.
    #[must_use]
    pub const fn maximum(&self) -> i32 {
        self.maximum
    }

    /// Authoritative formatted value, rendered as plain text.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Fractional fill, clamped to the visual range without integer overflow.
    #[must_use]
    pub fn fraction(&self) -> f64 {
        ((f64::from(self.value) - f64::from(self.minimum))
            / (f64::from(self.maximum) - f64::from(self.minimum)))
        .clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authoritative_values_validate_ranges_and_keep_out_of_range_text() {
        ParameterValue::new(0, 1, 1, "0".into()).unwrap_err();
        ParameterValue::new(0, 0, 1, " ".into()).unwrap_err();
        let value = ParameterValue::new(125, 0, 100, "125%".into()).unwrap();
        assert_eq!(value.fraction(), 1.0);
        assert_eq!(value.text(), "125%");
        let full = ParameterValue::new(0, i32::MIN, i32::MAX, "0".into()).unwrap();
        assert!((full.fraction() - 0.5).abs() < 1e-9);
        assert_eq!(
            toml::from_str::<ParameterValue>(&toml::to_string(&value).unwrap()).unwrap(),
            value
        );
    }
}
