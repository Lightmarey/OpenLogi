//! Validated Craft Crown functions, independent of capture and presentation.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::app::profile_key_for;
use crate::application_control::ProviderId;
use crate::binding::{Action, ActionRingIcon, Binding, ButtonId};
use crate::parameter::ParameterState;
use crate::rotary::{RotarySettings, TactileMode};

/// A read-only feedback source understood by a native Crown presenter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CrownFeedback {
    /// Current system output volume, read by the native presenter.
    SystemVolume,
    /// Built-in display backlight, measured by the agent.
    SystemBrightness,
    /// Agent- or application-published, validated parameter feedback.
    Parameter(ParameterState),
}

/// Maximum number of functions offered in one Crown profile.
pub const MAX_FUNCTIONS: usize = crate::application_control::MAX_FUNCTIONS;

/// Physical input reserved for selecting the next function.
///
/// A push uses button edges, while a touch tap uses the firmware gesture report.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrownSelectionTrigger {
    /// Press and release without rotating. The press binding is reserved.
    #[default]
    Press,
    /// Firmware touch tap. Press bindings remain available for actions.
    TouchTap,
}

impl CrownSelectionTrigger {
    /// Choices exposed by the function-mode inspector.
    pub const ALL: [Self; 2] = [Self::Press, Self::TouchTap];

    /// Shared catalog key for this physical input.
    #[must_use]
    pub const fn translation_key(self) -> &'static str {
        match self {
            Self::Press => "keyboard.crown_switch_press",
            Self::TouchTap => "keyboard.crown_switch_touch_tap",
        }
    }
}

/// Invalid persisted Crown configuration.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum CrownError {
    /// IDs must be nonempty ASCII identifiers of at most 64 bytes.
    #[error("Crown function ID must contain 1–64 ASCII letters, digits, '-' or '_'")]
    InvalidId,
    /// Names must be nonempty and fit the HUD.
    #[error("Crown function name must contain 1–64 characters")]
    InvalidName,
    /// A profile must contain a bounded, nonempty function list.
    #[error("Crown profile must contain 1–8 functions")]
    FunctionCount,
    /// Function IDs are stable and unique within a profile.
    #[error("Crown function IDs must be unique within a profile")]
    DuplicateId,
    /// Rotation has pulses, not hold or gesture edges.
    #[error("Crown rotation requires a single action that does not hold a key")]
    PulseBinding,
    /// Press has edges, but no raw XY gesture reports.
    #[error("Crown press cannot use a gesture or a held short-press action")]
    PressBinding,
}

/// Stable identity retained when a function is renamed or reordered.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CrownFunctionId(String);

impl CrownFunctionId {
    /// Validate an opaque function identifier.
    pub fn new(id: String) -> Result<Self, CrownError> {
        validate_id(&id)?;
        Ok(Self(id))
    }
}

impl AsRef<str> for CrownFunctionId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CrownFunctionId {
    type Error = CrownError;
    fn try_from(id: String) -> Result<Self, Self::Error> {
        Self::new(id)
    }
}

impl From<CrownFunctionId> for String {
    fn from(id: CrownFunctionId) -> Self {
        id.0
    }
}

fn validate_id(id: &str) -> Result<(), CrownError> {
    crate::application_control::validate_identifier(id).map_err(|_| CrownError::InvalidId)
}

/// Controls whose meaning changes with the selected Crown function.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CrownControl {
    /// Counterclockwise rotation.
    Left,
    /// Clockwise rotation.
    Right,
    /// Press and release.
    Press,
    /// Counterclockwise rotation while pressed.
    PressedLeft,
    /// Clockwise rotation while pressed.
    PressedRight,
}

impl CrownControl {
    /// Inspector order, shared by the editor and default bindings.
    pub const ALL: [Self; 5] = [
        Self::Left,
        Self::Right,
        Self::Press,
        Self::PressedLeft,
        Self::PressedRight,
    ];

    /// Physical input corresponding to this control.
    #[must_use]
    pub const fn button(self) -> ButtonId {
        match self {
            Self::Left => ButtonId::CrownLeft,
            Self::Right => ButtonId::CrownRight,
            Self::Press => ButtonId::CrownPress,
            Self::PressedLeft => ButtonId::CrownPressedLeft,
            Self::PressedRight => ButtonId::CrownPressedRight,
        }
    }

    /// Interpret a physical input as a function control.
    #[must_use]
    pub fn for_button(button: ButtonId) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|control| control.button() == button)
    }
}

/// Whether an input has real down/up edges and supports threshold or held output.
#[must_use]
pub const fn has_press_edges(button: ButtonId) -> bool {
    matches!(button, ButtonId::CrownTouch | ButtonId::CrownPress)
}

/// Whether the action being edited can retain output until a release edge.
/// A threshold binding's short action fires only after release.
#[must_use]
pub const fn is_held_output_allowed(button: ButtonId, short_press: bool) -> bool {
    has_press_edges(button) && !short_press
}

/// Validate the lifecycle of a direct Crown input or function control.
/// Touch and press have balanced edges. Rotation produces isolated pulses.
pub fn validate_input_binding(button: ButtonId, binding: &Binding) -> Result<(), CrownError> {
    match binding {
        Binding::Single(action) if !action.is_held() || is_held_output_allowed(button, false) => {
            Ok(())
        }
        Binding::LongPress(pair)
            if has_press_edges(button)
                && (!pair.short().is_held() || is_held_output_allowed(button, true)) =>
        {
            Ok(())
        }
        _ if has_press_edges(button) => Err(CrownError::PressBinding),
        _ => Err(CrownError::PulseBinding),
    }
}

/// One named function with its icon and validated directional bindings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawFunction")]
pub struct CrownFunction {
    id: CrownFunctionId,
    name: String,
    icon: ActionRingIcon,
    bindings: BTreeMap<CrownControl, Binding>,
    feel: TactileMode,
    rotation: RotarySettings,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFunction {
    id: CrownFunctionId,
    name: String,
    icon: ActionRingIcon,
    #[serde(default)]
    bindings: BTreeMap<CrownControl, Binding>,
    #[serde(default)]
    feel: TactileMode,
    #[serde(default)]
    rotation: RotarySettings,
}

impl TryFrom<RawFunction> for CrownFunction {
    type Error = CrownError;

    fn try_from(raw: RawFunction) -> Result<Self, Self::Error> {
        let mut function = Self::new(raw.id, &raw.name, raw.icon)?;
        function.feel = raw.feel;
        function.rotation = raw.rotation;
        for (control, binding) in raw.bindings {
            function.set_binding(control, binding)?;
        }
        Ok(function)
    }
}

impl CrownFunction {
    /// Create a function whose controls initially do nothing.
    pub fn new(id: CrownFunctionId, name: &str, icon: ActionRingIcon) -> Result<Self, CrownError> {
        let mut function = Self {
            id,
            name: String::new(),
            icon,
            bindings: BTreeMap::new(),
            feel: TactileMode::default(),
            rotation: RotarySettings::default(),
        };
        function.rename(name)?;
        Ok(function)
    }

    /// Stable identity, unrelated to list position or display name.
    #[must_use]
    pub const fn id(&self) -> &CrownFunctionId {
        &self.id
    }

    /// User-visible literal name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Presentation icon.
    #[must_use]
    pub const fn icon(&self) -> ActionRingIcon {
        self.icon
    }

    /// Rename without changing the remembered identity.
    pub fn rename(&mut self, name: &str) -> Result<(), CrownError> {
        self.name = crate::application_control::ControlLabel::new(name)
            .map_err(|_| CrownError::InvalidName)?
            .into();
        Ok(())
    }

    /// Change the function's presentation icon.
    pub const fn set_icon(&mut self, icon: ActionRingIcon) {
        self.icon = icon;
    }

    /// Requested tactile mode; inheritance resolves through the active profile.
    #[must_use]
    pub const fn feel(&self) -> TactileMode {
        self.feel
    }

    /// Set the function's tactile preference.
    pub const fn set_feel(&mut self, feel: TactileMode) {
        self.feel = feel;
    }

    /// Calibrated rotation settings for this function.
    #[must_use]
    pub const fn rotation(&self) -> RotarySettings {
        self.rotation
    }

    /// Change calibrated rotation settings.
    pub const fn set_rotation(&mut self, settings: RotarySettings) {
        self.rotation = settings;
    }

    /// Resolve a control; absent entries explicitly do nothing.
    #[must_use]
    pub fn binding(&self, control: CrownControl) -> Binding {
        self.bindings
            .get(&control)
            .cloned()
            .unwrap_or(Binding::Single(Action::None))
    }

    /// Resolve feedback from the actual rotation actions, independently of
    /// editable names, identifiers and icons. Remapped functions never claim
    /// to represent a system parameter their actions no longer change.
    #[must_use]
    pub fn feedback(&self) -> Option<CrownFeedback> {
        match (
            self.bindings.get(&CrownControl::Left),
            self.bindings.get(&CrownControl::Right),
        ) {
            (
                Some(Binding::Single(Action::VolumeDown)),
                Some(Binding::Single(Action::VolumeUp)),
            )
            | (
                Some(Binding::Single(Action::VolumeUp)),
                Some(Binding::Single(Action::VolumeDown)),
            ) => Some(CrownFeedback::SystemVolume),
            (
                Some(Binding::Single(Action::BrightnessDown)),
                Some(Binding::Single(Action::BrightnessUp)),
            )
            | (
                Some(Binding::Single(Action::BrightnessUp)),
                Some(Binding::Single(Action::BrightnessDown)),
            ) => Some(CrownFeedback::SystemBrightness),
            _ => None,
        }
    }

    /// Set a binding after validating its physical lifecycle.
    pub fn set_binding(
        &mut self,
        control: CrownControl,
        binding: Binding,
    ) -> Result<(), CrownError> {
        validate_input_binding(control.button(), &binding)?;
        self.bindings.insert(control, binding);
        Ok(())
    }
}

/// Ordered functions retained even while their mode is disabled.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawProfile")]
pub struct CrownProfile {
    enabled: bool,
    functions: Vec<CrownFunction>,
    selection_trigger: CrownSelectionTrigger,
    feel: TactileMode,
    rotation: RotarySettings,
    provider: Option<ProviderId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawProfile {
    #[serde(default)]
    enabled: bool,
    functions: Vec<CrownFunction>,
    #[serde(default)]
    selection_trigger: CrownSelectionTrigger,
    #[serde(default)]
    feel: TactileMode,
    #[serde(default)]
    rotation: RotarySettings,
    #[serde(default)]
    provider: Option<ProviderId>,
}

impl TryFrom<RawProfile> for CrownProfile {
    type Error = CrownError;

    fn try_from(raw: RawProfile) -> Result<Self, Self::Error> {
        let mut profile = Self::new(raw.enabled, raw.functions)?;
        profile.set_selection_trigger(raw.selection_trigger);
        profile.feel = raw.feel;
        profile.rotation = raw.rotation;
        profile.provider = raw.provider;
        Ok(profile)
    }
}

impl CrownProfile {
    /// Validate a complete profile before committing an edit.
    pub fn new(enabled: bool, functions: Vec<CrownFunction>) -> Result<Self, CrownError> {
        if functions.is_empty() || functions.len() > MAX_FUNCTIONS {
            return Err(CrownError::FunctionCount);
        }
        let ids: BTreeSet<_> = functions.iter().map(CrownFunction::id).collect();
        if ids.len() != functions.len() {
            return Err(CrownError::DuplicateId);
        }
        Ok(Self {
            enabled,
            functions,
            selection_trigger: CrownSelectionTrigger::default(),
            feel: TactileMode::default(),
            rotation: RotarySettings::default(),
            provider: None,
        })
    }

    /// Whether inputs use the selected function instead of direct bindings.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Toggle without erasing any functions.
    pub const fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Input that selects the next function.
    #[must_use]
    pub const fn selection_trigger(&self) -> CrownSelectionTrigger {
        self.selection_trigger
    }

    /// Reserve a different physical input without discarding its saved binding.
    pub const fn set_selection_trigger(&mut self, trigger: CrownSelectionTrigger) {
        self.selection_trigger = trigger;
    }

    /// Base tactile preference, also used in direct-binding mode.
    #[must_use]
    pub const fn feel(&self) -> TactileMode {
        self.feel
    }

    /// Set this profile's base tactile preference.
    pub const fn set_feel(&mut self, feel: TactileMode) {
        self.feel = feel;
    }

    /// Rotation settings used in direct-binding mode.
    #[must_use]
    pub const fn rotation(&self) -> RotarySettings {
        self.rotation
    }

    /// Change direct-binding rotation settings.
    pub const fn set_rotation(&mut self, settings: RotarySettings) {
        self.rotation = settings;
    }

    /// Explicit application provider, with saved functions retained as fallback.
    #[must_use]
    pub const fn provider(&self) -> Option<&ProviderId> {
        self.provider.as_ref()
    }

    /// Select a provider independently of its current connection state.
    pub fn set_provider(&mut self, provider: Option<ProviderId>) {
        self.provider = provider;
    }

    /// Controls that remain available for action bindings in this profile.
    pub fn binding_controls(&self) -> impl Iterator<Item = CrownControl> {
        CrownControl::ALL.into_iter().filter(|control| {
            *control != CrownControl::Press
                || self.selection_trigger == CrownSelectionTrigger::TouchTap
        })
    }

    /// Validate an edit while retaining enablement and the selected trigger.
    pub fn replace_functions(&mut self, functions: Vec<CrownFunction>) -> Result<(), CrownError> {
        let mut replacement = Self::new(self.enabled, functions)?;
        replacement.selection_trigger = self.selection_trigger;
        replacement.feel = self.feel;
        replacement.rotation = self.rotation;
        replacement.provider.clone_from(&self.provider);
        *self = replacement;
        Ok(())
    }

    /// The nonempty function list, in cycling order.
    #[must_use]
    pub fn functions(&self) -> &[CrownFunction] {
        &self.functions
    }
}

impl Default for CrownProfile {
    fn default() -> Self {
        let functions = [
            (
                "volume",
                "Volume",
                ActionRingIcon::Volume,
                Action::VolumeDown,
                Action::VolumeUp,
                Action::MuteVolume,
            ),
            (
                "workspaces",
                "Workspaces",
                ActionRingIcon::Monitor,
                Action::PreviousDesktop,
                Action::NextDesktop,
                Action::None,
            ),
            (
                "tabs",
                "Tabs",
                ActionRingIcon::NewTab,
                Action::PrevTab,
                Action::NextTab,
                Action::None,
            ),
        ]
        .into_iter()
        .map(|(id, name, icon, left, right, press)| CrownFunction {
            // Built-in identifiers are validated constants; no host state is involved.
            id: CrownFunctionId(id.to_owned()),
            name: name.to_owned(),
            icon,
            feel: TactileMode::default(),
            rotation: RotarySettings::default(),
            bindings: BTreeMap::from([
                (CrownControl::Left, Binding::Single(left)),
                (CrownControl::Right, Binding::Single(right)),
                (CrownControl::Press, Binding::Single(press)),
            ]),
        })
        .collect();
        Self {
            enabled: false,
            functions,
            selection_trigger: CrownSelectionTrigger::default(),
            feel: TactileMode::default(),
            rotation: RotarySettings::default(),
            provider: None,
        }
    }
}

/// Default Crown profile and complete per-application replacements.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[non_exhaustive]
pub struct CrownConfig {
    /// Functions used when no application-specific profile matches.
    pub default: CrownProfile,
    /// Profiles matched by the same application selector as button bindings.
    pub per_app: BTreeMap<String, CrownProfile>,
}

impl CrownConfig {
    /// Whether this config can be omitted from an otherwise unchanged device.
    #[must_use]
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }

    /// Effective profile and its stable application scope (`None` for default).
    #[must_use]
    pub fn resolve(
        &self,
        aliases: &crate::app::AppAliases,
        app: Option<&str>,
    ) -> (Option<&str>, &CrownProfile) {
        app.and_then(|app| profile_key_for(&self.per_app, aliases, app))
            .and_then(|key| self.per_app.get_key_value(key))
            .map_or((None, &self.default), |(key, profile)| {
                (Some(key.as_str()), profile)
            })
    }
}

#[cfg(test)]
mod tests;
