//! Crown palette shared by the window renderer and Shell bridge.

use gpui::{Hsla, WindowAppearance};
use openlogi_ui::color::{accent_at_lightness, neutral};

#[derive(Clone, Copy)]
pub(super) struct CrownColors {
    pub(super) panel: Hsla,
    pub(super) resting: Hsla,
    pub(super) text: Hsla,
    pub(super) selected: Hsla,
    pub(super) selected_text: Hsla,
}

impl CrownColors {
    pub(super) fn for_appearance(appearance: WindowAppearance) -> Self {
        let (panel, resting, text) = match appearance {
            WindowAppearance::Light | WindowAppearance::VibrantLight => {
                (neutral(0.97, 0.96), neutral(0.91, 1.0), neutral(0.15, 1.0))
            }
            WindowAppearance::Dark | WindowAppearance::VibrantDark => {
                (neutral(0.08, 0.96), neutral(0.18, 1.0), neutral(0.96, 1.0))
            }
        };
        Self {
            panel,
            resting,
            text,
            selected: accent_at_lightness(0.48),
            selected_text: neutral(1.0, 1.0),
        }
    }
}
