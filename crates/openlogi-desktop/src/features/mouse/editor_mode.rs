//! What the Buttons workspace's binding editor is doing: which control the
//! inspector edits, and whether the action picker is open on it and recording
//! a shortcut.

use openlogi_core::binding::{ButtonId, GestureDirection};

use super::force_sensing::ForceSensingSlider;
use super::hotspots::MouseControlId;
use super::inspector::InvalidDrafts;
use crate::features::binding_editor::ShortcutDraft;

/// The binding editor's mode. Transitions consume the mode and return the
/// next one, so whatever leaves a mode drops what it held: closing the
/// picker, selecting another control or switching devices drops a
/// [`ShortcutDraft`], and with it any key interception it installed.
#[derive(Default)]
pub(super) enum EditorMode {
    /// No control is selected; the inspector summarizes the profile.
    #[default]
    Idle,
    /// The inspector edits a control, with the action picker closed.
    Selected(Selection),
    /// The action picker is open on the selection.
    PickerOpen {
        selection: Selection,
        drafts: PickerDrafts,
    },
}

impl EditorMode {
    /// Edit `control`, with the picker closed. Selecting the control already
    /// edited changes nothing.
    pub(super) fn select(self, control: MouseControlId) -> Self {
        if self.control() == Some(control) {
            self
        } else {
            Self::Selected(Selection::new(control))
        }
    }

    /// Show `direction` in the selected button's gesture inspector, with the
    /// picker closed.
    pub(super) fn select_gesture_direction(self, direction: GestureDirection) -> Self {
        let mut mode = self.close_picker();
        if let Self::Selected(selection) = &mut mode {
            selection.set_gesture_direction(direction);
        }
        mode
    }

    /// Open the picker on the selection with fresh drafts.
    pub(super) fn open_picker(self, shortcut: ShortcutDraft) -> Self {
        match self {
            Self::Selected(selection) => Self::PickerOpen {
                selection,
                drafts: PickerDrafts {
                    shortcut,
                    invalid: InvalidDrafts::default(),
                },
            },
            mode @ (Self::Idle | Self::PickerOpen { .. }) => mode,
        }
    }

    /// Close the picker, ending any recording.
    pub(super) fn close_picker(self) -> Self {
        match self {
            Self::PickerOpen { selection, .. } => Self::Selected(selection),
            mode @ (Self::Idle | Self::Selected(_)) => mode,
        }
    }

    pub(super) fn selection(&self) -> Option<&Selection> {
        match self {
            Self::Idle => None,
            Self::Selected(selection) | Self::PickerOpen { selection, .. } => Some(selection),
        }
    }

    fn selection_mut(&mut self) -> Option<&mut Selection> {
        match self {
            Self::Idle => None,
            Self::Selected(selection) | Self::PickerOpen { selection, .. } => Some(selection),
        }
    }

    pub(super) fn control(&self) -> Option<MouseControlId> {
        self.selection().map(Selection::control)
    }

    /// The open picker's drafts, or `None` while the picker is closed.
    pub(super) fn drafts(&self) -> Option<&PickerDrafts> {
        match self {
            Self::PickerOpen { drafts, .. } => Some(drafts),
            Self::Idle | Self::Selected(_) => None,
        }
    }

    pub(super) fn drafts_mut(&mut self) -> Option<&mut PickerDrafts> {
        match self {
            Self::PickerOpen { drafts, .. } => Some(drafts),
            Self::Idle | Self::Selected(_) => None,
        }
    }

    #[cfg(test)]
    pub(super) fn is_recording(&self) -> bool {
        self.drafts()
            .is_some_and(|drafts| drafts.shortcut.is_recording())
    }

    /// The press-force slider, while the haptic panel is selected and its
    /// inspector shows one.
    pub(super) fn press_force(&self) -> Option<&ForceSensingSlider> {
        match self.selection()? {
            Selection::HapticPanel { press_force, .. } => press_force.as_ref(),
            Selection::Thumbwheel | Selection::Button { .. } => None,
        }
    }

    /// The haptic panel's slider slot, or `None` while another control or
    /// none is selected.
    pub(super) fn press_force_mut(&mut self) -> Option<&mut Option<ForceSensingSlider>> {
        match self.selection_mut()? {
            Selection::HapticPanel { press_force, .. } => Some(press_force),
            Selection::Thumbwheel | Selection::Button { .. } => None,
        }
    }
}

/// The control the inspector edits, with what only that control keeps.
/// [`EditorMode::select`] builds it from the control, so the haptic panel is
/// always [`Self::HapticPanel`].
pub(super) enum Selection {
    /// The thumb wheel, whose two directions take one preset.
    Thumbwheel,
    /// A button, with the direction its gesture inspector shows while the
    /// button is in gesture mode. Whether it is lives in `AppState`.
    Button {
        button: ButtonId,
        direction: GestureDirection,
    },
    /// The haptic panel: a button that also keeps the press-force slider
    /// while its inspector shows one.
    HapticPanel {
        direction: GestureDirection,
        press_force: Option<ForceSensingSlider>,
    },
}

impl Selection {
    fn new(control: MouseControlId) -> Self {
        match control {
            MouseControlId::ThumbwheelRotation => Self::Thumbwheel,
            MouseControlId::Button(ButtonId::HapticPanel) => Self::HapticPanel {
                direction: GestureDirection::Click,
                press_force: None,
            },
            MouseControlId::Button(button) => Self::Button {
                button,
                direction: GestureDirection::Click,
            },
        }
    }

    pub(super) fn control(&self) -> MouseControlId {
        self.button()
            .map_or(MouseControlId::ThumbwheelRotation, |(button, _)| {
                MouseControlId::Button(button)
            })
    }

    /// The selected button and its gesture direction, or `None` for the
    /// thumb wheel.
    pub(super) fn button(&self) -> Option<(ButtonId, GestureDirection)> {
        match self {
            Self::Thumbwheel => None,
            Self::Button { button, direction } => Some((*button, *direction)),
            Self::HapticPanel { direction, .. } => Some((ButtonId::HapticPanel, *direction)),
        }
    }

    fn set_gesture_direction(&mut self, direction: GestureDirection) {
        match self {
            Self::Thumbwheel => {}
            Self::Button {
                direction: kept, ..
            }
            | Self::HapticPanel {
                direction: kept, ..
            } => {
                *kept = direction;
            }
        }
    }
}

/// The open picker's shared shortcut draft and validation for its other
/// custom-action editors. Closing the picker drops the shortcut draft.
pub(super) struct PickerDrafts {
    pub shortcut: ShortcutDraft,
    pub invalid: InvalidDrafts,
}
