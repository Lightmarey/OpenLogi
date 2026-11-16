use super::*;
use std::sync::{Arc, Mutex};

#[test]
fn empty_wm_class_represents_no_focused_application() {
    assert_eq!(app_id(String::new()), None);
    assert_eq!(
        app_id("org.example.App".into()).as_deref(),
        Some("org.example.App")
    );
}

#[derive(Clone)]
struct ModifierBridge(Arc<Mutex<(bool, bool, bool, bool)>>);

#[zbus::interface(name = "org.openlogi.Frontmost")]
impl ModifierBridge {
    fn get_held_modifiers(&self) -> (bool, bool, bool, bool) {
        *self.0.lock().unwrap()
    }
}

struct LegacyBridge(String);

#[zbus::interface(name = "org.openlogi.Frontmost")]
impl LegacyBridge {
    fn get_focused_wm_class(&self) -> &str {
        &self.0
    }
}

#[test]
#[ignore = "requires OPENLOGI_MODIFIER_ISOLATED_BUS=1 dbus-run-session"]
fn gnome_modifier_snapshots_survive_extension_restart_and_older_versions() {
    assert_eq!(
        std::env::var("OPENLOGI_MODIFIER_ISOLATED_BUS").as_deref(),
        Ok("1")
    );
    let state = Arc::new(Mutex::new((true, false, true, false)));
    let bridge = Builder::session()
        .unwrap()
        .name("org.openlogi.Frontmost")
        .unwrap()
        .serve_at("/org/openlogi/Frontmost", ModifierBridge(state.clone()))
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(
        crate::held_modifiers(),
        Some(KeyModifiers {
            shift: true,
            control: false,
            option: true,
            command: false,
        }),
        "the native bridge answers independently of readable keyboard nodes"
    );
    *state.lock().unwrap() = (false, false, false, false);
    assert_eq!(crate::held_modifiers(), Some(KeyModifiers::default()));
    bridge.release_name("org.openlogi.Frontmost").unwrap();
    assert_eq!(
        held_modifiers(),
        None,
        "unavailable differs from no held keys"
    );
    let legacy = Builder::session()
        .unwrap()
        .name("org.openlogi.Frontmost")
        .unwrap()
        .serve_at("/org/openlogi/Frontmost", LegacyBridge(String::new()))
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(
        held_modifiers(),
        None,
        "older extensions remain usable for focus"
    );
    legacy.release_name("org.openlogi.Frontmost").unwrap();
    *state.lock().unwrap() = (false, true, false, true);
    bridge.request_name("org.openlogi.Frontmost").unwrap();
    assert_eq!(
        crate::held_modifiers(),
        Some(KeyModifiers {
            shift: false,
            control: true,
            option: false,
            command: true,
        }),
        "a replacement owner supplies its current snapshot"
    );
}
