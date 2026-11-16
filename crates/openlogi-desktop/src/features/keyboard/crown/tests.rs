use super::*;
use crate::state::ConfigPersistence;
use crate::state::tests::crown::{KEY, crown_state};
use gpui::{TestAppContext, VisualTestContext, px, size};
use openlogi_core::application_control::{ProviderDescriptor, ProviderId};
use openlogi_core::binding::Action;
use openlogi_core::config::{Config, ConfigFile};

fn editor(
    cx: &mut TestAppContext,
) -> (
    Entity<CrownEditor>,
    &mut VisualTestContext,
    tempfile::TempDir,
) {
    cx.update(gpui_kit::init);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(openlogi_core::paths::CONFIG_FILE);
    let (config, file) = ConfigFile::load_from_path(&path).unwrap();
    let (state, _commands) = crown_state(config, ConfigPersistence::UserFile(file));
    cx.update(|cx| {
        let state = cx.new(|_| state);
        AppState::set_global(state, cx);
    });
    let (view, cx) = cx.add_window_view(CrownEditor::new);
    cx.simulate_resize(size(px(340.), px(540.)));
    draw(cx);
    (view, cx, dir)
}

fn draw(cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.draw(cx).clear(cx));
}

#[gpui::test]
fn crown_provider_drafts_survive_directory_updates_cancel_and_save_offline(
    cx: &mut TestAppContext,
) {
    let _locale = crate::services::i18n::LOCALE_LOCK.lock().unwrap();
    let (view, cx, dir) = editor(cx);
    cx.update(|_, cx| {
        AppState::apply(cx, |state| {
            state.set_editing_app(Some("org.example.Editor".into()))
        });
    });
    draw(cx);
    click(cx, "crown-functions-mode");
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.provider.id_input.update(cx, |input, cx| {
                input.set_value("offline-preview", window, cx);
            });
        });
        AppState::apply(cx, |state| {
            state.set_application_providers(vec![ProviderDescriptor::new(
                ProviderId::new("online-preview".into()).unwrap(),
                "Online preview".into(),
            )])
        });
    });
    draw(cx);
    view.read_with(cx, |view, cx| {
        assert_eq!(
            view.provider.id_input.read(cx).value().as_ref(),
            "offline-preview"
        );
    });
    cx.simulate_event(gpui::ScrollEvent {
        position: gpui::point(px(170.), px(400.)),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-1600.))),
        modifiers: gpui::Modifiers::default(),
        touch_phase: gpui::TouchPhase::Moved,
        source: gpui::ScrollSource::Pointer,
    });
    draw(cx);
    click(cx, "crown-provider-save");
    let path = dir.path().join(openlogi_core::paths::CONFIG_FILE);
    let saved = Config::load_from_path(&path).unwrap();
    assert_eq!(
        saved.devices[KEY].crown.per_app["org.example.Editor"]
            .provider()
            .unwrap()
            .as_ref(),
        "offline-preview"
    );
    let bytes = std::fs::read(&path).unwrap();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.provider.id_input.update(cx, |input, cx| {
                input.set_value("unsaved-preview", window, cx);
            });
        });
    });
    draw(cx);
    click(cx, "crown-provider-cancel");
    view.read_with(cx, |view, cx| {
        assert_eq!(
            view.provider.id_input.read(cx).value().as_ref(),
            "offline-preview"
        );
    });
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.provider
                .id_input
                .update(cx, |input, cx| input.set_value("bad provider", window, cx));
        });
    });
    draw(cx);
    click(cx, "crown-provider-save");
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    view.read_with(cx, |view, _| assert!(view.error.is_some()));
    cx.update(|_, cx| {
        AppState::apply(cx, |state| state.set_editing_app(None));
        view.update(cx, super::CrownEditor::save_provider);
    });
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
}

#[gpui::test]
fn crown_rotation_settings_save_and_invalid_speed_preserves_the_saved_profile(
    cx: &mut TestAppContext,
) {
    let _locale = crate::services::i18n::LOCALE_LOCK.lock().unwrap();
    let (view, cx, dir) = editor(cx);
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.feel.update(cx, |select, cx| {
                select.set_selected_index(Some(IndexPath::new(1)), window, cx);
            });
            view.speed
                .update(cx, |speed, cx| speed.set_value("150", window, cx));
        });
    });
    draw(cx);
    click(cx, "crown-save-rotation");
    let path = dir.path().join(openlogi_core::paths::CONFIG_FILE);
    let saved = Config::load_from_path(&path).unwrap();
    assert_eq!(saved.devices[KEY].crown.default.feel(), TactileMode::Smooth);
    assert_eq!(
        saved.devices[KEY]
            .crown
            .default
            .rotation()
            .sensitivity
            .percent(),
        150
    );
    let saved_bytes = std::fs::read(&path).unwrap();
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.speed
                .update(cx, |speed, cx| speed.set_value("0", window, cx));
        });
    });
    draw(cx);
    click(cx, "crown-save-rotation");
    assert_eq!(std::fs::read(&path).unwrap(), saved_bytes);
    view.read_with(cx, |view, _| assert!(view.error.is_some()));
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .expect("Crown control is rendered");
    assert!(
        bounds.top() >= px(0.) && bounds.bottom() <= px(540.),
        "{selector} stays reachable in the narrow inspector: {bounds:?}"
    );
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    draw(cx);
}

#[gpui::test]
fn crown_direct_binding_can_cancel_save_and_reopen_through_its_controls(cx: &mut TestAppContext) {
    let _locale = crate::services::i18n::LOCALE_LOCK.lock().unwrap();
    let (view, cx, dir) = editor(cx);
    click(cx, "crown-edit-binding");
    click(cx, "crown-action-Copy");
    click(cx, "crown-binding-cancel");
    cx.read(|cx| {
        assert_eq!(
            AppState::try_read(cx)
                .unwrap()
                .crown_input_binding(ButtonId::CrownTouch),
            Binding::Single(Action::None)
        );
    });
    click(cx, "crown-edit-binding");
    click(cx, "crown-action-Copy");
    click(cx, "crown-binding-save");
    let saved =
        Config::load_from_path(&dir.path().join(openlogi_core::paths::CONFIG_FILE)).unwrap();
    assert_eq!(
        saved.stored_bindings(KEY)[&ButtonId::CrownTouch],
        Binding::Single(Action::Copy)
    );
    click(cx, "crown-edit-binding");
    view.read_with(cx, |view, _| assert!(view.custom.is_some()));
    assert!(cx.debug_bounds("crown-action-Copy-check").is_some());
}

#[gpui::test]
fn crown_function_management_reserves_press_and_retains_stable_selection(cx: &mut TestAppContext) {
    let _locale = crate::services::i18n::LOCALE_LOCK.lock().unwrap();
    let (view, cx, _dir) = editor(cx);
    click(cx, "crown-functions-mode");
    view.read_with(cx, |view, cx| {
        assert_eq!(view.selected.as_ref().unwrap().as_ref(), "volume");
        assert_ne!(
            view.control.read(cx).selected_value(),
            Some(&ButtonId::CrownPress)
        );
        assert!(
            !AppState::try_read(cx)
                .unwrap()
                .crown_profile()
                .binding_controls()
                .any(|control| control == CrownControl::Press)
        );
    });
    click(cx, "crown-add");
    let selected = view.read_with(cx, |view, _| view.selected.clone());
    click(cx, "crown-move-up");
    assert_eq!(
        view.read_with(cx, |view, _| view.selected.clone()),
        selected
    );
    click(cx, "crown-remove");
    cx.read(|cx| {
        assert_eq!(
            AppState::try_read(cx)
                .unwrap()
                .crown_profile()
                .functions()
                .len(),
            3
        );
    });
}

#[gpui::test]
fn crown_binding_callbacks_expire_when_the_device_app_or_draft_changes(cx: &mut TestAppContext) {
    let _locale = crate::services::i18n::LOCALE_LOCK.lock().unwrap();
    let (view, cx, _dir) = editor(cx);
    click(cx, "crown-edit-binding");
    let stale = view.read_with(cx, |view, cx| {
        view.custom.as_ref().unwrap().read(cx).save.clone()
    });
    click(cx, "crown-binding-cancel");
    click(cx, "crown-edit-binding");
    cx.update(|_, cx| stale(Binding::Single(Action::Copy), cx));
    cx.read(|cx| {
        assert_eq!(
            AppState::try_read(cx)
                .unwrap()
                .crown_input_binding(ButtonId::CrownTouch),
            Binding::Single(Action::None)
        );
    });
    cx.update(|_, cx| {
        AppState::apply(cx, |state| {
            state.set_editing_app(Some("org.example.Editor".into()))
        });
    });
    draw(cx);
    cx.update(|_, cx| stale(Binding::Single(Action::Copy), cx));
    cx.read(|cx| {
        assert_eq!(
            AppState::try_read(cx)
                .unwrap()
                .crown_input_binding(ButtonId::CrownTouch),
            Binding::Single(Action::None)
        );
    });
    view.read_with(cx, |view, _| assert!(view.custom.is_none()));
}

#[gpui::test]
fn craft_crown_direct_controls_keep_the_edit_action_with_its_input(cx: &mut TestAppContext) {
    let _locale = crate::services::i18n::LOCALE_LOCK.lock().unwrap();
    let (_view, cx, _dir) = editor(cx);
    let edit = cx.debug_bounds("crown-edit-binding").unwrap();
    assert!(
        edit.top() < px(250.),
        "the input and Edit action stay grouped: {edit:?}"
    );
}

#[gpui::test]
fn inherited_threshold_can_be_overridden_in_a_direct_application_profile(cx: &mut TestAppContext) {
    let _locale = crate::services::i18n::LOCALE_LOCK.lock().unwrap();
    let (view, cx, dir) = editor(cx);
    let inherited = Binding::LongPress(openlogi_core::binding::LongPressBinding::new(
        Action::Copy,
        Action::Paste,
    ));
    cx.update(|_, cx| {
        AppState::apply(cx, |state| {
            let context = state.crown_editing_context().unwrap();
            state.commit_crown_input(&context, ButtonId::CrownPress, inherited.clone())
        });
        AppState::apply(cx, |state| {
            state.set_editing_app(Some("org.example.Editor".into()))
        });
    });
    cx.update(|window, cx| {
        view.update(cx, |view, cx| {
            view.control.update(cx, |state, cx| {
                state.set_selected_value(&ButtonId::CrownPress, window, cx);
            });
        });
    });
    draw(cx);
    click(cx, "crown-edit-binding");
    assert!(cx.debug_bounds("crown-long").is_none());
    click(cx, "crown-action-Paste");
    click(cx, "crown-binding-save");
    let saved =
        Config::load_from_path(&dir.path().join(openlogi_core::paths::CONFIG_FILE)).unwrap();
    assert_eq!(
        saved.devices[KEY].per_app_bindings["org.example.Editor"][&ButtonId::CrownPress],
        Action::Paste,
    );
    assert_eq!(
        saved.devices[KEY].bindings[&ButtonId::CrownPress],
        inherited
    );
    click(cx, "crown-edit-binding");
    assert!(cx.debug_bounds("crown-action-Paste-check").is_some());
}
