//! Crown scopes and persistence use the same state owner as other device edits.

use super::*;
use crate::services::ipc::Command;
use openlogi_core::config::ConfigFile;
use openlogi_core::crown::{CrownControl, CrownProfile};

pub(crate) const KEY: &str = "unit:a393cae0";

pub(crate) fn crown_state(
    config: Config,
    persistence: ConfigPersistence,
) -> (AppState, tokio::sync::mpsc::UnboundedReceiver<Command>) {
    let resolver = AssetResolver::new();
    let (commands, mut received) = tokio::sync::mpsc::unbounded_channel();
    let mut inventory = direct_inventory([0xa3, 0x93, 0xca, 0xe0]);
    inventory.paired[0].kind = DeviceKind::Keyboard;
    inventory.paired[0].codename = Some("Craft".into());
    inventory.paired[0].capabilities = Some(Capabilities {
        crown: true,
        ..Capabilities::default()
    });
    let state = AppState::new(Sources {
        inventories: &[inventory],
        persistence,
        ..Sources::in_memory(config, &resolver, commands)
    });
    while received.try_recv().is_ok() {}
    (state, received)
}

#[test]
fn crown_profiles_round_trip_and_stale_scope_cannot_save() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(openlogi_core::paths::CONFIG_FILE);
    let (config, file) = ConfigFile::load_from_path(&path).unwrap();
    let (mut state, mut received) = crown_state(config, ConfigPersistence::UserFile(file));
    let default = state.crown_editing_context().unwrap();
    let mut profile = CrownProfile::default();
    profile.set_enabled(true);
    let _ = state.commit_crown_profile(&default, profile.clone());
    assert_eq!(state.crown_profile(), profile);
    let _ = state.set_editing_app(Some("org.example.Editor".into()));
    let _ = state.commit_crown_profile(&default, CrownProfile::default());
    assert_eq!(state.crown_profile(), profile);
    let context = state.crown_editing_context().unwrap();
    let mut functions = profile.functions().to_vec();
    functions.rotate_left(1);
    functions[0].rename("Editor workspace").unwrap();
    functions[0]
        .set_binding(CrownControl::Right, Binding::Single(Action::Copy))
        .unwrap();
    profile.replace_functions(functions).unwrap();
    let _ = state.commit_crown_profile(&context, profile.clone());
    let saved = Config::load_from_path(&path).unwrap();
    assert_eq!(
        saved.devices[KEY]
            .crown
            .resolve(&saved.apps, Some("org.example.Editor"))
            .1,
        &profile
    );
    assert_eq!(
        saved.devices[KEY].crown.default.functions()[0].name(),
        "Volume"
    );
    let commands: Vec<_> = std::iter::from_fn(|| received.try_recv().ok()).collect();
    assert_eq!(
        commands.len(),
        2,
        "only two accepted saves reload the agent"
    );
    assert!(
        commands
            .iter()
            .all(|command| matches!(command, Command::ReloadConfig(_)))
    );
}

#[test]
fn crown_save_failure_preserves_persisted_profile_and_direct_input_constraints() {
    let (mut state, mut received) = crown_state(
        Config::ephemeral(),
        ConfigPersistence::ReadOnly("read-only".into()),
    );
    let original = state.crown_profile();
    let context = state.crown_editing_context().unwrap();
    let mut changed = original.clone();
    changed.set_enabled(true);
    let _ = state.commit_crown_profile(&context, changed);
    assert_eq!(state.crown_profile(), original);
    assert_eq!(state.config_issue(), Some("read-only"));
    let held = Action::HoldShortcut("Ctrl+L".parse().unwrap());
    let _ = state.commit_crown_input(&context, ButtonId::CrownRight, held.into());
    assert_eq!(
        state.crown_input_binding(ButtonId::CrownRight),
        Binding::Single(Action::None)
    );
    assert!(
        received.try_recv().is_err(),
        "failed or invalid edits never reload"
    );
}
