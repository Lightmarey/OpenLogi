use super::{Adjustment, BrightnessError, ParameterValue};

pub(super) fn read() -> Result<ParameterValue, BrightnessError> {
    Err(BrightnessError::Unavailable)
}

pub(super) fn adjust(_: Adjustment) -> Result<ParameterValue, BrightnessError> {
    Err(BrightnessError::Unavailable)
}
