//! Public IOKit display parameters for a built-in Apple backlight.

#![expect(
    unsafe_code,
    reason = "typed IOKit display calls require valid service and out pointers"
)]

use std::num::NonZeroU32;

use objc2_core_foundation::{CFDictionary, CFRetained, CFString};
use objc2_io_kit::{
    IODisplayGetFloatParameter, IODisplaySetFloatParameter, IOObjectRelease,
    IOServiceGetMatchingService, IOServiceMatching, kIOMainPortDefault,
};

use super::{Adjustment, BrightnessError, ParameterValue, feedback, next_level};

struct Backlight(NonZeroU32);

impl Drop for Backlight {
    fn drop(&mut self) {
        let _ = IOObjectRelease(self.0.get());
    }
}

impl Backlight {
    fn discover() -> Result<Self, BrightnessError> {
        // SAFETY: the class name is a NUL-terminated static. The generated
        // binding owns the returned dictionary and transfers it to the lookup.
        let matching = unsafe { IOServiceMatching(c"AppleBacklightDisplay".as_ptr()) }
            .ok_or(BrightnessError::Unavailable)?;
        let matching = CFRetained::<CFDictionary>::from(&*matching);
        // SAFETY: IOKit supplies the main port. A successful lookup returns an
        // owned service reference, released by Backlight's Drop.
        let raw = unsafe { IOServiceGetMatchingService(kIOMainPortDefault, Some(matching)) };
        NonZeroU32::new(raw)
            .map(Self)
            .ok_or(BrightnessError::Unavailable)
    }

    fn read(&self) -> Result<u32, BrightnessError> {
        let name = CFString::from_str("brightness");
        let mut value = 0.0_f32;
        // SAFETY: this object owns a live display service, the parameter name
        // lives through the call, and value is a writable local.
        let result =
            unsafe { IODisplayGetFloatParameter(self.0.get(), 0, Some(&name), &raw mut value) };
        status(result)?;
        scaled(value)
    }

    fn set(&self, value: u32) -> Result<(), BrightnessError> {
        let name = CFString::from_str("brightness");
        let value =
            f32::from(u16::try_from(value).map_err(|_| BrightnessError::InvalidValue)?) / 10_000.0;
        // SAFETY: the service and CFString are owned by this scope. The value
        // is within the public display parameter's normalized range.
        status(unsafe { IODisplaySetFloatParameter(self.0.get(), 0, Some(&name), value) })
    }
}

fn status(result: i32) -> Result<(), BrightnessError> {
    if result == 0 {
        Ok(())
    } else {
        Err(BrightnessError::Backend(format!(
            "IOKit display parameter returned {result}"
        )))
    }
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "the finite normalized value was checked before conversion to 0–10000 units"
)]
fn scaled(value: f32) -> Result<u32, BrightnessError> {
    if !value.is_finite() || !(0.0..=1.0).contains(&value) {
        return Err(BrightnessError::InvalidValue);
    }
    Ok((value * 10_000.0).round() as u32)
}

pub(super) fn read() -> Result<ParameterValue, BrightnessError> {
    feedback(Backlight::discover()?.read()?, 10_000)
}

pub(super) fn adjust(direction: Adjustment) -> Result<ParameterValue, BrightnessError> {
    let backlight = Backlight::discover()?;
    backlight.set(next_level(backlight.read()?, 10_000, direction)?)?;
    feedback(backlight.read()?, 10_000)
}
