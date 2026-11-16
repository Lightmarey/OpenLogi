//! Host-independent Craft crown access, using the typed HID++ `0x4600` feature.

use std::sync::Arc;

use hidpp::{
    channel::HidppChannel,
    feature::{CreatableFeature, crown::CrownFeature},
    protocol::v20::{self, Hidpp20Error},
};

pub use hidpp::feature::crown::{
    ActivityState, ButtonState, CrownGesture, CrownMode, CrownUpdate, RatchetMode, ReportingMode,
    RotationState,
};

/// One raw physical movement, shared by action, scrolling and application dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CrownMotion {
    /// Sensor slots, with the firmware's original sign.
    pub slots: i8,
    /// Detents measuring the same movement.
    pub ratchets: i8,
    /// Calibration measured by getInfo.
    pub resolution: openlogi_core::rotary::RotationResolution,
    /// Physical mode observed when capture armed.
    pub ratcheted: bool,
    /// Firmware rotation lifecycle.
    pub phase: RotationState,
    /// Signed sensor slots per second.
    pub speed: i16,
    /// Mechanical press observed for this report before its release edge.
    pub pressed: bool,
}

impl CrownMotion {
    /// Construct uncalibrated motion for callers without a device getInfo result.
    #[must_use]
    pub const fn new(slots: i8, ratchets: i8, ratcheted: bool) -> Self {
        Self {
            slots,
            ratchets,
            ratcheted,
            resolution: openlogi_core::rotary::RotationResolution::UNKNOWN,
            phase: RotationState::Inactive,
            speed: 0,
            pressed: false,
        }
    }
}

/// The Craft crown's driving feature ID.
pub const FEATURE_ID: u16 = 0x4600;

/// An accessor for a resolved crown feature on the inventory-owned channel.
pub struct Crown {
    inner: CrownFeature,
    feature_index: u8,
}

impl Crown {
    /// Bind the typed feature to a device and its dynamically resolved index.
    #[must_use]
    pub fn new(channel: Arc<HidppChannel>, device_index: u8, feature_index: u8) -> Self {
        Self {
            inner: CrownFeature::new(channel, device_index, feature_index),
            feature_index,
        }
    }

    /// The index used to attribute unsolicited reports.
    #[must_use]
    pub fn feature_index(&self) -> u8 {
        self.feature_index
    }

    /// Read sensor resolution and physical control capabilities.
    pub async fn get_info(&self) -> Result<hidpp::feature::crown::CrownInfo, Hidpp20Error> {
        self.inner.get_info().await
    }

    /// Read the reporting and ratchet mode without changing the device.
    pub async fn get_mode(&self) -> Result<CrownMode, Hidpp20Error> {
        self.inner.get_mode().await
    }

    /// Change only reporting; leave ratchet mode and all gesture timeouts alone.
    pub async fn set_reporting(&self, mode: ReportingMode) -> Result<(), Hidpp20Error> {
        self.inner
            .set_mode(hidpp::feature::crown::SetCrownMode {
                diverting: Some(mode),
                ratchet_mode: None,
                rotation_timeout: None,
                short_long_timeout: None,
                double_tap_speed: None,
            })
            .await
    }
}

/// Open the Crown wrapper through the inventory-owned connection.
async fn on_channel(shared: &crate::SharedChannel) -> Result<Arc<CrownFeature>, crate::WriteError> {
    let index = shared.device_index();
    let mut device = hidpp::device::Device::new(Arc::clone(shared.channel()), index)
        .await
        .map_err(|_| crate::WriteError::DeviceUnreachable { index })?;
    crate::write::open_feature::<CrownFeature>(&mut device).await
}

/// Read tactile mode through an existing connection.
pub async fn get_mode_on(shared: &crate::SharedChannel) -> Result<RatchetMode, crate::WriteError> {
    let feature = on_channel(shared).await?;
    feature
        .get_mode()
        .await
        .map(|mode| mode.ratchet_mode)
        .map_err(|error| {
            crate::write::classify_hidpp_error(error, crate::HidppOperation::ReadCrown, FEATURE_ID)
        })
}

/// Change tactile behavior while retaining reporting and gesture timing.
/// A fresh readback is required because setMode only echoes request sentinels.
pub async fn set_mode_on(
    shared: &crate::SharedChannel,
    mode: RatchetMode,
) -> Result<RatchetMode, crate::WriteError> {
    let feature = on_channel(shared).await?;
    let classify = |error| {
        crate::write::classify_hidpp_error(error, crate::HidppOperation::WriteCrown, FEATURE_ID)
    };
    let info = feature.get_info().await.map_err(classify)?;
    if !info
        .controls
        .contains(hidpp::feature::crown::CrownControlCapabilities::MECHANIZED_RATCHET)
    {
        return Err(crate::WriteError::FeatureUnsupported {
            feature_hex: FEATURE_ID,
        });
    }
    let before = feature.get_mode().await.map_err(classify)?;
    if before.ratchet_mode != mode {
        feature
            .set_mode(hidpp::feature::crown::SetCrownMode {
                diverting: None,
                ratchet_mode: Some(mode),
                rotation_timeout: None,
                short_long_timeout: None,
                double_tap_speed: None,
            })
            .await
            .map_err(classify)?;
    }
    let after = feature.get_mode().await.map_err(classify)?.ratchet_mode;
    if after != mode {
        return Err(crate::WriteError::Hidpp(
            "Crown tactile mode did not match its readback".into(),
        ));
    }
    Ok(after)
}

/// Attribute a crown notification, rejecting replies and other devices/features.
#[must_use]
pub fn decode_event(
    message: &v20::Message,
    device_index: u8,
    feature_index: u8,
) -> Option<CrownUpdate> {
    let header = message.header();
    if header.device_index != device_index
        || header.feature_index != feature_index
        || header.software_id.to_lo() != 0
    {
        return None;
    }
    match hidpp::feature::crown::event::decode_event(
        header.function_id.to_lo(),
        &message.extend_payload(),
    )? {
        hidpp::feature::crown::CrownEvent::Update(update) => Some(update),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use hidpp::nibble::U4;

    use super::*;

    #[test]
    fn crown_events_are_attributed_and_responses_never_become_input() {
        let event = |device, feature, function, software| {
            v20::Message::Long(
                v20::MessageHeader {
                    device_index: device,
                    feature_index: feature,
                    function_id: U4::from_lo(function),
                    software_id: U4::from_lo(software),
                },
                [0; 16],
            )
        };
        assert!(decode_event(&event(2, 9, 0, 0), 2, 9).is_some());
        for message in [
            event(1, 9, 0, 0),
            event(2, 8, 0, 0),
            event(2, 9, 0, 5),
            event(2, 9, 1, 0),
        ] {
            assert!(decode_event(&message, 2, 9).is_none());
        }
    }
}
