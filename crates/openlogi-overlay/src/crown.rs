//! Read-only function HUD. The agent owns selection, action output and expiry.

#[cfg(target_os = "linux")]
mod gnome;
mod theme;

use gpui::{
    App, AppContext as _, Context, IntoElement, ParentElement, Render, Styled, Window,
    WindowHandle, div, rems, size, svg,
};
use openlogi_ipc::{CrownFunctionPresentation, CrownHud};
use tracing::warn;

use self::theme::CrownColors;
use crate::platform::NoticePlacement;

/// Owns either one native Shell connection or one ordinary overlay window.
#[derive(Default)]
pub(crate) struct CrownDisplay {
    window: Option<WindowHandle<CrownView>>,
    #[cfg(target_os = "linux")]
    shell: Option<gnome::Presenter>,
}

impl CrownDisplay {
    pub(crate) fn present(&mut self, hud: Option<CrownHud>, cx: &mut App) {
        #[cfg(target_os = "linux")]
        if uses_gnome_shell() {
            if self.shell.is_none() {
                self.shell = gnome::Presenter::start()
                    .map_err(|error| warn!(%error, "could not start the Crown HUD bridge"))
                    .ok();
            }
            if let Some(shell) = &self.shell {
                shell.show(hud);
            }
            return;
        }
        let Some(hud) = hud else {
            self.close(cx);
            return;
        };
        if let Some(window) = self.window {
            let updated = window.update(cx, |view, _, cx| {
                // A function-count change changes the native window bounds.
                if view.hud.functions.len() != hud.functions.len() {
                    return false;
                }
                view.hud = hud.clone();
                cx.notify();
                true
            });
            if matches!(updated, Ok(true)) {
                return;
            }
        }
        self.close(cx);
        // GPUI's default overlay rem is 16 points. These are native window
        // bounds, while the view's content uses the same relative scale.
        let rows = hud.functions.len().div_ceil(4);
        let rows = f32::from(u8::try_from(rows).unwrap_or(2));
        let dimensions = size(
            rems(22.).to_pixels(gpui::px(16.)),
            rems(1.5 + rows * 5.).to_pixels(gpui::px(16.)),
        );
        #[cfg_attr(
            windows,
            expect(
                unused_mut,
                reason = "only the macOS and Linux placements drain their openings"
            )
        )]
        let mut placement = match NoticePlacement::capture(cx, dimensions) {
            Ok(placement) => placement,
            Err(error) => {
                warn!(%error, "could not locate the Crown HUD display");
                return;
            }
        };
        let alternatives = placement
            .openings()
            .into_iter()
            .map(|mut options| {
                options.app_id = Some("openlogi-crown-hud".into());
                vec![options]
            })
            .collect();
        let Some(handle) = crate::platform::open_first(alternatives, |options| {
            let hud = hud.clone();
            cx.open_window(options, |_, cx| cx.new(|_| CrownView { hud }))
        }) else {
            return;
        };
        let shown = handle
            .update(cx, |_, window, cx| {
                window.set_input_region(Some(&[]));
                placement.show(window).map(|()| window.bounds_changed(cx))
            })
            .and_then(std::convert::identity);
        if let Err(error) = shown {
            warn!(%error, "could not position the Crown HUD");
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        } else {
            self.window = Some(handle);
        }
    }

    fn close(&mut self, cx: &mut App) {
        if let Some(handle) = self.window.take() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }
}

#[cfg(target_os = "linux")]
fn uses_gnome_shell() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| {
        desktop
            .split(':')
            .any(|part| part.eq_ignore_ascii_case("gnome"))
    })
}

struct CrownView {
    hud: CrownHud,
}

impl CrownView {
    fn function(&self, function: &CrownFunctionPresentation, colors: CrownColors) -> gpui::Div {
        let selected = function.id == self.hud.selected;
        let value = if selected {
            match &function.feedback {
                Some(openlogi_core::crown::CrownFeedback::Parameter(
                    openlogi_core::parameter::ParameterState::Available(value),
                )) => Some(value.text().to_owned()),
                Some(openlogi_core::crown::CrownFeedback::Parameter(
                    openlogi_core::parameter::ParameterState::Unavailable,
                )) => Some(rust_i18n::t!("common.unavailable").into_owned()),
                _ => None,
            }
        } else {
            None
        };
        div()
            .w_16()
            .flex()
            .flex_col()
            .items_center()
            .gap_2()
            .child(
                div()
                    .size_11()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .text_color(if selected {
                        colors.selected_text
                    } else {
                        colors.text
                    })
                    .bg(if selected {
                        colors.selected
                    } else {
                        colors.resting
                    })
                    .children(value.clone().map(|value| {
                        div()
                            .w_full()
                            .text_center()
                            .text_xs()
                            .truncate()
                            .child(value)
                    }))
                    .children(value.is_none().then(|| {
                        svg()
                            .path(function.icon.asset_path())
                            .size_6()
                            .text_color(if selected {
                                colors.selected_text
                            } else {
                                colors.text
                            })
                    })),
            )
            .child(
                div()
                    .w_full()
                    .truncate()
                    .text_center()
                    .text_xs()
                    .text_color(colors.text)
                    .child(function.name.clone()),
            )
    }
}

impl Render for CrownView {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let colors = CrownColors::for_appearance(window.appearance());
        div()
            .size_full()
            .p_3()
            .rounded_2xl()
            .bg(colors.panel)
            .flex()
            .flex_wrap()
            .gap_3()
            .justify_center()
            .items_center()
            .children(
                self.hud
                    .functions
                    .iter()
                    .map(|function| self.function(function, colors)),
            )
    }
}
