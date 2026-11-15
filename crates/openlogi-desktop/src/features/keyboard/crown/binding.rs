//! Crown binding drafts composed from the existing action and field editors.

use std::rc::Rc;

use gpui::InteractiveElement as _;
use gpui::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement, Render, Styled, Window, div,
    prelude::FluentBuilder as _,
};
use gpui_kit::component::{
    Selectable as _, button::ButtonVariants as _, h_flex, input::InputState,
    scroll::ScrollableElement as _, v_flex,
};
use openlogi_core::binding::{Action, Binding, ButtonId, LongPressBinding};
use openlogi_core::crown::validate_input_binding;

use crate::features::binding_editor::{
    ActionOffer, EditsShortcut, PickFn, Recording, RecordsShortcut, ShortcutDraft,
    action_rows_with, application_editor, command_editor, shortcut_editor,
};
use crate::ui::action::localized_action_label;
use crate::ui::components::control_button;
use crate::ui::theme::{self, Typography as _};

pub(super) type SaveFn = Rc<dyn Fn(Binding, &mut App)>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditorKind {
    Actions,
    Shortcut,
    Application,
    Command,
}

impl EditorKind {
    const ALL: [Self; 4] = [
        Self::Actions,
        Self::Shortcut,
        Self::Application,
        Self::Command,
    ];

    fn title(self) -> gpui::SharedString {
        match self {
            Self::Actions => tr!("keyboard.crown_actions"),
            Self::Shortcut => tr!("action_ring.custom_shortcut"),
            Self::Application => tr!("action_ring.open_application_or_folder"),
            Self::Command => tr!("actions.run_shell_command_heading"),
        }
    }
}

pub(super) struct BindingEditor {
    button: ButtonId,
    short: Action,
    long: Option<Action>,
    editing_long: bool,
    kind: EditorKind,
    thresholds: bool,
    shortcut: ShortcutDraft,
    path: Entity<InputState>,
    command: Entity<InputState>,
    invalid: Option<EditorKind>,
    error: Option<String>,
    pub(super) save: SaveFn,
    cancel: Rc<dyn Fn(&mut App)>,
}

impl BindingEditor {
    pub(super) fn new(
        button: ButtonId,
        seed: Binding,
        thresholds: bool,
        save: SaveFn,
        cancel: Rc<dyn Fn(&mut App)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (short, long) = match seed {
            Binding::LongPress(pair) => (pair.short().clone(), Some(pair.long().clone())),
            binding => (binding.click_action(), None),
        };
        let shortcut = ShortcutDraft::new(window, cx);
        let path = cx.new(|cx| InputState::new(window, cx));
        let command = cx.new(|cx| InputState::new(window, cx));
        let mut editor = Self {
            button,
            short,
            long,
            editing_long: false,
            kind: EditorKind::Actions,
            thresholds,
            shortcut,
            path,
            command,
            invalid: None,
            error: None,
            save,
            cancel,
        };
        editor.seed_fields(window, cx);
        editor
    }

    fn action(&self) -> &Action {
        if self.editing_long {
            self.long.as_ref().unwrap_or(&self.short)
        } else {
            &self.short
        }
    }

    fn binding(&self, action: Action) -> Binding {
        let short = if self.editing_long {
            self.short.clone()
        } else {
            action.clone()
        };
        let long = if self.editing_long {
            Some(action)
        } else {
            self.long.clone()
        };
        match long {
            Some(long) => Binding::LongPress(LongPressBinding::new(short, long)),
            None => Binding::Single(short),
        }
    }

    fn seed_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let action = self.action().clone();
        self.shortcut.reset(Some(&action), window, cx);
        self.shortcut
            .set_hold_allowed(openlogi_core::crown::is_held_output_allowed(
                self.button,
                self.long.is_some() && !self.editing_long,
            ));
        let path = match &action {
            Action::OpenApplication(target) => target.path(),
            _ => "",
        };
        self.path
            .update(cx, |input, cx| input.set_value(path, window, cx));
        let command = match &action {
            Action::RunShellCommand(command) => command.as_str(),
            _ => "",
        };
        self.command
            .update(cx, |input, cx| input.set_value(command, window, cx));
        self.error = None;
        self.invalid = None;
    }

    fn pick(&mut self, action: Action, cx: &mut Context<Self>) {
        let binding = self.binding(action.clone());
        if let Err(error) = validate_input_binding(self.button, &binding) {
            self.error = Some(error.to_string());
        } else if self.editing_long {
            self.long = Some(action);
            self.error = None;
        } else {
            self.short = action;
            self.error = None;
        }
        cx.notify();
    }
}

impl RecordsShortcut for BindingEditor {
    fn shortcut_recorded(
        &mut self,
        recording: Recording,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.shortcut.recorded(recording, window, cx);
        cx.notify();
    }
}

impl EditsShortcut for BindingEditor {
    fn shortcut_draft(&mut self) -> Option<&mut ShortcutDraft> {
        Some(&mut self.shortcut)
    }
}

impl BindingEditor {
    fn threshold_controls(&self, cx: &mut Context<Self>) -> gpui::Div {
        h_flex()
            .gap_2()
            .flex_wrap()
            .child(
                control_button("crown-short")
                    .debug_selector(|| "crown-short".into())
                    .label(tr!("keyboard.crown_short_press"))
                    .selected(!self.editing_long)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.editing_long = false;
                        this.seed_fields(window, cx);
                        cx.notify();
                    })),
            )
            .child(
                control_button("crown-long")
                    .debug_selector(|| "crown-long".into())
                    .label(tr!("keyboard.crown_long_press"))
                    .selected(self.editing_long)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.long.get_or_insert(Action::None);
                        this.editing_long = true;
                        this.seed_fields(window, cx);
                        cx.notify();
                    })),
            )
            .when(self.long.is_some(), |row| {
                row.child(
                    control_button("crown-clear-long")
                        .debug_selector(|| "crown-clear-long".into())
                        .ghost()
                        .label(tr!("keyboard.crown_remove_long_press"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.long = None;
                            this.editing_long = false;
                            this.seed_fields(window, cx);
                            cx.notify();
                        })),
                )
            })
    }

    fn action_fields(
        &self,
        rows: Vec<gpui::Div>,
        pick: &PickFn,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let pal = theme::palette(cx);
        let view = cx.entity();
        let invalid_path = view.downgrade();
        let invalid_command = view.downgrade();
        v_flex()
            .flex_1()
            .min_h_0()
            .gap_2()
            .when(self.kind == EditorKind::Actions, |body| {
                body.child(
                    div()
                        .id("crown-action-list")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scrollbar()
                        .children(rows),
                )
            })
            .when(self.kind == EditorKind::Shortcut, |body| {
                body.child(shortcut_editor("crown", &self.shortcut, &view, pick, pal))
            })
            .when(self.kind == EditorKind::Application, |body| {
                body.child(application_editor(
                    "crown",
                    &self.path,
                    self.invalid == Some(EditorKind::Application),
                    Rc::new(move |cx| {
                        if let Some(view) = invalid_path.upgrade() {
                            view.update(cx, |view, cx| {
                                view.invalid = Some(EditorKind::Application);
                                cx.notify();
                            });
                        }
                    }),
                    pick,
                    pal,
                ))
            })
            .when(self.kind == EditorKind::Command, |body| {
                body.child(command_editor(
                    "crown",
                    &self.command,
                    self.invalid == Some(EditorKind::Command),
                    Rc::new(move |cx| {
                        if let Some(view) = invalid_command.upgrade() {
                            view.update(cx, |view, cx| {
                                view.invalid = Some(EditorKind::Command);
                                cx.notify();
                            });
                        }
                    }),
                    pick,
                    pal,
                ))
            })
    }
}

impl Render for BindingEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pal = theme::palette(cx);
        self.shortcut.localize(window, cx);
        let view = cx.entity();
        let weak = view.downgrade();
        let pick: PickFn = Rc::new(move |action, _, cx| {
            if let Some(view) = weak.upgrade() {
                view.update(cx, |view, cx| view.pick(action, cx));
            }
        });
        let hidden: Vec<_> = Action::catalog()
            .into_iter()
            .filter(|action| {
                validate_input_binding(self.button, &self.binding(action.clone())).is_err()
            })
            .collect();
        let rows = action_rows_with(
            "crown-action",
            Some(self.action()),
            "",
            ActionOffer {
                hidden: &hidden,
                extra: &[Action::None],
            },
            &pick,
            pal,
        );
        v_flex()
            .size_full()
            .min_h_0()
            .gap_2()
            .when(self.thresholds, |body| {
                body.child(self.threshold_controls(cx))
            })
            .child(
                div()
                    .text_caption()
                    .text_color(pal.text_muted)
                    .child(localized_action_label(self.action())),
            )
            .child(h_flex().gap_1().flex_wrap().children(
                EditorKind::ALL.into_iter().enumerate().map(|(ix, kind)| {
                    control_button(("crown-editor-kind", ix))
                        .label(kind.title())
                        .selected(self.kind == kind)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.kind = kind;
                            cx.notify();
                        }))
                }),
            ))
            .child(self.action_fields(rows, &pick, cx))
            .when_some(self.error.as_ref(), |body, error| {
                body.child(
                    div()
                        .text_caption()
                        .text_color(pal.text_danger)
                        .child(error.clone()),
                )
            })
            .child(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        control_button("crown-binding-cancel")
                            .debug_selector(|| "crown-binding-cancel".into())
                            .label(tr!("common.cancel"))
                            .on_click(cx.listener(|this, _, _, cx| (this.cancel)(cx))),
                    )
                    .child(
                        control_button("crown-binding-save")
                            .debug_selector(|| "crown-binding-save".into())
                            .primary()
                            .label(tr!("common.save"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                let binding = this.binding(this.action().clone());
                                match validate_input_binding(this.button, &binding) {
                                    Ok(()) => (this.save)(binding, cx),
                                    Err(error) => {
                                        this.error = Some(error.to_string());
                                        cx.notify();
                                    }
                                }
                            })),
                    ),
            )
    }
}
