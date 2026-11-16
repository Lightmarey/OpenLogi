//! Crown contact edges and detent/slot pulses, without host input or action policy.

use openlogi_core::binding::ButtonId;
use tokio::sync::mpsc;

use super::CapturedInput;
use crate::crown::{ActivityState, ButtonState, CrownGesture, CrownUpdate};

/// Keep Crown report attribution and contact normalization together.
pub(super) fn report_handler(
    armed: Option<&super::arm::ArmedCrown>,
    accum: std::sync::Arc<std::sync::Mutex<CrownAccum>>,
    device_index: u8,
    sink: mpsc::UnboundedSender<CapturedInput>,
) -> impl Fn(&hidpp::protocol::v20::Message) + Send + Sync + 'static {
    let crown = armed.map(|armed| (armed.crown.feature_index(), armed.ratcheted));
    if let Some(armed) = armed {
        accum
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .resolution = armed.resolution;
    }
    move |message| {
        if let Some((index, ratcheted)) = crown
            && let Some(event) = crate::crown::decode_event(message, device_index, index)
        {
            accum
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .on_event(event, ratcheted, &sink);
        }
    }
}

#[derive(Default)]
enum TapContact {
    Armed,
    Pending,
    #[default]
    Consumed,
}

#[derive(Default)]
pub(super) struct CrownAccum {
    touched: bool,
    pressed: bool,
    rotating: bool,
    tap_contact: TapContact,
    resolution: openlogi_core::rotary::RotationResolution,
}

impl CrownAccum {
    pub(super) fn on_event(
        &mut self,
        event: CrownUpdate,
        ratcheted: bool,
        sink: &mpsc::UnboundedSender<CapturedInput>,
    ) {
        // Unknown states carry no evidence of an edge. Keep the previous hold
        // rather than inventing a release, which could commit an open ring.
        let touched = match event.touch {
            ActivityState::Start | ActivityState::Active => Some(true),
            ActivityState::Inactive | ActivityState::Stop => Some(false),
            _ => None,
        };
        let pressed = match event.button {
            ButtonState::Press
            | ButtonState::ShortPressActive
            | ButtonState::LongPress
            | ButtonState::LongPressActive => Some(true),
            ButtonState::Inactive | ButtonState::Release => Some(false),
            _ => None,
        };
        if touched == Some(true) && !self.touched {
            self.tap_contact = TapContact::Armed;
        }
        // A latched rotation Stop phase is not movement: real firmware leaves
        // it in clean tap reports. Only actual deltas or press edges disqualify.
        if event.relative_slot_rotation != 0
            || event.relative_ratchet_rotation != 0
            || pressed == Some(true)
            || self.pressed
        {
            self.tap_contact = TapContact::Consumed;
        }
        if touched == Some(true) {
            emit_edge(&mut self.touched, touched, ButtonId::CrownTouch, sink);
        }
        if pressed == Some(true) {
            emit_edge(&mut self.pressed, pressed, ButtonId::CrownPress, sink);
        }
        if event.relative_slot_rotation != 0
            || event.relative_ratchet_rotation != 0
            || (self.rotating && matches!(event.rotation_state, crate::crown::RotationState::Stop))
        {
            let _ = sink.send(CapturedInput::CrownRotation(crate::crown::CrownMotion {
                slots: event.relative_slot_rotation,
                ratchets: event.relative_ratchet_rotation,
                resolution: self.resolution,
                ratcheted,
                phase: event.rotation_state,
                speed: event.speed,
                pressed: self.pressed,
            }));
        }
        if event.relative_slot_rotation != 0 || event.relative_ratchet_rotation != 0 {
            self.rotating = true;
        }
        if matches!(event.rotation_state, crate::crown::RotationState::Stop) {
            self.rotating = false;
        }
        // Both contact holds include the report's final movement, even when
        // it has not crossed a detent. Release their output only afterwards.
        emit_edge(&mut self.touched, touched, ButtonId::CrownTouch, sink);
        emit_edge(&mut self.pressed, pressed, ButtonId::CrownPress, sink);
        if event.gesture == CrownGesture::Tap && matches!(self.tap_contact, TapContact::Armed) {
            self.tap_contact = TapContact::Pending;
        }
        if matches!(self.tap_contact, TapContact::Pending) && !self.touched {
            self.tap_contact = TapContact::Consumed;
            let _ = sink.send(CapturedInput::CrownTap);
        }
    }
}

fn emit_edge(
    held: &mut bool,
    now: Option<bool>,
    button: ButtonId,
    sink: &mpsc::UnboundedSender<CapturedInput>,
) {
    if let Some(now) = now
        && now != *held
    {
        *held = now;
        let _ = sink.send(if now {
            CapturedInput::ButtonDown(button)
        } else {
            CapturedInput::ButtonUp(button)
        });
    }
}

#[cfg(test)]
mod tests {
    use hidpp::feature::crown::{CrownEvent, event::decode_event};

    use super::*;

    fn update(slots: i8, detents: i8, touch: u8, button: u8, gesture: u8) -> CrownUpdate {
        let mut payload = [0; 16];
        payload[1] = slots.cast_unsigned();
        payload[2] = detents.cast_unsigned();
        payload[4] = touch;
        payload[5] = gesture;
        payload[6] = button;
        match decode_event(0, &payload).expect("valid crown report") {
            CrownEvent::Update(event) => event,
            _ => panic!("unexpected crown event"),
        }
    }

    fn drain(inputs: &mut mpsc::UnboundedReceiver<CapturedInput>) -> Vec<CapturedInput> {
        std::iter::from_fn(|| inputs.try_recv().ok()).collect()
    }

    #[test]
    fn crown_holds_have_balanced_edges_and_unknown_states_do_not_release() {
        let (sink, mut inputs) = mpsc::unbounded_channel();
        let mut crown = CrownAccum::default();
        for (touch, button) in [(1, 1), (2, 2), (255, 255), (2, 3), (2, 4), (3, 5), (0, 0)] {
            crown.on_event(update(0, 0, touch, button, 0), true, &sink);
        }
        assert_eq!(
            drain(&mut inputs),
            vec![
                CapturedInput::ButtonDown(ButtonId::CrownTouch),
                CapturedInput::ButtonDown(ButtonId::CrownPress),
                CapturedInput::ButtonUp(ButtonId::CrownTouch),
                CapturedInput::ButtonUp(ButtonId::CrownPress),
            ]
        );
    }

    fn motion(slots: i8, ratchets: i8, ratcheted: bool, pressed: bool) -> CapturedInput {
        let mut motion = crate::crown::CrownMotion::new(slots, ratchets, ratcheted);
        motion.pressed = pressed;
        CapturedInput::CrownRotation(motion)
    }

    #[test]
    fn crown_rotation_preserves_magnitude_and_separates_pressed_rotation() {
        let (sink, mut inputs) = mpsc::unbounded_channel();
        let mut crown = CrownAccum::default();
        crown.on_event(update(12, 2, 0, 0, 0), true, &sink);
        assert_eq!(drain(&mut inputs), [motion(12, 2, true, false)]);
        crown.on_event(update(-12, -3, 0, 1, 0), true, &sink);
        assert_eq!(
            drain(&mut inputs),
            [
                CapturedInput::ButtonDown(ButtonId::CrownPress),
                motion(-12, -3, true, true),
            ]
        );
        crown.on_event(update(1, 0, 0, 2, 0), true, &sink);
        assert_eq!(drain(&mut inputs), [motion(1, 0, true, true)]);
    }

    #[test]
    fn crown_free_rotation_preserves_magnitude_and_tap_reports_do_not_dispatch() {
        let (sink, mut inputs) = mpsc::unbounded_channel();
        let mut crown = CrownAccum::default();
        crown.on_event(update(i8::MIN, -1, 0, 0, 0), false, &sink);
        assert_eq!(drain(&mut inputs), [motion(i8::MIN, -1, false, false)]);
        for gesture in [1, 2, 255] {
            crown.on_event(update(0, 0, 0, 0, gesture), true, &sink);
        }
        assert!(drain(&mut inputs).is_empty());
    }

    #[test]
    fn final_sub_detent_movement_is_observed_before_press_release() {
        let (sink, mut inputs) = mpsc::unbounded_channel();
        let mut crown = CrownAccum::default();
        crown.on_event(update(0, 0, 1, 1, 0), true, &sink);
        drain(&mut inputs);
        crown.on_event(update(1, 0, 3, 5, 0), true, &sink);
        assert_eq!(
            drain(&mut inputs),
            [
                motion(1, 0, true, true),
                CapturedInput::ButtonUp(ButtonId::CrownTouch),
                CapturedInput::ButtonUp(ButtonId::CrownPress),
            ]
        );
    }

    #[test]
    fn final_rotation_precedes_touch_release_with_or_without_a_press() {
        for pressed in [false, true] {
            for ratcheted in [false, true] {
                for direction in [-1, 1] {
                    let (sink, mut inputs) = mpsc::unbounded_channel();
                    let mut crown = CrownAccum::default();
                    crown.on_event(update(0, 0, 1, u8::from(pressed), 0), ratcheted, &sink);
                    drain(&mut inputs);
                    crown.on_event(update(direction, direction, 3, 5, 0), ratcheted, &sink);
                    let mut expected = vec![
                        motion(direction, direction, ratcheted, pressed),
                        CapturedInput::ButtonUp(ButtonId::CrownTouch),
                    ];
                    if pressed {
                        expected.push(CapturedInput::ButtonUp(ButtonId::CrownPress));
                    }
                    assert_eq!(drain(&mut inputs), expected);
                }
            }
        }
    }

    #[test]
    fn firmware_tap_waits_for_release_and_motion_can_still_cancel_it() {
        let (sink, mut inputs) = mpsc::unbounded_channel();
        let mut crown = CrownAccum::default();
        crown.on_event(update(0, 0, 1, 0, 1), true, &sink);
        assert_eq!(
            drain(&mut inputs),
            [CapturedInput::ButtonDown(ButtonId::CrownTouch)]
        );
        crown.on_event(update(0, 0, 2, 0, 1), true, &sink);
        assert!(drain(&mut inputs).is_empty());
        crown.on_event(update(0, 0, 3, 0, 0), true, &sink);
        assert_eq!(
            drain(&mut inputs),
            [
                CapturedInput::ButtonUp(ButtonId::CrownTouch),
                CapturedInput::CrownTap,
            ]
        );
        crown.on_event(update(0, 0, 0, 0, 1), true, &sink);
        assert!(drain(&mut inputs).is_empty());

        for (slots, button) in [(1, 0), (0, 1)] {
            crown.on_event(update(0, 0, 1, 0, 1), true, &sink);
            crown.on_event(update(slots, 0, 2, button, 0), true, &sink);
            crown.on_event(update(0, 0, 3, 5, 1), true, &sink);
            assert!(!drain(&mut inputs).contains(&CapturedInput::CrownTap));
        }
    }

    #[test]
    fn clean_firmware_tap_is_emitted_once_and_press_or_motion_cannot_select() {
        let (sink, mut inputs) = mpsc::unbounded_channel();
        let mut crown = CrownAccum::default();
        crown.on_event(update(0, 0, 1, 0, 0), true, &sink);
        let mut tap = update(0, 0, 3, 0, 1);
        tap.rotation_state = hidpp::feature::crown::RotationState::Stop;
        crown.on_event(tap, true, &sink);
        crown.on_event(tap, true, &sink);
        assert_eq!(
            drain(&mut inputs),
            [
                CapturedInput::ButtonDown(ButtonId::CrownTouch),
                CapturedInput::ButtonUp(ButtonId::CrownTouch),
                CapturedInput::CrownTap,
            ]
        );
        for (slots, press) in [(1, 0), (0, 1)] {
            crown.on_event(update(0, 0, 1, 0, 0), true, &sink);
            crown.on_event(update(slots, 0, 2, press, 0), true, &sink);
            crown.on_event(tap, true, &sink);
            assert!(!drain(&mut inputs).contains(&CapturedInput::CrownTap));
        }
    }
}
