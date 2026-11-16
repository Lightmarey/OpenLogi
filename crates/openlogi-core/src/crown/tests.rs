use super::*;
use crate::binding::{KeyCombo, LongPressBinding};
use crate::config::{Config, DeviceConfig};

#[test]
fn old_profiles_load_with_native_rotation_and_new_settings_survive_edits() {
    use crate::rotary::{RotarySensitivity, RotarySettings, TactileMode};
    let old = "enabled = true\n[[functions]]\nid = 'custom'\nname = 'Custom'\nicon = 'Volume'\n";
    let mut profile: CrownProfile = toml::from_str(old).unwrap();
    assert_eq!(profile.feel(), TactileMode::Inherit);
    assert_eq!(profile.rotation(), RotarySettings::default());
    profile.set_feel(TactileMode::Smooth);
    let rotation = RotarySettings::default()
        .with_sensitivity(RotarySensitivity::new(150).unwrap())
        .with_inverted(true);
    profile.set_rotation(rotation);
    let mut functions = profile.functions().to_vec();
    functions[0].set_feel(TactileMode::Ratchet);
    functions[0].set_rotation(rotation);
    profile.replace_functions(functions).unwrap();
    let saved = toml::to_string(&profile).unwrap();
    let reloaded: CrownProfile = toml::from_str(&saved).unwrap();
    assert_eq!(reloaded, profile);
    assert_eq!(reloaded.feel(), TactileMode::Smooth);
    assert_eq!(reloaded.functions()[0].feel(), TactileMode::Ratchet);
    toml::from_str::<CrownProfile>(&saved.replace("sensitivity = 150", "sensitivity = 0"))
        .unwrap_err();
}

#[test]
fn feedback_tracks_actions_rather_than_editable_names_and_icons() {
    let mut function = CrownProfile::default().functions()[0].clone();
    assert_eq!(function.feedback(), Some(CrownFeedback::SystemVolume));
    function.rename("画笔大小").unwrap();
    function.set_icon(ActionRingIcon::Monitor);
    assert_eq!(function.feedback(), Some(CrownFeedback::SystemVolume));
    function
        .set_binding(CrownControl::Right, Binding::Single(Action::Copy))
        .unwrap();
    assert_eq!(function.feedback(), None);
    function
        .set_binding(CrownControl::Left, Binding::Single(Action::VolumeUp))
        .unwrap();
    function
        .set_binding(CrownControl::Right, Binding::Single(Action::VolumeDown))
        .unwrap();
    assert_eq!(function.feedback(), Some(CrownFeedback::SystemVolume));
    let function = CrownFunction::new(
        CrownFunctionId::new("volume".into()).unwrap(),
        "Volume",
        ActionRingIcon::Volume,
    )
    .unwrap();
    assert_eq!(function.feedback(), None);
}

#[test]
fn selection_trigger_is_explicit_and_function_edits_preserve_it() {
    let mut profile = CrownProfile::default();
    assert_eq!(profile.selection_trigger(), CrownSelectionTrigger::Press);
    assert!(
        !profile
            .binding_controls()
            .any(|control| control == CrownControl::Press)
    );
    let press_binding = profile.functions()[0].binding(CrownControl::Press);
    profile.set_selection_trigger(CrownSelectionTrigger::TouchTap);
    assert!(
        profile
            .binding_controls()
            .any(|control| control == CrownControl::Press)
    );
    let mut functions = profile.functions().to_vec();
    functions[0].rename("Volume retained").unwrap();
    functions.rotate_left(1);
    profile.replace_functions(functions).unwrap();
    assert_eq!(profile.selection_trigger(), CrownSelectionTrigger::TouchTap);
    assert_eq!(
        profile.functions()[2].binding(CrownControl::Press),
        press_binding
    );
    let encoded = toml::to_string(&profile).unwrap();
    assert_eq!(toml::from_str::<CrownProfile>(&encoded).unwrap(), profile);
    let before = profile.clone();
    assert_eq!(
        profile.replace_functions(vec![]),
        Err(CrownError::FunctionCount)
    );
    assert_eq!(
        profile, before,
        "invalid edits must leave the whole profile intact"
    );
    let missing_trigger = encoded.replace("selection_trigger = \"touch_tap\"\n", "");
    assert_eq!(
        toml::from_str::<CrownProfile>(&missing_trigger)
            .unwrap()
            .selection_trigger(),
        CrownSelectionTrigger::Press
    );
    toml::from_str::<CrownProfile>(&encoded.replace("\"touch_tap\"", "\"tap\"")).unwrap_err();
}

#[test]
fn profile_rejects_empty_duplicate_and_unbounded_functions() {
    let defaults = CrownProfile::default();
    let function = defaults.functions()[0].clone();
    assert_eq!(
        CrownProfile::new(true, vec![]),
        Err(CrownError::FunctionCount)
    );
    assert_eq!(
        CrownProfile::new(true, vec![function.clone(); 2]),
        Err(CrownError::DuplicateId)
    );
    assert_eq!(
        CrownProfile::new(true, vec![function; MAX_FUNCTIONS + 1]),
        Err(CrownError::FunctionCount)
    );
}

#[test]
fn pulse_controls_cannot_leave_keys_held_and_press_retains_real_edges() {
    let mut function = CrownProfile::default().functions()[0].clone();
    let held = Binding::Single(Action::HoldShortcut(
        "Ctrl+L".parse::<KeyCombo>().expect("combo"),
    ));
    for control in CrownControl::ALL {
        let result = function.set_binding(control, held.clone());
        assert_eq!(result.is_ok(), control == CrownControl::Press);
    }
    let pair = Binding::LongPress(LongPressBinding::new(Action::Copy, Action::Paste));
    function
        .set_binding(CrownControl::Press, pair.clone())
        .unwrap();
    assert_eq!(
        function.set_binding(CrownControl::Left, pair),
        Err(CrownError::PulseBinding)
    );
}

#[test]
fn profile_round_trips_with_stable_ids_and_disabled_mode_retains_functions() {
    let mut profile = CrownProfile::default();
    profile.set_enabled(true);
    let mut functions = profile.functions().to_vec();
    let selected = functions[0].id().clone();
    functions[0].rename("音量").expect("valid name");
    functions.rotate_left(1);
    let mut edited = CrownProfile::new(true, functions).expect("valid profile");
    edited.set_enabled(false);
    let encoded = toml::to_string(&edited).expect("encode");
    let decoded: CrownProfile = toml::from_str(&encoded).expect("decode");
    assert_eq!(decoded, edited);
    assert_eq!(decoded.functions()[2].id(), &selected);
    assert_eq!(decoded.functions()[2].name(), "音量");
}

#[test]
fn deserialization_enforces_ids_names_and_lifecycle_constraints() {
    let profile = CrownProfile::default();
    let text = toml::to_string(&profile).expect("encode");
    toml::from_str::<CrownProfile>(&text.replace("id = \"volume\"", "id = \"../volume\""))
        .unwrap_err();
    toml::from_str::<CrownProfile>(&text.replace("name = \"Volume\"", "name = \" \"")).unwrap_err();
    toml::from_str::<CrownProfile>(&text.replace(
        "Left = \"VolumeDown\"",
        "Left = { HoldModifier = \"Shift\" }",
    ))
    .unwrap_err();
}

#[test]
fn old_devices_stay_direct_and_crown_only_app_profiles_are_discoverable() {
    let device: DeviceConfig = toml::from_str("").expect("old config");
    assert!(device.crown.is_default());
    assert!(!toml::to_string(&device).expect("encode").contains("crown"));
    let mut config = Config::default();
    let device = config.devices.entry("craft".into()).or_default();
    let mut profile = CrownProfile::default();
    profile.set_enabled(true);
    device.crown.per_app.insert("exe:zen.exe".into(), profile);
    let (scope, resolved) = device
        .crown
        .resolve(&config.apps, Some(r"c:\zen-v3\zen.exe"));
    assert_eq!(scope, Some("exe:zen.exe"));
    assert!(resolved.is_enabled());
    assert_eq!(
        config.app_profiles("craft").collect::<Vec<_>>(),
        ["exe:zen.exe"]
    );
    assert!(config.has_app_override("craft", r"c:\zen-v3\zen.exe"));
    config.remove_app_profile("craft", "exe:zen.exe");
    assert!(config.app_profiles("craft").next().is_none());
}

#[test]
fn crown_alias_resolution_returns_the_scope_the_runtime_and_editor_share() {
    let source = r#"
schema_version = 12
[apps.editor]
macos = "org.example.Editor"
linux = ["editor", "editor-beta"]
windows = "exe:editor.exe"
[devices.craft.crown.per_app."app:editor"]
enabled = true
functions = [{ id = "volume", name = "Editor volume", icon = "Volume" }]
"#;
    let mut config: Config = toml::from_str(source).unwrap();
    let app = if cfg!(target_os = "windows") {
        r"c:\editor\editor.exe"
    } else if cfg!(target_os = "macos") {
        "org.example.Editor"
    } else {
        "editor-beta"
    };
    let crown = &config.devices["craft"].crown;
    let (scope, profile) = crown.resolve(&config.apps, Some(app));
    assert_eq!(scope, Some("app:editor"));
    assert_eq!(profile.functions()[0].name(), "Editor volume");
    assert!(config.has_app_override("craft", app));
    assert_eq!(
        config.app_profiles("craft").collect::<Vec<_>>(),
        ["app:editor"]
    );
    config
        .devices
        .get_mut("craft")
        .unwrap()
        .crown
        .per_app
        .insert(app.into(), CrownProfile::default());
    assert_eq!(
        config.devices["craft"]
            .crown
            .resolve(&config.apps, Some(app))
            .0,
        Some(app)
    );
}

#[test]
fn direct_crown_tables_enforce_the_same_input_constraints_as_functions() {
    for table in ["bindings", "per_app_bindings.Editor", "gshift_bindings"] {
        for button in ["CrownRight", "CrownPressedLeft"] {
            let source = format!(
                "schema_version = 12\n[devices.craft.{table}]\n{button} = {{ HoldModifier = \"Shift\" }}\n"
            );
            let error = toml::from_str::<Config>(&source).unwrap_err();
            assert!(error.to_string().contains("Crown rotation"), "{error}");
        }
    }
    let source = "schema_version = 12\n[devices.craft.bindings]\nCrownTouch = { HoldModifier = \"Shift\" }\n";
    toml::from_str::<Config>(source).unwrap();
}

#[test]
#[cfg(feature = "fs")]
fn invalid_direct_crown_bindings_fail_file_load_and_cannot_replace_saved_config() {
    use crate::config::ConfigEdit;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        "schema_version = 12\n[devices.craft.bindings]\nCrownRight = { HoldModifier = \"Shift\" }\n",
    )
    .unwrap();
    Config::load_from_path(&path).unwrap_err();
    std::fs::write(&path, "schema_version = 12\n").unwrap();
    let mut edit = ConfigEdit::load(&path).unwrap();
    edit.config_mut().set_binding(
        "craft",
        ButtonId::CrownRight,
        Action::HoldModifier("Shift".parse().unwrap()).into(),
    );
    let before = std::fs::read(&path).unwrap();
    edit.commit().unwrap_err();
    assert_eq!(std::fs::read(&path).unwrap(), before);
}
