//! Host-independent Craft crown access, using the typed HID++ `0x4600` feature.

use std::sync::Arc;

use hidpp::{
    channel::HidppChannel,
    feature::{CreatableFeature, crown::CrownFeature},
    protocol::v20::{self, Hidpp20Error},
};

pub use hidpp::feature::crown::{
    ActivityState, ButtonState, CrownGesture, CrownMode, CrownUpdate, RatchetMode, ReportingMode,
};

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
