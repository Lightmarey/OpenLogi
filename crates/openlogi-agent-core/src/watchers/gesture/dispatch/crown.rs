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

#[derive(Default)]
enum SelectionPress {
    #[default]
    Idle,
    Pending(SelectionDirection),
    Consumed,
}

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
    selection_press: SelectionPress,
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
            outputs.actions.cancel_hidpp_session(&session.output);
        }
        outputs.crown.cancel(parent);
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
        if !is_crown_input(input) {
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
        let session = self
            .sessions
            .entry(parent.clone())
            .or_insert_with(|| Session {
                output: HidppSessionId::new(parent.device_key()),
                selected,
                touched: false,
                pressed: false,
                selection_press: SelectionPress::default(),
            });
        handle_input(session, parent, policy, input, modifiers, outputs, plan);
        self.remembered
            .insert(profile_key, session.selected.clone());
        true
    }
}

fn is_crown_input(input: CapturedInput) -> bool {
    match input {
        CapturedInput::CrownTap | CapturedInput::CrownRotation { .. } => true,
        CapturedInput::ButtonDown(button)
        | CapturedInput::ButtonUp(button)
        | CapturedInput::ButtonPulse(button) => {
            button == ButtonId::CrownTouch || CrownControl::for_button(button).is_some()
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
        CapturedInput::CrownRotation { .. } => {
            if session.pressed {
                session.selection_press = SelectionPress::Consumed;
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
        CapturedInput::ButtonDown(button)
        | CapturedInput::ButtonUp(button)
        | CapturedInput::ButtonPulse(button) => {
            let Some(control) = CrownControl::for_button(button) else {
                return;
            };
            if matches!(input, CapturedInput::ButtonDown(ButtonId::CrownPress)) && !session.pressed
            {
                session.selection_press =
                    SelectionPress::Pending(SelectionDirection::from_modifiers(modifiers));
            }
            dispatch_control(session, parent, policy, control, input, outputs, plan);
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
        outputs.actions.cancel_hidpp_session(&session.output);
        session.output = HidppSessionId::new(parent.device_key());
        session.selected = next.id().clone();
    }
}

fn dispatch_control(
    session: &mut Session,
    parent: &HidppSessionId,
    policy: &CrownDispatch,
    control: CrownControl,
    input: CapturedInput,
    outputs: &GestureOutputs,
    plan: &DispatchPlan,
) {
    if control == CrownControl::Press
        && policy.profile.selection_trigger() == CrownSelectionTrigger::Press
    {
        match input {
            CapturedInput::ButtonDown(_) => {
                session.pressed = true;
            }
            CapturedInput::ButtonUp(_) => {
                session.pressed = false;
                if let SelectionPress::Pending(direction) =
                    std::mem::take(&mut session.selection_press)
                {
                    select_function(session, parent, policy, outputs, direction);
                }
            }
            _ => {}
        }
        return;
    }
    let Some(function) = policy
        .profile
        .functions()
        .iter()
        .find(|function| function.id() == &session.selected)
    else {
        return;
    };
    let binding = function.binding(control);
    let button = control.button();
    match input {
        CapturedInput::ButtonDown(_) => {
            if control == CrownControl::Press {
                session.pressed = true;
            }
            outputs.actions.try_hidpp_button_down(
                &session.output,
                button,
                Some(&binding),
                plan.pointer.as_ref().map(|pointer| pointer.press(button)),
            );
        }
        CapturedInput::ButtonUp(_) => {
            if control == CrownControl::Press {
                session.pressed = false;
            }
            outputs.actions.try_hidpp_button_up(&session.output, button);
        }
        CapturedInput::ButtonPulse(_) => {
            outputs.actions.dispatch_hidpp_button_pulse(
                &session.output,
                button,
                Some(&binding),
                plan.pointer.as_ref().map(|pointer| pointer.press(button)),
            );
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests;
