use super::*;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

#[test]
fn presentation_contains_embedded_glyphs_and_native_feedback_without_actions() {
    let profile = openlogi_core::crown::CrownProfile::default();
    let encoded = encode(&CrownHud::new(
        &profile,
        profile.functions()[0].id().clone(),
    ))
    .unwrap();
    let value: serde_json::Value = serde_json::from_str(&encoded).unwrap();
    assert_eq!(value["selected"], "volume");
    assert_eq!(value["functions"][0]["feedback"]["kind"], "system_volume");
    assert!(value["functions"][1].get("feedback").is_none());
    assert!(
        value["functions"][0]["svg"]
            .as_str()
            .unwrap()
            .contains("<svg")
    );
    assert!(!encoded.contains("VolumeDown"));
    assert!(!encoded.contains("bindings"));
    assert_eq!(value.as_object().unwrap().len(), 3);
}

#[test]
fn measured_parameters_encode_their_actual_text_range_and_availability() {
    use openlogi_core::crown::CrownFeedback;
    use openlogi_core::parameter::{ParameterState, ParameterValue};
    let profile = openlogi_core::crown::CrownProfile::default();
    let mut hud = CrownHud::new(&profile, profile.functions()[0].id().clone());
    hud.functions[0].feedback = Some(CrownFeedback::Parameter(ParameterState::Available(
        ParameterValue::new(250, 0, 1000, "25%".into()).unwrap(),
    )));
    let encoded: serde_json::Value = serde_json::from_str(&encode(&hud).unwrap()).unwrap();
    assert_eq!(encoded["functions"][0]["feedback"]["fraction"], 0.25);
    assert_eq!(encoded["functions"][0]["feedback"]["text"], "25%");
    hud.functions[0].feedback = Some(CrownFeedback::Parameter(ParameterState::Unavailable));
    let encoded: serde_json::Value = serde_json::from_str(&encode(&hud).unwrap()).unwrap();
    assert_eq!(encoded["functions"][0]["feedback"]["kind"], "unavailable");
}

#[test]
#[ignore = "requires the native GNOME Crown HUD test runner"]
fn crown_hud_native_shell() {
    use std::io::Read as _;
    let advance = || {
        std::io::stdin()
            .read_exact(&mut [0])
            .expect("native harness acknowledges the rendered stage");
    };
    let presenter = Presenter::start().expect("presentation worker");
    let profile = openlogi_core::crown::CrownProfile::default();
    presenter.show(Some(CrownHud::new(
        &profile,
        profile.functions()[0].id().clone(),
    )));
    advance();
    presenter.show(Some(CrownHud::new(
        &profile,
        profile.functions()[1].id().clone(),
    )));
    advance();
    presenter.show(None);
    advance();
    presenter.show(Some(CrownHud::new(
        &profile,
        profile.functions()[2].id().clone(),
    )));
    advance();
    let mut parameter = CrownHud::new(&profile, profile.functions()[0].id().clone());
    parameter.functions[0].name = "Brightness".into();
    parameter.functions[0].feedback = Some(openlogi_core::crown::CrownFeedback::Parameter(
        openlogi_core::parameter::ParameterState::Available(
            openlogi_core::parameter::ParameterValue::new(250, 0, 1000, "25%".into())
                .expect("measured range"),
        ),
    ));
    presenter.show(Some(parameter.clone()));
    advance();
    parameter.functions[0].feedback = Some(openlogi_core::crown::CrownFeedback::Parameter(
        openlogi_core::parameter::ParameterState::Unavailable,
    ));
    presenter.show(Some(parameter));
    advance();
    drop(presenter);
}

#[derive(Clone)]
struct Shell {
    version: Arc<AtomicU32>,
    shown: Arc<Mutex<Vec<Option<String>>>>,
}
#[zbus::interface(name = "org.openlogi.CrownHud")]
impl Shell {
    fn get_version(&self) -> u32 {
        self.version.load(Ordering::SeqCst)
    }
    fn show(&self, payload: String) {
        self.shown.lock().unwrap().push(Some(payload));
    }
    fn hide(&self) {
        self.shown.lock().unwrap().push(None);
    }
}
async fn until(predicate: impl Fn() -> bool) {
    tokio::time::timeout(Duration::from_secs(6), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("bridge converges within its retry window");
}

/// The real worker against real D-Bus ownership changes, with no Shell process.
#[tokio::test]
#[ignore = "requires OPENLOGI_CROWN_ISOLATED_BUS=1 dbus-run-session"]
async fn bridge_recovers_late_enable_restart_disable_and_version_mismatch() {
    assert_eq!(
        std::env::var("OPENLOGI_CROWN_ISOLATED_BUS").as_deref(),
        Ok("1")
    );
    let shell = Shell {
        version: Arc::new(AtomicU32::new(BRIDGE_VERSION)),
        shown: Arc::default(),
    };
    let (state, updates) = watch::channel(None);
    let worker = tokio::spawn(present(updates));
    let profile = openlogi_core::crown::CrownProfile::default();
    state.send_replace(Some(CrownHud::new(
        &profile,
        profile.functions()[1].id().clone(),
    )));
    tokio::time::sleep(Duration::from_millis(50)).await;
    let first = Builder::session()
        .unwrap()
        .name(SERVICE)
        .unwrap()
        .serve_at(PATH, shell.clone())
        .unwrap()
        .build()
        .await
        .unwrap();
    until(|| {
        shell
            .shown
            .lock()
            .unwrap()
            .iter()
            .flatten()
            .any(|payload| payload.contains("\"selected\":\"workspaces\""))
    })
    .await;
    first.release_name(SERVICE).await.unwrap();
    shell.shown.lock().unwrap().clear();
    let replacement = Builder::session()
        .unwrap()
        .name(SERVICE)
        .unwrap()
        .serve_at(PATH, shell.clone())
        .unwrap()
        .build()
        .await
        .unwrap();
    until(|| !shell.shown.lock().unwrap().is_empty()).await;
    assert!(
        shell
            .shown
            .lock()
            .unwrap()
            .iter()
            .flatten()
            .all(|payload| payload.contains("\"selected\":\"workspaces\""))
    );
    replacement.release_name(SERVICE).await.unwrap();
    state.send_replace(None);
    tokio::time::sleep(Duration::from_millis(1100)).await;
    shell.version.store(999, Ordering::SeqCst);
    shell.shown.lock().unwrap().clear();
    replacement.request_name(SERVICE).await.unwrap();
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert!(
        shell.shown.lock().unwrap().is_empty(),
        "version mismatch never presents a stale HUD"
    );
    shell.version.store(BRIDGE_VERSION, Ordering::SeqCst);
    until(|| !shell.shown.lock().unwrap().is_empty()).await;
    assert!(
        shell.shown.lock().unwrap().iter().all(Option::is_none),
        "expiry during disconnection remains hidden after reconnect"
    );
    drop(state);
    tokio::time::timeout(Duration::from_secs(2), worker)
        .await
        .unwrap()
        .unwrap();
}
