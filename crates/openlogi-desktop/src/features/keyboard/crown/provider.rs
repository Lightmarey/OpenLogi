//! Provider references and async catalog updates within one Crown editing scope.

use gpui::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement,
    SharedString, Styled, Subscription, Window, div, prelude::FluentBuilder as _,
};
use gpui_kit::component::{
    button::ButtonVariants as _,
    h_flex,
    input::InputState,
    select::{SelectEvent, SelectItem, SelectState},
    v_flex,
};
use openlogi_core::application_control::{ProviderDescriptor, ProviderId};
use openlogi_core::crown::CrownProfile;

use super::CrownEditor;
use crate::state::AppState;
use crate::ui::components::{control_button, control_input, control_select};
use crate::ui::theme::{self, Typography as _};

#[derive(Clone)]
struct ProviderItem {
    id: Option<ProviderId>,
    name: String,
}

impl SelectItem for ProviderItem {
    type Value = Option<ProviderId>;
    fn title(&self) -> SharedString {
        self.name.clone().into()
    }
    fn value(&self) -> &Self::Value {
        &self.id
    }
}

#[derive(Clone, PartialEq, Eq)]
enum ProviderChoice {
    Local,
    Application(ProviderId),
}

pub(super) struct ProviderDraft {
    pub(super) id_input: Entity<InputState>,
    choices: Entity<SelectState<Vec<ProviderItem>>>,
    choice: Option<ProviderId>,
    loaded: Option<ProviderChoice>,
    providers: Vec<ProviderDescriptor>,
    _selection: Subscription,
}

impl ProviderDraft {
    pub(super) fn new(window: &mut Window, cx: &mut Context<CrownEditor>) -> Self {
        let provider_id = cx.new(|cx| InputState::new(window, cx));
        let provider = cx.new(|cx| SelectState::new(Vec::<ProviderItem>::new(), None, window, cx));
        let provider_selection = cx.subscribe_in(
            &provider,
            window,
            |this, _, _: &SelectEvent<Vec<ProviderItem>>, window, cx| {
                let choice = this
                    .provider
                    .choices
                    .read(cx)
                    .selected_value()
                    .cloned()
                    .flatten();
                if choice == this.provider.choice {
                    return;
                }
                this.provider.choice.clone_from(&choice);
                this.provider.id_input.update(cx, |input, cx| {
                    input.set_value(choice.as_ref().map_or("", AsRef::as_ref), window, cx);
                });
            },
        );
        Self {
            id_input: provider_id,
            choices: provider,
            choice: None,
            loaded: None,
            providers: Vec::new(),
            _selection: provider_selection,
        }
    }

    pub(super) fn refresh(
        &mut self,
        profile: &CrownProfile,
        providers: Vec<ProviderDescriptor>,
        context_changed: bool,
        window: &mut Window,
        cx: &mut Context<CrownEditor>,
    ) {
        if context_changed {
            self.loaded = None;
        }
        let provider = profile.provider().cloned();
        let loaded = provider
            .clone()
            .map_or(ProviderChoice::Local, ProviderChoice::Application);
        let provider_changed = self.loaded.as_ref() != Some(&loaded);
        if provider_changed {
            self.choice.clone_from(&provider);
            self.id_input.update(cx, |input, cx| {
                input.set_value(provider.as_ref().map_or("", AsRef::as_ref), window, cx);
            });
            self.loaded = Some(loaded);
        }
        if provider_changed || self.providers != providers {
            self.providers = providers;
            let mut items = vec![ProviderItem {
                id: None,
                name: tr!("keyboard.crown_local_functions").to_string(),
            }];
            items.extend(self.providers.iter().map(|provider| ProviderItem {
                id: Some(provider.id.clone()),
                name: provider.name.clone(),
            }));
            if let Some(id) = &self.choice
                && !items.iter().any(|item| item.id.as_ref() == Some(id))
            {
                items.push(ProviderItem {
                    id: Some(id.clone()),
                    name: format!(
                        "{} ({})",
                        id.as_ref(),
                        tr!("keyboard.crown_provider_offline")
                    ),
                });
            }
            self.choices.update(cx, |select, cx| {
                select.set_items(items, window, cx);
                select.set_selected_value(&self.choice, window, cx);
            });
        }
    }
}

impl CrownEditor {
    pub(super) fn save_provider(&mut self, cx: &mut Context<Self>) {
        let value = self.provider.id_input.read(cx).value().trim().to_owned();
        let provider = if value.is_empty() {
            None
        } else {
            let Ok(id) = ProviderId::new(value) else {
                self.error = Some(tr!("keyboard.crown_provider_invalid").to_string());
                cx.notify();
                return;
            };
            Some(id)
        };
        let Some(state) =
            AppState::try_read(cx).filter(|state| state.crown_editing_context() == self.context)
        else {
            return;
        };
        let mut profile = state.crown_profile();
        profile.set_provider(provider);
        self.commit(profile, cx);
    }

    pub(super) fn provider_controls(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = theme::palette(cx);
        let configured = match self.provider.loaded.as_ref() {
            Some(ProviderChoice::Application(id)) => Some(id),
            _ => None,
        };
        let online = configured.is_some_and(|id| {
            self.provider
                .providers
                .iter()
                .any(|provider| &provider.id == id)
        });
        v_flex()
            .gap_2()
            .child(div().text_caption().child(tr!("keyboard.crown_provider")))
            .child(control_select(&self.provider.choices).w_full())
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .debug_selector(|| "crown-provider-id".into())
                            .child(control_input(&self.provider.id_input).w_full()),
                    )
                    .child(
                        control_button("crown-provider-save")
                            .debug_selector(|| "crown-provider-save".into())
                            .label(tr!("common.save"))
                            .on_click(cx.listener(|this, _, _, cx| this.save_provider(cx))),
                    )
                    .child(
                        control_button("crown-provider-cancel")
                            .debug_selector(|| "crown-provider-cancel".into())
                            .ghost()
                            .label(tr!("common.cancel"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.provider.loaded = None;
                                this.refresh(window, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .text_caption()
                    .text_color(pal.text_muted)
                    .child(tr!("keyboard.crown_provider_id_hint")),
            )
            .when(configured.is_some(), |body| {
                body.child(
                    div()
                        .text_caption()
                        .text_color(pal.text_muted)
                        .child(if online {
                            tr!("keyboard.crown_provider_connected")
                        } else {
                            tr!("keyboard.crown_provider_offline_hint")
                        }),
                )
            })
    }
}
