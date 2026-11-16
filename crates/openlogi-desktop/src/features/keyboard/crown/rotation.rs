//! Calibrated rotation drafts and the shared atomic configuration save.

use gpui::{Context, InteractiveElement as _, ParentElement, Styled, Window, div};
use gpui_kit::component::IndexPath;
use gpui_kit::component::{Selectable as _, h_flex, v_flex};
use openlogi_core::crown::{CrownFunction, CrownProfile};
use openlogi_core::rotary::{RotarySensitivity, RotarySettings, TactileMode};

use super::CrownEditor;
use crate::state::AppState;
use crate::ui::components::{Toggle, control_button, control_input, control_select};
use crate::ui::theme::Typography as _;

impl CrownEditor {
    pub(super) fn refresh_rotation(
        &mut self,
        profile: &CrownProfile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let function = profile
            .is_enabled()
            .then(|| {
                self.selected.as_ref().and_then(|id| {
                    profile
                        .functions()
                        .iter()
                        .find(|function| function.id() == id)
                })
            })
            .flatten();
        let rotation = (
            function.map(|function| function.id().clone()),
            function.map_or_else(|| profile.feel(), CrownFunction::feel),
            function.map_or_else(|| profile.rotation(), CrownFunction::rotation),
        );
        if self.loaded_rotation.as_ref() != Some(&rotation) {
            self.inverted = rotation.2.inverted;
            self.speed.update(cx, |input, cx| {
                input.set_value(rotation.2.sensitivity.percent().to_string(), window, cx);
            });
            let index = TactileMode::ALL
                .iter()
                .position(|mode| *mode == rotation.1)
                .unwrap_or(0);
            self.feel.update(cx, |select, cx| {
                select.set_selected_index(Some(IndexPath::new(index)), window, cx);
            });
            self.loaded_rotation = Some(rotation);
        }
    }

    fn save_rotation(&mut self, cx: &mut Context<Self>) {
        let speed = self
            .speed
            .read(cx)
            .value()
            .parse::<u16>()
            .ok()
            .and_then(|value| RotarySensitivity::new(value).ok());
        let Some(sensitivity) = speed else {
            self.error = Some(tr!("keyboard.crown_speed_invalid").to_string());
            cx.notify();
            return;
        };
        let Some(state) = AppState::try_read(cx) else {
            return;
        };
        if state.crown_editing_context() != self.context {
            return;
        }
        let mut profile = state.crown_profile();
        let mode = self
            .feel
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or_default();
        let rotation = RotarySettings::default()
            .with_sensitivity(sensitivity)
            .with_inverted(self.inverted);
        if profile.is_enabled() {
            let mut functions = profile.functions().to_vec();
            if let Some(function) = functions
                .iter_mut()
                .find(|function| Some(function.id()) == self.selected.as_ref())
            {
                function.set_feel(mode);
                function.set_rotation(rotation);
            }
            if let Err(error) = profile.replace_functions(functions) {
                self.error = Some(error.to_string());
                cx.notify();
                return;
            }
        } else {
            profile.set_feel(mode);
            profile.set_rotation(rotation);
        }
        self.commit(profile, cx);
    }

    pub(super) fn rotation_controls(&self, cx: &mut Context<Self>) -> gpui::Div {
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .child(div().text_caption().child(tr!("keyboard.crown_feel")))
                    .child(control_select(&self.feel).flex_1()),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_caption()
                            .child(tr!("keyboard.crown_scroll_speed")),
                    )
                    .child(control_input(&self.speed).flex_1()),
            )
            .child(
                Toggle::new("crown-invert")
                    .label(tr!("keyboard.crown_reverse_rotation"))
                    .selected(self.inverted)
                    .on_change(cx.listener(|this, value, _, cx| {
                        this.inverted = *value;
                        cx.notify();
                    })),
            )
            .child(
                control_button("crown-save-rotation")
                    .debug_selector(|| "crown-save-rotation".into())
                    .label(tr!("keyboard.crown_save_rotation"))
                    .on_click(cx.listener(|this, _, _, cx| this.save_rotation(cx))),
            )
    }
}
