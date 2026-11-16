//! System backlight control and authoritative parameter feedback.

use std::sync::Mutex;

use openlogi_core::parameter::{Adjustment, ParameterError, ParameterValue};
use thiserror::Error;

#[cfg(target_os = "linux")]
#[path = "brightness/linux.rs"]
mod platform;
#[cfg(target_os = "macos")]
#[path = "brightness/macos.rs"]
mod platform;
#[cfg(target_os = "windows")]
#[path = "brightness/windows.rs"]
mod platform;
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
#[path = "brightness/unsupported.rs"]
mod platform;

/// Failure to discover, read or adjust a native backlight.
#[derive(Debug, Error)]
pub enum BrightnessError {
    /// This host or display exposes no supported system backlight.
    #[error("system backlight control is unavailable")]
    Unavailable,
    /// A native setting could not be read.
    #[error("could not read system brightness: {0}")]
    Io(#[from] std::io::Error),
    /// A native service rejected or failed the operation.
    #[error("system brightness service failed: {0}")]
    Backend(String),
    /// A driver supplied values outside its declared range.
    #[error("system brightness is outside its reported range")]
    InvalidValue,
    /// The native feedback could not form a valid presentation.
    #[error(transparent)]
    Parameter(#[from] ParameterError),
}

static ADJUSTMENT: Mutex<()> = Mutex::new(());

/// Read the current system backlight. No value is retained or predicted here.
pub fn read() -> Result<ParameterValue, BrightnessError> {
    platform::read()
}

/// Apply one five-percent native step, serialized with other OpenLogi adjustments.
/// Run on an action/background worker rather than an input callback.
pub fn adjust(direction: Adjustment) -> Result<ParameterValue, BrightnessError> {
    let _guard = ADJUSTMENT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    platform::adjust(direction)
}

pub(crate) fn execute(direction: Adjustment) {
    if let Err(error) = adjust(direction) {
        tracing::warn!(%error, "could not adjust system brightness");
    }
}

fn next_level(current: u32, maximum: u32, direction: Adjustment) -> Result<u32, BrightnessError> {
    if maximum == 0 || current > maximum {
        return Err(BrightnessError::InvalidValue);
    }
    let step = (maximum / 20).max(1);
    Ok(match direction {
        Adjustment::Increase => current.saturating_add(step).min(maximum),
        Adjustment::Decrease => current.saturating_sub(step),
    })
}

fn feedback(current: u32, maximum: u32) -> Result<ParameterValue, BrightnessError> {
    if maximum == 0 || current > maximum {
        return Err(BrightnessError::InvalidValue);
    }
    let scaled = u64::from(current) * 10_000 / u64::from(maximum);
    let scaled = i32::try_from(scaled).map_err(|_| BrightnessError::InvalidValue)?;
    Ok(ParameterValue::new(
        scaled,
        0,
        10_000,
        format!("{:.0}%", f64::from(current) * 100.0 / f64::from(maximum)),
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn brightness_steps_and_feedback_use_the_driver_range() {
        assert_eq!(next_level(200, 1000, Adjustment::Increase).unwrap(), 250);
        assert_eq!(next_level(20, 1000, Adjustment::Decrease).unwrap(), 0);
        assert_eq!(next_level(990, 1000, Adjustment::Increase).unwrap(), 1000);
        assert_eq!(feedback(250, 1000).unwrap().text(), "25%");
        assert_eq!(feedback(250, 1000).unwrap().fraction(), 0.25);
        next_level(0, 0, Adjustment::Increase).unwrap_err();
        feedback(1001, 1000).unwrap_err();
    }
}
