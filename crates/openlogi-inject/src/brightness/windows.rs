//! Windows' built-in monitor backlight provider. WMI owns COM initialization.

use serde::{Deserialize, Serialize};
use wmi::WMIConnection;

use super::{Adjustment, BrightnessError, ParameterValue, feedback, next_level};

#[derive(Deserialize)]
#[serde(rename = "WmiMonitorBrightness", rename_all = "PascalCase")]
struct Monitor {
    active: bool,
    current_brightness: u8,
    instance_name: String,
}

#[derive(Deserialize)]
#[serde(rename = "WmiMonitorBrightnessMethods", rename_all = "PascalCase")]
struct Methods {
    instance_name: String,
    #[serde(rename = "__Path")]
    path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct SetBrightness {
    timeout: u32,
    brightness: u8,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct MethodResult {
    return_value: u32,
}

impl From<wmi::WMIError> for BrightnessError {
    fn from(error: wmi::WMIError) -> Self {
        Self::Backend(error.to_string())
    }
}

fn connection() -> Result<WMIConnection, BrightnessError> {
    WMIConnection::with_namespace_path("ROOT\\WMI").map_err(BrightnessError::from)
}

fn monitor(connection: &WMIConnection) -> Result<Monitor, BrightnessError> {
    let mut monitors: Vec<Monitor> = connection.query().map_err(BrightnessError::from)?;
    monitors.retain(|monitor| monitor.active);
    monitors.sort_by(|left, right| left.instance_name.cmp(&right.instance_name));
    monitors
        .into_iter()
        .next()
        .ok_or(BrightnessError::Unavailable)
}

pub(super) fn read() -> Result<ParameterValue, BrightnessError> {
    let monitor = monitor(&connection()?)?;
    feedback(u32::from(monitor.current_brightness), 100)
}

pub(super) fn adjust(direction: Adjustment) -> Result<ParameterValue, BrightnessError> {
    // The connection and all its COM values remain on this action worker.
    let connection = connection()?;
    let current = monitor(&connection)?;
    let methods: Vec<Methods> = connection.query().map_err(BrightnessError::from)?;
    let methods = methods
        .into_iter()
        .find(|methods| methods.instance_name == current.instance_name)
        .ok_or(BrightnessError::Unavailable)?;
    let desired = next_level(u32::from(current.current_brightness), 100, direction)?;
    let result: MethodResult = connection
        .exec_instance_method::<Methods, _>(
            &methods.path,
            "WmiSetBrightness",
            SetBrightness {
                timeout: 0,
                brightness: u8::try_from(desired).map_err(|_| BrightnessError::InvalidValue)?,
            },
        )
        .map_err(BrightnessError::from)?;
    if result.return_value != 0 {
        return Err(BrightnessError::Backend(format!(
            "WmiSetBrightness returned {}",
            result.return_value
        )));
    }
    let monitors: Vec<Monitor> = connection.query().map_err(BrightnessError::from)?;
    let actual = monitors
        .into_iter()
        .find(|monitor| monitor.instance_name == current.instance_name)
        .ok_or(BrightnessError::Unavailable)?;
    feedback(u32::from(actual.current_brightness), 100)
}
