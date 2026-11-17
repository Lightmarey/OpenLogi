//! Resolve captured HID++ inputs against the active per-device plan.

mod crown;
mod wheel;

use std::collections::{HashMap, HashSet};
use std::num::NonZeroU8;
use std::time::Instant;

use openlogi_core::binding::{Action, Binding, ButtonId, HeldWheel, default_binding};
use openlogi_core::config::ThumbwheelSensitivity;
use openlogi_core::scroll::ScrollDelta;
use openlogi_hid::CapturedInput;
use openlogi_hid::thumbwheel::WheelResolution;
use tracing::{debug, warn};

use self::wheel::{ScrollScale, WheelAccumulators, WheelOutput, WheelRotation};
use super::GestureOutputs;
use crate::capture_plan::DispatchPlan;
use crate::runtime::hook::SharedHookMaps;
use crate::runtime::{HidppSessionId, PointerPress, PressToken};

/// Effective thumb-wheel configuration whose continuity is tied to one
/// dispatch plan. A binding or sensitivity update clears accumulated state
/// without cycling an unchanged HID++ diversion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct WheelConfiguration {
    up: Action,
    down: Action,
    sensitivity: ThumbwheelSensitivity,
}

impl WheelConfiguration {
    /// Resolve both directional bindings and their shared sensitivity.
    pub(super) fn for_plan(plan: &DispatchPlan) -> Self {
        let action = |button| {
            plan.bindings
                .get(&button)
                .map_or_else(|| default_binding(button), Binding::click_action)
        };
        Self {
            up: action(ButtonId::ThumbwheelScrollUp),
            down: action(ButtonId::ThumbwheelScrollDown),
            sensitivity: plan.thumbwheel_sensitivity,
        }
    }

    fn action(&self, rotation: WheelRotation) -> &Action {
        match rotation.button() {
            ButtonId::ThumbwheelScrollUp => &self.up,
            ButtonId::ThumbwheelScrollDown => &self.down,
            _ => unreachable!("wheel rotations only map to thumb-wheel directions"),
        }
    }
}

/// Correlates what happens during a held press — a completed HID++ gesture,
/// or wheel movement under a wheel-hold button — with the exact physical
/// press token admitted by the shared button runtime. The runtime remains the
/// sole authority on whether the token is still active.
#[derive(Default)]
struct HeldPresses {
    tokens: HashMap<(HidppSessionId, ButtonId), PressToken>,
}

impl HeldPresses {
    fn start(&mut self, session: &HidppSessionId, button: ButtonId, press: PressToken) {
        self.tokens.insert((session.clone(), button), press);
    }

    fn get(&self, session: &HidppSessionId, button: ButtonId) -> Option<&PressToken> {
        self.tokens.get(&(session.clone(), button))
    }

    fn end(&mut self, session: &HidppSessionId, button: ButtonId) {
        self.tokens.remove(&(session.clone(), button));
    }

    fn cancel_session(&mut self, session: &HidppSessionId) {
        self.tokens.retain(|(candidate, _), _| candidate != session);
    }
}

/// Wheel state scoped to exact capture-session incarnations. Keying by session
/// rather than device prevents a replacement epoch from inheriting progress or
/// having its state removed by a stale completion from the previous epoch.
#[derive(Default)]
struct SessionWheels(HashMap<HidppSessionId, WheelAccumulators>);

impl SessionWheels {
    fn for_session(&mut self, session: &HidppSessionId) -> &mut WheelAccumulators {
        self.0.entry(session.clone()).or_default()
    }

    fn cancel_session(&mut self, session: &HidppSessionId) {
        self.0.remove(session);
    }
}

struct CrownSteps {
    output: HidppSessionId,
    progress: HashMap<ButtonId, CrownStep>,
}

struct CrownStep {
    binding: Binding,
    remainder_percent: u16,
}

impl CrownSteps {
    fn advance(
        &mut self,
        output: &HidppSessionId,
        button: ButtonId,
        binding: &Binding,
        increments: u8,
        sensitivity: openlogi_core::rotary::RotarySensitivity,
    ) -> u16 {
        if self.output != *output {
            self.output = output.clone();
            self.progress.clear();
        }
        let step = self.progress.entry(button).or_insert_with(|| CrownStep {
            binding: binding.clone(),
            remainder_percent: 0,
        });
        if step.binding != *binding {
            step.binding = binding.clone();
            step.remainder_percent = 0;
        }
        // Hundredths keep every configured percentage exact. Splitting whole
        // pulses from the fraction keeps both products within u16.
        let speed = sensitivity.percent();
        let increments = u16::from(increments);
        let fractional = step.remainder_percent + increments * (speed % 100);
        let pulses = increments * (speed / 100) + fractional / 100;
        step.remainder_percent = fractional % 100;
        pulses
    }
}

/// Input routing plus the per-session state retained between
/// captured events. Capture-session lifecycle remains owned by the parent.
pub(super) struct InputDispatcher {
    hook_maps: SharedHookMaps,
    outputs: GestureOutputs,
    wheels: SessionWheels,
    held_presses: HeldPresses,
    shift_held: HeldShift,
    crown: crown::CrownFunctions,
    crown_modes: HashMap<u64, crate::hardware::crown::CrownModeHandle>,
    crown_steps: HashMap<u64, CrownSteps>,
    crown_confirmed: HashMap<u64, openlogi_core::rotary::TactileMode>,
}

impl InputDispatcher {
    /// Build a dispatcher for session-owned capture-plan snapshots.
    pub(super) fn new(outputs: GestureOutputs) -> Self {
        Self {
            hook_maps: outputs.hook_maps.clone(),
            outputs,
            wheels: SessionWheels::default(),
            held_presses: HeldPresses::default(),
            shift_held: HeldShift::default(),
            crown: crown::CrownFunctions::default(),
            crown_modes: HashMap::new(),
            crown_steps: HashMap::new(),
            crown_confirmed: HashMap::new(),
        }
    }

    pub(super) fn register_crown_mode(
        &mut self,
        session: &HidppSessionId,
        mode: crate::hardware::crown::CrownModeHandle,
        plan: &DispatchPlan,
    ) {
        self.crown_modes.insert(session.epoch(), mode);
        self.refresh_crown_mode(session, plan);
    }

    pub(super) fn retire_crown_mode(&mut self, session: &HidppSessionId) {
        self.crown_modes.remove(&session.epoch());
        self.crown.retire(session);
        self.crown_confirmed.remove(&session.epoch());
    }

    pub(super) fn refresh_crown_mode(&self, session: &HidppSessionId, plan: &DispatchPlan) {
        if let Some(handle) = self.crown_modes.get(&session.epoch()) {
            let feel = plan
                .crown
                .as_ref()
                .map_or(openlogi_core::rotary::TactileMode::Inherit, |policy| {
                    self.crown.feel(session, policy, &plan.config_key)
                });
            handle.request(feel);
        }
    }

    /// Whether `session` still holds an admitted gesture press for `button`.
    #[cfg(test)]
    pub(super) fn holds_gesture_press(&self, session: &HidppSessionId, button: ButtonId) -> bool {
        self.held_presses.get(session, button).is_some()
    }

    /// Cancel every input lifecycle retained for one capture session.
    pub(super) fn cancel_session(&mut self, session: &HidppSessionId) {
        self.crown.cancel(session, &self.outputs);
        self.crown_steps.remove(&session.epoch());
        self.outputs.cancel_session(session);
        self.wheels.cancel_session(session);
        self.held_presses.cancel_session(session);
        self.shift_held.0.remove(session);
        self.publish_shift(session);
    }

    /// Tell the OS hook whether `session`'s device has G-Shift held, so its
    /// gesture buttons resolve the shifted layer too.
    fn publish_shift(&self, session: &HidppSessionId) {
        let key = session.device_key();
        if let Ok(mut maps) = self.hook_maps.write() {
            if self.shift_held.active(session) {
                maps.shift_held.insert(key.to_owned());
            } else {
                maps.shift_held.remove(key);
            }
        }
    }

    /// Hold G-Shift if `button` is bound to it; `true` when it was.
    fn start_gshift(
        &mut self,
        session: &HidppSessionId,
        plan: &DispatchPlan,
        button: ButtonId,
    ) -> bool {
        let is_shift = plan.bindings.get(&button).is_some_and(Binding::is_gshift);
        if is_shift {
            debug!(key = session.device_key(), ?button, "G-Shift held");
            self.shift_held.press(session, button);
            self.publish_shift(session);
        }
        is_shift
    }

    /// Release `button` if it is a held G-Shift button; `true` when it was.
    fn end_gshift(&mut self, session: &HidppSessionId, button: ButtonId) -> bool {
        let released = self.shift_held.release(session, button);
        if released {
            self.publish_shift(session);
        }
        released
    }

    /// `button`'s binding, from the G-Shift layer while G-Shift is held, and
    /// the pointer identity that the press carries. A gesture source has no
    /// binding: its click/swipe map owns the press.
    fn binding_for<'p>(
        &self,
        session: &HidppSessionId,
        plan: &'p DispatchPlan,
        button: ButtonId,
        is_gesture: bool,
    ) -> (Option<&'p Binding>, Option<PointerPress>) {
        let shifted = !is_gesture && self.shift_held.active(session);
        let pointer = plan.pointer.as_ref().map(|pointer| {
            if shift_binding(plan, button, shifted).is_some() {
                pointer.unscoped_press()
            } else {
                pointer.press(button)
            }
        });
        let binding = (!is_gesture)
            .then(|| press_binding(plan, button, shifted))
            .flatten();
        (binding, pointer)
    }

    /// Route one captured input from `session` to its bound action or
    /// re-synthesised scroll output.
    #[cfg(test)]
    pub(super) fn dispatch(
        &mut self,
        session: &HidppSessionId,
        plan: &DispatchPlan,
        input: CapturedInput,
    ) {
        self.dispatch_with_modifiers(
            session,
            plan,
            input,
            openlogi_core::config::KeyModifiers::default(),
        );
    }

    fn dispatch_crown_input(
        &mut self,
        session: &HidppSessionId,
        plan: &DispatchPlan,
        input: CapturedInput,
        modifiers: openlogi_core::config::KeyModifiers,
    ) -> bool {
        match input {
            CapturedInput::CrownModeApplied { mode, request } => {
                if self
                    .crown_modes
                    .get(&session.epoch())
                    .is_none_or(|handle| handle.accepts(request))
                {
                    self.crown_confirmed.insert(session.epoch(), mode);
                }
                return true;
            }
            CapturedInput::CrownReady(capabilities) => {
                self.crown.set_capabilities(session, capabilities);
                return true;
            }
            _ => {}
        }
        let handled = self
            .crown
            .dispatch(session, plan, input, modifiers, &self.outputs);
        if let CapturedInput::CrownRotation(motion) = input {
            self.dispatch_crown_motion(session, plan, motion);
            self.refresh_crown_mode(session, plan);
            return true;
        }
        if handled {
            self.refresh_crown_mode(session, plan);
        }
        handled
    }

    pub(super) fn dispatch_with_modifiers(
        &mut self,
        session: &HidppSessionId,
        plan: &DispatchPlan,
        input: CapturedInput,
        modifiers: openlogi_core::config::KeyModifiers,
    ) {
        if self.dispatch_crown_input(session, plan, input, modifiers) {
            return;
        }
        let key = session.device_key();
        match input {
            CapturedInput::Gesture(button, direction) => {
                let Some(press) = self.held_presses.get(session, button) else {
                    debug!(key, %button, ?direction, "gesture from a canceled button lifecycle — ignored");
                    return;
                };
                if let Some(action) = plan
                    .gesture_bindings
                    .get(&button)
                    .or_else(|| plan.side_gesture_bindings.get(&button))
                    .and_then(|map| map.get(&direction))
                {
                    debug!(key, %button, ?direction, action = %action.label(), "gesture → action");
                    if !self
                        .outputs
                        .actions
                        .try_dispatch_while_pressed(press, action)
                    {
                        debug!(key, %button, ?direction, "gesture press no longer active — ignored");
                    }
                } else {
                    debug!(key, %button, ?direction, "gesture with no binding — ignored");
                }
            }
            CapturedInput::ButtonDown(button) => {
                if self.start_gshift(session, plan, button) {
                    return;
                }
                // A raw-XY gesture source owns its click/swipe map; its physical
                // lifecycle is still tracked, but it must not also fire the
                // single-action projection on down.
                let is_gesture = plan.gesture_bindings.contains_key(&button)
                    || plan.side_gesture_bindings.contains_key(&button);
                let (binding, pointer) = self.binding_for(session, plan, button, is_gesture);
                let tracks_hold = is_gesture || binding.is_some_and(Binding::is_wheel_hold);
                if let Some(binding) = binding {
                    debug!(key, ?button, action = %binding.click_action().label(), "HID++ button → binding");
                } else {
                    debug!(key, ?button, "HID++ button with no binding — ignored");
                }
                let press = self
                    .outputs
                    .actions
                    .try_hidpp_button_down(session, button, binding, pointer);
                if tracks_hold {
                    if let Some(press) = press {
                        self.held_presses.start(session, button, press);
                    } else {
                        self.held_presses.end(session, button);
                    }
                }
            }
            CapturedInput::ButtonUp(button) => {
                if self.end_gshift(session, button) {
                    return;
                }
                self.outputs.actions.try_hidpp_button_up(session, button);
                self.held_presses.end(session, button);
            }
            CapturedInput::ButtonPulse(button) => self.dispatch_button_pulse(session, plan, button),
            CapturedInput::Scroll {
                increments,
                resolution,
            } => self.dispatch_wheel(session, plan, increments, resolution),
            CapturedInput::HeldWheel {
                button,
                delta,
                per_notch,
            } => self.dispatch_held_wheel(session, plan, button, delta, per_notch),
            CapturedInput::MainWheel { delta, per_notch } => self.outputs.post_main_wheel_scroll(
                session,
                main_wheel_scroll(
                    delta,
                    per_notch,
                    plan.invert_wheel,
                    openlogi_inject::natural_scrolling(),
                ),
                plan.momentum.resolve(),
            ),
            CapturedInput::Interrupted => {
                self.cancel_session(session);
                self.crown_confirmed.remove(&session.epoch());
                if let Some(handle) = self.crown_modes.get(&session.epoch()) {
                    handle.reconfirm();
                }
            }
            CapturedInput::HostModeEntered => self.outputs.actions.host_mode_entered(key),
            CapturedInput::ButtonDiverted(_) => {
                unreachable!("the session's forwarder records diverted buttons instead")
            }
            CapturedInput::CrownTap
            | CapturedInput::CrownRotation(_)
            | CapturedInput::CrownReady(_)
            | CapturedInput::CrownModeApplied { .. } => {}
        }
    }

    fn dispatch_crown_motion(
        &mut self,
        parent: &HidppSessionId,
        plan: &DispatchPlan,
        motion: openlogi_hid::crown::CrownMotion,
    ) {
        use openlogi_core::crown::CrownControl;
        use openlogi_core::rotary::{RotarySettings, TactileMode, scroll_for_action};
        let function = self.crown.rotation_function(parent, plan.crown.as_ref());
        let (output, settings, pressed) = function.as_ref().map_or_else(
            || {
                (
                    parent.clone(),
                    plan.crown
                        .as_ref()
                        .map_or_else(RotarySettings::default, |policy| policy.profile.rotation()),
                    motion.pressed,
                )
            },
            |(output, function, pressed)| (output.clone(), function.rotation(), *pressed),
        );
        let ratcheted = self
            .crown_confirmed
            .get(&parent.epoch())
            .map_or(motion.ratcheted, |mode| *mode == TactileMode::Ratchet);
        let native = if motion.slots == 0 {
            f64::from(motion.ratchets)
        } else {
            f64::from(motion.slots) * motion.resolution.native_per_increment()
        };
        let distance = native * settings.sensitivity.multiplier();
        if self.crown.dispatch_application_motion(
            parent,
            plan.crown.as_ref(),
            motion,
            if settings.inverted {
                -distance
            } else {
                distance
            },
            ratcheted,
            &self.outputs,
        ) {
            return;
        }
        let steps = if ratcheted {
            motion.ratchets
        } else {
            motion.slots
        };
        let direction = if motion.slots == 0 {
            motion.ratchets
        } else {
            motion.slots
        };
        if direction == 0 {
            return;
        }
        let positive = (direction > 0) != settings.inverted;
        let control = match (pressed, positive) {
            (false, false) => CrownControl::Left,
            (false, true) => CrownControl::Right,
            (true, false) => CrownControl::PressedLeft,
            (true, true) => CrownControl::PressedRight,
        };
        let binding = function.as_ref().map_or_else(
            || {
                press_binding(plan, control.button(), self.shift_held.active(parent))
                    .cloned()
                    .unwrap_or_else(|| Action::None.into())
            },
            |(_, function, _)| function.binding(control),
        );
        let action = binding.click_action();
        let scroll = scroll_for_action(&action, distance.abs());
        let accumulated = self
            .crown_steps
            .entry(parent.epoch())
            .or_insert_with(|| CrownSteps {
                output: output.clone(),
                progress: HashMap::new(),
            });
        let pulses = accumulated.advance(
            &output,
            control.button(),
            &binding,
            if scroll.is_some() {
                0
            } else {
                steps.unsigned_abs()
            },
            settings.sensitivity,
        );
        if let Some(delta) = scroll {
            self.outputs.post_crown_scroll(&output, delta);
            return;
        }
        // An i8 report and validated speed bound this work to 1280 actions.
        for _ in 0..pulses {
            self.outputs.actions.dispatch_hidpp_button_pulse(
                &output,
                control.button(),
                Some(&binding),
                None,
            );
        }
    }

    /// A firmware-reported tap with no hold, such as the thumb-wheel touch
    /// sensor: fire `button`'s binding once.
    fn dispatch_button_pulse(
        &self,
        session: &HidppSessionId,
        plan: &DispatchPlan,
        button: ButtonId,
    ) {
        let key = session.device_key();
        let binding = plan.bindings.get(&button);
        if let Some(binding) = binding {
            debug!(key, ?button, action = %binding.click_action().label(), "HID++ button pulse → binding");
        } else {
            debug!(key, ?button, "HID++ button pulse with no binding — ignored");
        }
        self.outputs.actions.dispatch_hidpp_button_pulse(
            session,
            button,
            binding,
            plan.pointer.as_ref().map(|pointer| pointer.press(button)),
        );
    }

    /// Main-wheel movement under a held wheel-hold button: scroll in its
    /// place. The press learns of the movement first, so its click is
    /// suppressed even when the scroll cannot be posted.
    fn dispatch_held_wheel(
        &mut self,
        session: &HidppSessionId,
        plan: &DispatchPlan,
        button: ButtonId,
        delta: i16,
        per_notch: NonZeroU8,
    ) {
        let key = session.device_key();
        let Some(press) = self.held_presses.get(session, button) else {
            debug!(key, %button, delta, "held wheel from a canceled button lifecycle — ignored");
            return;
        };
        if !self.outputs.actions.try_wheel_while_pressed(press) {
            debug!(key, %button, delta, "held wheel after the press ended — ignored");
            return;
        }
        let Some(Binding::WheelHold(binding)) = plan.bindings.get(&button) else {
            debug!(key, %button, "held wheel without a wheel-hold binding — ignored");
            return;
        };
        match binding.wheel() {
            HeldWheel::HorizontalScroll => self.outputs.post_scroll(
                session,
                held_wheel_scroll(delta, per_notch, plan.invert_wheel),
            ),
        }
    }

    fn dispatch_wheel(
        &mut self,
        session: &HidppSessionId,
        plan: &DispatchPlan,
        increments: i16,
        resolution: WheelResolution,
    ) {
        let Some(rotation) = WheelRotation::from_increments(increments) else {
            return;
        };
        let key = session.device_key();
        let button = rotation.button();
        let configuration = WheelConfiguration::for_plan(plan);
        let action = configuration.action(rotation);
        let wheels = self.wheels.for_session(session);
        match wheels.advance(
            rotation,
            action,
            ScrollScale::new(resolution, configuration.sensitivity),
            Instant::now(),
        ) {
            WheelOutput::Idle => {}
            WheelOutput::Scroll(delta) => {
                debug!(key, ?button, action = %action.label(), ?delta, "thumb wheel → scroll");
                self.outputs.post_scroll(session, delta);
            }
            WheelOutput::FireAction(repeats) => {
                debug!(key, ?button, action = %action.label(), repeats, "thumb wheel → action");
                for _ in 0..repeats {
                    self.outputs.actions.dispatch_pointer_action(
                        action,
                        Some(key),
                        plan.pointer.as_ref().map(|pointer| pointer.press(button)),
                    );
                }
            }
        }
    }
}

/// The horizontal scroll one held-wheel report stands for: away from the
/// user scrolls left and toward the user right, matching the thumb wheel's
/// default bindings, flipped when the device's wheel scrolls inverted (Logi
/// Options+ flips it for natural scrolling the same way).
fn held_wheel_scroll(delta: i16, per_notch: NonZeroU8, inverted: bool) -> ScrollDelta {
    let notches = f64::from(delta) / f64::from(per_notch.get());
    let rightward = if inverted { notches } else { -notches };
    ScrollDelta::wheel_ticks(rightward, 0.0)
}

/// The scroll a diverted main wheel's movement stands for: `delta` units
/// away from the user scroll up by `delta / per_notch` notches, as the wheel
/// does natively. It is reversed for the device's inversion, which the
/// wheel's native invert bit no longer applies while diverted, and for the
/// OS's natural scrolling, which macOS applies to the wheel's own events but
/// not to synthesised ones; both on cancel out.
fn main_wheel_scroll(
    delta: i16,
    per_notch: NonZeroU8,
    inverted: bool,
    natural: bool,
) -> ScrollDelta {
    let notches = f64::from(delta) / f64::from(per_notch.get());
    let upward = if inverted == natural {
        notches
    } else {
        -notches
    };
    ScrollDelta::wheel_ticks(0.0, upward)
}

/// G-Shift buttons held per session; the layer stays on until all are released.
#[derive(Default)]
struct HeldShift(HashMap<HidppSessionId, HashSet<ButtonId>>);

impl HeldShift {
    fn press(&mut self, session: &HidppSessionId, button: ButtonId) {
        self.0.entry(session.clone()).or_default().insert(button);
    }

    /// Whether `button` was a held G-Shift button.
    fn release(&mut self, session: &HidppSessionId, button: ButtonId) -> bool {
        self.0
            .get_mut(session)
            .is_some_and(|held| held.remove(&button))
    }

    fn active(&self, session: &HidppSessionId) -> bool {
        self.0.get(session).is_some_and(|held| !held.is_empty())
    }
}

/// The held G-Shift layer's binding for `button`.
fn shift_binding(plan: &DispatchPlan, button: ButtonId, shifted: bool) -> Option<&Binding> {
    shifted.then(|| plan.shift_bindings.get(&button)).flatten()
}

fn press_binding(plan: &DispatchPlan, button: ButtonId, shifted: bool) -> Option<&Binding> {
    shift_binding(plan, button, shifted).or_else(|| plan.bindings.get(&button))
}

#[cfg(test)]
mod tests;

/// Read modifier state only for Crown selection edges, preserving input order.
pub(super) async fn selection_modifiers(
    input: CapturedInput,
) -> openlogi_core::config::KeyModifiers {
    if matches!(
        input,
        CapturedInput::ButtonDown(ButtonId::CrownPress) | CapturedInput::CrownTap
    ) {
        match tokio::task::spawn_blocking(openlogi_hook::held_modifiers).await {
            Ok(Some(modifiers)) => modifiers,
            Ok(None) => {
                warn!("desktop modifiers unavailable for Crown backward selection");
                openlogi_core::config::KeyModifiers::default()
            }
            Err(error) => {
                warn!(%error, "could not read modifiers for Crown selection");
                openlogi_core::config::KeyModifiers::default()
            }
        }
    } else {
        openlogi_core::config::KeyModifiers::default()
    }
}
