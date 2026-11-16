//! Desktop modifier snapshots, independent of keyboard-event permissions.

use evdev::{Device, KeyCode};

use crate::KeyModifiers;

pub(crate) fn held_modifiers() -> Option<KeyModifiers> {
    super::foreground::gnome_shell::held_modifiers()
        .or_else(|| {
            super::probe::Probe::connect()?
                .state()
                .map(|state| state.modifiers)
        })
        .or_else(|| {
            evdev::enumerate()
                .filter_map(|(_, device)| keyboard_modifiers(&device))
                .reduce(|a, b| KeyModifiers {
                    shift: a.shift || b.shift,
                    control: a.control || b.control,
                    option: a.option || b.option,
                    command: a.command || b.command,
                })
        })
}

fn keyboard_modifiers(device: &Device) -> Option<KeyModifiers> {
    let supported = device.supported_keys()?;
    if ![
        KeyCode::KEY_LEFTSHIFT,
        KeyCode::KEY_RIGHTSHIFT,
        KeyCode::KEY_LEFTCTRL,
        KeyCode::KEY_RIGHTCTRL,
        KeyCode::KEY_LEFTALT,
        KeyCode::KEY_RIGHTALT,
        KeyCode::KEY_LEFTMETA,
        KeyCode::KEY_RIGHTMETA,
    ]
    .iter()
    .any(|key| supported.contains(*key))
    {
        return None;
    }
    let held = device.get_key_state().ok()?;
    Some(KeyModifiers {
        shift: held.contains(KeyCode::KEY_LEFTSHIFT) || held.contains(KeyCode::KEY_RIGHTSHIFT),
        control: held.contains(KeyCode::KEY_LEFTCTRL) || held.contains(KeyCode::KEY_RIGHTCTRL),
        option: held.contains(KeyCode::KEY_LEFTALT) || held.contains(KeyCode::KEY_RIGHTALT),
        command: held.contains(KeyCode::KEY_LEFTMETA) || held.contains(KeyCode::KEY_RIGHTMETA),
    })
}

#[cfg(test)]
mod tests;
