//! Craft Crown profiles and direct inputs in the device application scope.

use std::rc::Rc;

use gpui::InteractiveElement as _;
use gpui::{
    AppContext as _, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled,
    Subscription, Window, div, prelude::FluentBuilder as _, px,
};
use gpui_kit::component::{
    Disableable as _, IndexPath, Selectable as _,
    button::ButtonVariants as _,
    h_flex,
    input::InputState,
    scroll::ScrollableElement as _,
    select::{SelectEvent, SelectItem, SelectState},
    v_flex,
};
use openlogi_core::binding::{ActionRingIcon, Binding, ButtonId};
use openlogi_core::crown::{
    CrownControl, CrownFunction, CrownFunctionId, CrownProfile, CrownSelectionTrigger,
    MAX_FUNCTIONS,
};

use openlogi_core::rotary::{RotarySettings, TactileMode};

mod binding;
mod provider;
mod rotation;
use self::provider::ProviderDraft;
#[cfg(test)]
mod tests;

use self::binding::{BindingEditor, SaveFn};
use crate::state::AppState;
use crate::ui::components::{Toggle, control_button, control_input, control_select};
use crate::ui::theme::{self, Typography as _};

#[derive(Clone)]
struct ControlItem(ButtonId);

#[derive(Clone)]
struct TriggerItem(CrownSelectionTrigger);

impl SelectItem for TriggerItem {
    type Value = CrownSelectionTrigger;
    fn title(&self) -> SharedString {
        rust_i18n::t!(self.0.translation_key()).into_owned().into()
    }
    fn value(&self) -> &CrownSelectionTrigger {
        &self.0
    }
}

impl SelectItem for ControlItem {
    type Value = ButtonId;
    fn title(&self) -> SharedString {
        rust_i18n::t!(self.0.translation_key()).into_owned().into()
    }
    fn value(&self) -> &ButtonId {
        &self.0
    }
}

#[derive(Clone)]
struct FeelItem(TactileMode);

impl SelectItem for FeelItem {
    type Value = TactileMode;
    fn title(&self) -> SharedString {
        rust_i18n::t!(self.0.translation_key()).into_owned().into()
    }
    fn value(&self) -> &TactileMode {
        &self.0
    }
}

pub(super) struct CrownEditor {
    provider: ProviderDraft,
    selected: Option<CrownFunctionId>,
    context: Option<(String, Option<String>)>,
    name: Entity<InputState>,
    feel: Entity<SelectState<Vec<FeelItem>>>,
    speed: Entity<InputState>,
    inverted: bool,
    loaded_rotation: Option<(Option<CrownFunctionId>, TactileMode, RotarySettings)>,
    loaded_name: Option<(CrownFunctionId, String)>,
    control: Entity<SelectState<Vec<ControlItem>>>,
    trigger: Entity<SelectState<Vec<TriggerItem>>>,
    loaded_trigger: Option<CrownSelectionTrigger>,
    loaded_mode: Option<bool>,
    custom: Option<Entity<BindingEditor>>,
    draft_generation: u64,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl CrownEditor {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let provider = ProviderDraft::new(window, cx);
        let name = cx.new(|cx| InputState::new(window, cx));
        let speed = cx.new(|cx| InputState::new(window, cx));
        let feel = cx.new(|cx| {
            SelectState::new(
                TactileMode::ALL.map(FeelItem).to_vec(),
                Some(IndexPath::default()),
                window,
                cx,
            )
        });
        let control = cx.new(|cx| {
            SelectState::new(
                ButtonId::CROWN.map(ControlItem).to_vec(),
                Some(IndexPath::default()),
                window,
                cx,
            )
        });
        let selection = cx.subscribe_in(
            &control,
            window,
            |this, _, _: &SelectEvent<Vec<ControlItem>>, _, cx| {
                this.custom = None;
                cx.notify();
            },
        );
        let trigger = cx.new(|cx| {
            SelectState::new(
                CrownSelectionTrigger::ALL.map(TriggerItem).to_vec(),
                Some(IndexPath::default()),
                window,
                cx,
            )
        });
        let trigger_selection = cx.subscribe_in(
            &trigger,
            window,
            |this, _, _: &SelectEvent<Vec<TriggerItem>>, _, cx| {
                let Some(trigger) = this.trigger.read(cx).selected_value().copied() else {
                    return;
                };
                let Some(state) = AppState::try_read(cx) else {
                    return;
                };
                let mut profile = state.crown_profile();
                if profile.selection_trigger() != trigger {
                    profile.set_selection_trigger(trigger);
                    this.custom = None;
                    this.commit(profile, cx);
                }
            },
        );
        let state = cx.subscribe_in(&AppState::global(cx), window, |this, _, _, window, cx| {
            this.refresh(window, cx);
        });
        let mut editor = Self {
            provider,
            selected: None,
            context: None,
            name,
            feel,
            speed,
            inverted: false,
            loaded_rotation: None,
            loaded_name: None,
            control,
            trigger,
            loaded_trigger: None,
            loaded_mode: None,
            custom: None,
            draft_generation: 0,
            error: None,
            _subscriptions: vec![selection, trigger_selection, state],
        };
        editor.refresh(window, cx);
        editor
    }

    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = AppState::try_read(cx);
        let context = state.and_then(AppState::crown_editing_context);
        let profile = state.map_or_default(AppState::crown_profile);
        let providers = state.map_or_default(|state| state.application_providers().to_vec());
        let context_changed = context != self.context;
        if context_changed {
            self.context = context;
            self.selected = None;
            self.loaded_name = None;
            self.custom = None;
            self.loaded_trigger = None;
            self.loaded_rotation = None;
        }
        self.provider
            .refresh(&profile, providers, context_changed, window, cx);
        if !profile
            .functions()
            .iter()
            .any(|function| Some(function.id()) == self.selected.as_ref())
        {
            self.selected = profile
                .functions()
                .first()
                .map(|function| function.id().clone());
            self.custom = None;
        }
        let name = profile
            .functions()
            .iter()
            .find(|function| Some(function.id()) == self.selected.as_ref())
            .map(|function| (function.id().clone(), function.name().to_owned()));
        // State changes from the same scope must not erase an unfinished rename.
        if name != self.loaded_name {
            self.name.update(cx, |input, cx| {
                input.set_value(
                    name.as_ref()
                        .map_or_else(String::new, |(_, name)| name.clone()),
                    window,
                    cx,
                );
            });
            self.loaded_name = name;
        }
        self.refresh_rotation(&profile, window, cx);
        let trigger = profile.selection_trigger();
        if self.loaded_trigger != Some(trigger) || self.loaded_mode != Some(profile.is_enabled()) {
            self.loaded_mode = Some(profile.is_enabled());
            self.loaded_trigger = Some(trigger);
            self.custom = None;
            let controls: Vec<_> = if profile.is_enabled() {
                profile
                    .binding_controls()
                    .map(CrownControl::button)
                    .collect()
            } else {
                ButtonId::CROWN.to_vec()
            };
            let selected = self
                .control
                .read(cx)
                .selected_value()
                .copied()
                .filter(|control| controls.contains(control))
                .unwrap_or(ButtonId::CrownLeft);
            self.control.update(cx, |state, cx| {
                state.set_items(controls.into_iter().map(ControlItem).collect(), window, cx);
                state.set_selected_value(&selected, window, cx);
            });
            self.trigger.update(cx, |state, cx| {
                state.set_selected_value(&trigger, window, cx);
            });
        }
        cx.notify();
    }

    fn select(&mut self, id: CrownFunctionId, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = Some(id);
        self.custom = None;
        self.error = None;
        self.refresh(window, cx);
    }

    fn commit(&mut self, profile: CrownProfile, cx: &mut Context<Self>) {
        let Some(context) = self.context.clone() else {
            return;
        };
        self.error = None;
        AppState::apply(cx, |state| state.commit_crown_profile(&context, profile));
    }

    fn edit_functions(
        &mut self,
        edit: impl FnOnce(&mut Vec<CrownFunction>) -> Result<(), openlogi_core::crown::CrownError>,
        cx: &mut Context<Self>,
    ) {
        let Some(state) = AppState::try_read(cx) else {
            return;
        };
        let mut profile = state.crown_profile();
        let mut functions = profile.functions().to_vec();
        let result = edit(&mut functions).and_then(|()| profile.replace_functions(functions));
        match result {
            Ok(()) => self.commit(profile, cx),
            Err(error) => {
                self.error = Some(error.to_string());
                cx.notify();
            }
        }
    }

    fn save_binding(
        &mut self,
        id: &CrownFunctionId,
        control: CrownControl,
        binding: Binding,
        cx: &mut Context<Self>,
    ) {
        self.edit_functions(
            |functions| {
                if let Some(function) = functions.iter_mut().find(|function| function.id() == id) {
                    if control == CrownControl::Left {
                        function.set_icon(ActionRingIcon::for_action(&binding.click_action()));
                    }
                    function.set_binding(control, binding)?;
                }
                Ok(())
            },
            cx,
        );
        self.custom = None;
    }

    fn open_binding(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(context) = self.context.clone() else {
            return;
        };
        let Some(state) = AppState::try_read(cx) else {
            return;
        };
        let button = self
            .control
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or(ButtonId::CrownLeft);
        let profile = state.crown_profile();
        let selected = profile
            .is_enabled()
            .then(|| self.selected.clone())
            .flatten();
        let seed = if profile.is_enabled() {
            let Some(function) = profile
                .functions()
                .iter()
                .find(|function| Some(function.id()) == selected.as_ref())
            else {
                return;
            };
            let Some(control) = CrownControl::for_button(button) else {
                return;
            };
            function.binding(control)
        } else {
            state.crown_input_binding(button)
        };
        let thresholds = (profile.is_enabled() || context.1.is_none())
            && openlogi_core::crown::has_press_edges(button);
        self.draft_generation += 1;
        let generation = self.draft_generation;
        let original = context.clone();
        let weak = cx.entity().downgrade();
        let save: SaveFn = Rc::new(move |binding, cx| {
            if let Some(view) = weak.upgrade() {
                view.update(cx, |view, cx| {
                    if view.custom.is_none()
                        || view.draft_generation != generation
                        || view.context.as_ref() != Some(&original)
                        || view.control.read(cx).selected_value().copied() != Some(button)
                        || AppState::try_read(cx)
                            .and_then(AppState::crown_editing_context)
                            .as_ref()
                            != Some(&original)
                    {
                        return;
                    }
                    if let Some(id) = &selected {
                        if view.selected.as_ref() == Some(id)
                            && let Some(control) = CrownControl::for_button(button)
                        {
                            view.save_binding(id, control, binding, cx);
                        }
                    } else {
                        AppState::apply(cx, |state| {
                            state.commit_crown_input(&original, button, binding)
                        });
                        view.custom = None;
                    }
                    cx.notify();
                });
            }
        });
        let weak = cx.entity().downgrade();
        let cancel = Rc::new(move |cx: &mut gpui::App| {
            if let Some(view) = weak.upgrade() {
                view.update(cx, |view, cx| {
                    view.custom = None;
                    cx.notify();
                });
            }
        });
        self.custom = Some(
            cx.new(|cx| BindingEditor::new(button, seed, thresholds, save, cancel, window, cx)),
        );
        cx.notify();
    }

    fn function_tools(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .gap_1()
            .child(control_input(&self.name).w_full())
            .child(
                control_button("crown-name-save")
                    .debug_selector(|| "crown-name-save".into())
                    .ghost()
                    .label(tr!("common.save"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        let Some(id) = this.selected.clone() else {
                            return;
                        };
                        let name = this.name.read(cx).value().to_string();
                        this.edit_functions(
                            |functions| {
                                if let Some(function) =
                                    functions.iter_mut().find(|function| function.id() == &id)
                                {
                                    function.rename(&name)?;
                                }
                                Ok(())
                            },
                            cx,
                        );
                    })),
            )
    }
}

impl CrownEditor {
    fn enable_functions(&mut self, enabled: bool, cx: &mut Context<Self>) {
        let mut profile = AppState::try_read(cx).map_or_default(AppState::crown_profile);
        if enabled && profile == CrownProfile::default() {
            let mut functions = profile.functions().to_vec();
            for (function, name) in functions.iter_mut().zip([
                tr!("keyboard.crown_volume"),
                tr!("keyboard.crown_workspaces"),
                tr!("keyboard.crown_tabs"),
            ]) {
                if let Err(error) = function.rename(name.as_ref()) {
                    self.error = Some(error.to_string());
                    cx.notify();
                    return;
                }
            }
            match profile.replace_functions(functions) {
                Ok(()) => {}
                Err(error) => {
                    self.error = Some(error.to_string());
                    cx.notify();
                    return;
                }
            }
        }
        profile.set_enabled(enabled);
        self.custom = None;
        self.commit(profile, cx);
    }

    fn add_function(&mut self, cx: &mut Context<Self>) {
        let name = tr!("keyboard.crown_new_function").to_string();
        let mut added = None;
        self.edit_functions(
            |functions| {
                let id = CrownFunctionId::new(format!("function-{:032x}", fastrand::u128(..)))?;
                functions.push(CrownFunction::new(
                    id.clone(),
                    &name,
                    ActionRingIcon::Keyboard,
                )?);
                added = Some(id);
                Ok(())
            },
            cx,
        );
        if self.error.is_none() {
            self.selected = added;
            self.custom = None;
        }
    }

    fn move_function(&mut self, forward: bool, cx: &mut Context<Self>) {
        let selected = self.selected.clone();
        self.edit_functions(
            |functions| {
                if let Some(index) = functions
                    .iter()
                    .position(|function| Some(function.id()) == selected.as_ref())
                {
                    let next = if forward {
                        index.checked_add(1)
                    } else {
                        index.checked_sub(1)
                    };
                    if let Some(next) = next.filter(|next| *next < functions.len()) {
                        functions.swap(index, next);
                    }
                }
                Ok(())
            },
            cx,
        );
    }

    fn function_management(&self, profile: &CrownProfile, cx: &mut Context<Self>) -> gpui::Div {
        let index = profile
            .functions()
            .iter()
            .position(|function| Some(function.id()) == self.selected.as_ref())
            .unwrap_or(0);
        let buttons = profile
            .functions()
            .iter()
            .map(|function| {
                let id = function.id().clone();
                control_button(SharedString::from(format!(
                    "crown-function-{}",
                    id.as_ref()
                )))
                .ghost()
                .selected(Some(function.id()) == self.selected.as_ref())
                .label(function.name().to_owned())
                .on_click(
                    cx.listener(move |this, _, window, cx| this.select(id.clone(), window, cx)),
                )
            })
            .collect::<Vec<_>>();
        v_flex()
            .gap_2()
            .child(h_flex().flex_wrap().gap_1().children(buttons))
            .child(self.function_tools(cx))
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        control_button("crown-add")
                            .debug_selector(|| "crown-add".into())
                            .ghost()
                            .label(tr!("keyboard.crown_add_function"))
                            .disabled(profile.functions().len() >= MAX_FUNCTIONS)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.add_function(cx);
                                this.refresh(window, cx);
                            })),
                    )
                    .child(
                        control_button("crown-remove")
                            .debug_selector(|| "crown-remove".into())
                            .ghost()
                            .label(tr!("keyboard.crown_remove_function"))
                            .disabled(profile.functions().len() <= 1)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let selected = this.selected.clone();
                                this.custom = None;
                                this.edit_functions(
                                    |functions| {
                                        functions.retain(|function| {
                                            Some(function.id()) != selected.as_ref()
                                        });
                                        Ok(())
                                    },
                                    cx,
                                );
                            })),
                    )
                    .child(
                        control_button("crown-move-up")
                            .debug_selector(|| "crown-move-up".into())
                            .ghost()
                            .label("↑")
                            .tooltip(tr!("keyboard.crown_move_up"))
                            .disabled(index == 0)
                            .on_click(cx.listener(|this, _, _, cx| this.move_function(false, cx))),
                    )
                    .child(
                        control_button("crown-move-down")
                            .debug_selector(|| "crown-move-down".into())
                            .ghost()
                            .label("↓")
                            .tooltip(tr!("keyboard.crown_move_down"))
                            .disabled(index + 1 >= profile.functions().len())
                            .on_click(cx.listener(|this, _, _, cx| this.move_function(true, cx))),
                    ),
            )
    }
}

impl CrownEditor {
    fn settings_content(&self, profile: &CrownProfile, cx: &mut Context<Self>) -> gpui::Div {
        let pal = theme::palette(cx);
        v_flex()
            .gap_2()
            .when(!profile.is_enabled(), |body| {
                body.child(
                    div()
                        .text_caption()
                        .text_color(pal.text_muted)
                        .child(tr!("keyboard.crown_description")),
                )
            })
            .when(profile.is_enabled(), |body| {
                body.child(
                    h_flex()
                        .gap_2()
                        .h(px(theme::CONTROL_H))
                        .flex_none()
                        .child(
                            div()
                                .text_caption()
                                .child(tr!("keyboard.crown_switch_trigger")),
                        )
                        .child(control_select(&self.trigger).flex_1()),
                )
                .child(
                    div()
                        .text_caption()
                        .text_color(pal.text_muted)
                        .child(tr!("keyboard.crown_function_hint")),
                )
                .when(
                    profile.selection_trigger() == CrownSelectionTrigger::Press,
                    |body| {
                        body.child(
                            div()
                                .text_caption()
                                .text_color(pal.text_muted)
                                .child(tr!("keyboard.crown_press_reserved")),
                        )
                    },
                )
                .when(profile.provider().is_some(), |body| {
                    body.child(
                        div()
                            .text_caption()
                            .text_color(pal.text_muted)
                            .child(tr!("keyboard.crown_fallback_functions")),
                    )
                })
                .child(self.function_management(profile, cx))
            })
            .child(
                div()
                    .h(px(theme::CONTROL_H))
                    .flex_none()
                    .child(control_select(&self.control).w_full()),
            )
            .child(
                control_button("crown-edit-binding")
                    .debug_selector(|| "crown-edit-binding".into())
                    .label(tr!("keyboard.crown_edit_binding"))
                    .on_click(cx.listener(|this, _, window, cx| this.open_binding(window, cx))),
            )
            .child(self.rotation_controls(cx))
            .when(
                profile.is_enabled() && self.context.as_ref().is_some_and(|(_, app)| app.is_some()),
                |body| body.child(self.provider_controls(cx)),
            )
    }
}

impl Render for CrownEditor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let profile = AppState::try_read(cx).map_or_default(AppState::crown_profile);
        let pal = theme::palette(cx);
        v_flex()
            .size_full()
            .min_h_0()
            .gap_2()
            .p_2()
            .child(
                div()
                    .debug_selector(|| "crown-functions-mode".into())
                    .child(
                        Toggle::new("crown-functions-mode")
                            .label(tr!("keyboard.crown_function_mode"))
                            .selected(profile.is_enabled())
                            .disabled(self.context.is_none())
                            .on_change(cx.listener(|this, enabled, _, cx| {
                                this.enable_functions(*enabled, cx);
                            })),
                    ),
            )
            .when_some(self.custom.clone(), |body, editor| {
                let button = self
                    .control
                    .read(cx)
                    .selected_value()
                    .copied()
                    .unwrap_or(ButtonId::CrownLeft);
                let input = rust_i18n::t!(button.translation_key()).into_owned();
                let name = profile
                    .functions()
                    .iter()
                    .find(|function| Some(function.id()) == self.selected.as_ref())
                    .map(CrownFunction::name);
                let label = if profile.is_enabled() {
                    format!("{} · {input}", name.unwrap_or(""))
                } else {
                    input
                };
                body.child(div().text_caption().child(label))
                    .child(div().flex_1().min_h_0().child(editor))
            })
            .when(self.custom.is_none(), |body| {
                body.child(
                    div()
                        .id("crown-settings-scroll")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scrollbar()
                        .child(self.settings_content(&profile, cx)),
                )
            })
            .when_some(self.error.as_ref(), |body, error| {
                body.child(
                    div()
                        .text_caption()
                        .text_color(pal.text_danger)
                        .child(error.clone()),
                )
            })
    }
}
