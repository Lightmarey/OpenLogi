//! Function selection and action lifecycles for one captured Crown.

use std::collections::HashMap;

use openlogi_core::binding::ButtonId;
use openlogi_core::config::KeyModifiers;
use openlogi_core::crown::{CrownControl, CrownFunctionId, CrownSelectionTrigger};
use openlogi_hid::CapturedInput;
use openlogi_ipc::CrownHud;

use crate::capture_plan::{CrownDispatch, DispatchPlan};
use crate::crown::{CONTACT_LIFETIME, RELEASE_LIFETIME};
use crate::runtime::HidppSessionId;

use super::super::GestureOutputs;

#[derive(Clone, Copy)]
enum SelectionDirection {
    Forward,
    Backward,
}

impl SelectionDirection {
    fn from_modifiers(modifiers: KeyModifiers) -> Self {
        if modifiers.shift {
            Self::Backward
        } else {
            Self::Forward
        }
    }
}

struct Session {
    /// Separate output epoch: switching functions cancels only Crown output,
    /// not another control held by the same keyboard capture session.
    output: HidppSessionId,
    selected: CrownFunctionId,
    touched: bool,
    pressed: bool,
    selection_press: Option<SelectionDirection>,
}

impl Session {
    fn function<'a>(
        &self,
        policy: &'a CrownDispatch,
    ) -> Option<&'a openlogi_core::crown::CrownFunction> {
        policy
            .profile
            .functions()
            .iter()
            .find(|function| function.id() == &self.selected)
    }
}

type ProfileKey = (String, Option<String>);

#[derive(Default)]
pub(super) struct CrownFunctions {
    sessions: HashMap<HidppSessionId, Session>,
    remembered: HashMap<ProfileKey, CrownFunctionId>,
}

impl CrownFunctions {
    pub(super) fn cancel(&mut self, parent: &HidppSessionId, outputs: &GestureOutputs) {
        if let Some(session) = self.sessions.remove(parent) {
            outputs.cancel_session(&session.output);
        }
        outputs.crown.cancel(parent);
    }

    pub(super) fn rotation_function<'a>(
        &self,
        parent: &HidppSessionId,
        policy: Option<&'a CrownDispatch>,
    ) -> Option<(
        HidppSessionId,
        &'a openlogi_core::crown::CrownFunction,
        bool,
    )> {
        let session = self.sessions.get(parent)?;
        Some((
            session.output.clone(),
            session.function(policy?)?,
            session.pressed,
        ))
    }

    pub(super) fn feel(
        &self,
        parent: &HidppSessionId,
        policy: &CrownDispatch,
        config_key: &str,
    ) -> openlogi_core::rotary::TactileMode {
        let inherited = policy.profile.feel();
        if !policy.profile.is_enabled() {
            return inherited;
        }
        let selected = self
            .sessions
            .get(parent)
            .map(|session| &session.selected)
            .or_else(|| {
                self.remembered
                    .get(&(config_key.to_owned(), policy.scope.clone()))
            });
        policy
            .profile
            .functions()
            .iter()
            .find(|function| Some(function.id()) == selected)
            .or_else(|| policy.profile.functions().first())
            .map_or(inherited, |function| match function.feel() {
                openlogi_core::rotary::TactileMode::Inherit => inherited,
                mode => mode,
            })
    }

    /// Consume only Crown input in function mode. Direct mode is dispatched
    /// by the parent using its original bindings.
    pub(super) fn dispatch(
        &mut self,
        parent: &HidppSessionId,
        plan: &DispatchPlan,
        input: CapturedInput,
        modifiers: KeyModifiers,
        outputs: &GestureOutputs,
    ) -> bool {
        let Some(policy) = &plan.crown else {
            return false;
        };
        if !policy.profile.is_enabled() || !is_crown_input(input) {
            return false;
        }
        let profile_key = (plan.config_key.clone(), policy.scope.clone());
        let Some(first) = policy.profile.functions().first() else {
            return true;
        };
        let selected = self
            .remembered
            .get(&profile_key)
            .filter(|id| {
                policy
                    .profile
                    .functions()
                    .iter()
                    .any(|function| function.id() == *id)
            })
            .cloned()
            .unwrap_or_else(|| first.id().clone());
        let (touched, pressed) = match input {
            CapturedInput::CrownRotation(motion) => (motion.touched, motion.pressed),
            _ => (false, false),
        };
        let session = self
            .sessions
            .entry(parent.clone())
            .or_insert_with(|| Session {
                output: HidppSessionId::new(parent.device_key()),
                selected,
                touched,
                pressed,
                selection_press: None,
            });
        handle_input(session, parent, policy, input, modifiers, outputs, plan);
        self.remembered
            .insert(profile_key, session.selected.clone());
        true
    }
}

fn is_crown_input(input: CapturedInput) -> bool {
    match input {
        CapturedInput::CrownTap | CapturedInput::CrownRotation(_) => true,
        CapturedInput::ButtonDown(button) | CapturedInput::ButtonUp(button) => {
            openlogi_core::crown::has_press_edges(button)
        }
        _ => false,
    }
}

fn handle_input(
    session: &mut Session,
    parent: &HidppSessionId,
    policy: &CrownDispatch,
    input: CapturedInput,
    modifiers: KeyModifiers,
    outputs: &GestureOutputs,
    plan: &DispatchPlan,
) {
    match input {
        CapturedInput::ButtonDown(ButtonId::CrownTouch) => session.touched = true,
        CapturedInput::ButtonUp(ButtonId::CrownTouch) => session.touched = false,
        CapturedInput::CrownRotation(motion) => {
            if session.pressed && (motion.slots != 0 || motion.ratchets != 0) {
                session.selection_press = None;
            }
        }
        CapturedInput::CrownTap => {
            if policy.profile.selection_trigger() != CrownSelectionTrigger::TouchTap
                || session.pressed
            {
                return;
            }
            select_function(
                session,
                parent,
                policy,
                outputs,
                SelectionDirection::from_modifiers(modifiers),
            );
        }
        CapturedInput::ButtonDown(ButtonId::CrownPress)
        | CapturedInput::ButtonUp(ButtonId::CrownPress) => {
            handle_press(session, parent, policy, input, modifiers, outputs, plan);
        }
        _ => return,
    }
    let lifetime = if session.touched || session.pressed {
        CONTACT_LIFETIME
    } else {
        RELEASE_LIFETIME
    };
    outputs.crown.show(
        parent,
        CrownHud::new(&policy.profile, session.selected.clone()),
        lifetime,
    );
}

fn select_function(
    session: &mut Session,
    parent: &HidppSessionId,
    policy: &CrownDispatch,
    outputs: &GestureOutputs,
    direction: SelectionDirection,
) {
    let functions = policy.profile.functions();
    let index = functions
        .iter()
        .position(|function| function.id() == &session.selected)
        .unwrap_or(0);
    let next = match direction {
        SelectionDirection::Forward => (index + 1) % functions.len(),
        SelectionDirection::Backward => (index + functions.len() - 1) % functions.len(),
    };
    if let Some(next) = functions.get(next) {
        outputs.cancel_session(&session.output);
        session.output = HidppSessionId::new(parent.device_key());
        session.selected = next.id().clone();
    }
}

fn handle_press(
    session: &mut Session,
    parent: &HidppSessionId,
    policy: &CrownDispatch,
    input: CapturedInput,
    modifiers: KeyModifiers,
    outputs: &GestureOutputs,
    plan: &DispatchPlan,
) {
    let pressed = matches!(input, CapturedInput::ButtonDown(_));
    let reserved = policy.profile.selection_trigger() == CrownSelectionTrigger::Press;
    if reserved && pressed && !session.pressed {
        session.selection_press = Some(SelectionDirection::from_modifiers(modifiers));
    }
    session.pressed = pressed;
    if reserved {
        if !pressed && let Some(direction) = session.selection_press.take() {
            select_function(session, parent, policy, outputs, direction);
        }
        return;
    }
    let Some(function) = session.function(policy) else {
        return;
    };
    if pressed {
        let button = ButtonId::CrownPress;
        outputs.actions.try_hidpp_button_down(
            &session.output,
            button,
            Some(&function.binding(CrownControl::Press)),
            plan.pointer.as_ref().map(|pointer| pointer.press(button)),
        );
    } else {
        outputs
            .actions
            .try_hidpp_button_up(&session.output, ButtonId::CrownPress);
    }
}

#[cfg(test)]
mod tests;
