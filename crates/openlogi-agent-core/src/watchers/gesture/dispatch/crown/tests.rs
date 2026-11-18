use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use openlogi_core::binding::Action;
use openlogi_core::config::{ThumbwheelSensitivity, VerticalScrollSensitivity};
use openlogi_core::crown::CrownProfile;

use crate::runtime::scroll::{ScrollPreferences, ScrollRuntime};
use crate::runtime::testing::{ButtonRuntimeEvent, EndReason, test_dispatcher};

use super::*;

fn test_outputs(
    applications: Arc<crate::applications::ApplicationHost>,
) -> (
    GestureOutputs,
    crate::runtime::testing::ButtonRuntimeOwner,
    std::sync::mpsc::Receiver<ButtonRuntimeEvent>,
    ScrollRuntime,
) {
    let (actions, buttons, events) = test_dispatcher();
    let scroll = ScrollRuntime::spawn(Arc::new(ScrollPreferences::new(
        false,
        VerticalScrollSensitivity::DEFAULT,
    )))
    .unwrap();
    let display = Arc::new(crate::crown::CrownDisplay::with_applications(Arc::clone(
        &applications,
    )));
    let outputs = GestureOutputs::new(
        actions,
        scroll.input(),
        Arc::default(),
        display,
        applications,
    );
    (outputs, buttons, events, scroll)
}

fn plan(scope: Option<&str>) -> DispatchPlan {
    let mut profile = CrownProfile::default();
    profile.set_enabled(true);
    DispatchPlan {
        config_key: "craft".into(),
        // Direct bindings must never run alongside the selected function.
        bindings: BTreeMap::from([(ButtonId::CrownRight, Action::Copy.into())]),
        gesture_bindings: BTreeMap::new(),
        side_gesture_bindings: BTreeMap::new(),
        thumbwheel_sensitivity: ThumbwheelSensitivity::DEFAULT,
        pointer: None,
        invert_wheel: false,
        momentum: openlogi_core::config::MomentumScrolling::default(),
        shift_bindings: BTreeMap::new(),
        crown: Some(CrownDispatch {
            scope: scope.map(str::to_owned),
            profile,
            application: None,
        }),
    }
}

fn touch_plan(scope: Option<&str>) -> DispatchPlan {
    let mut plan = plan(scope);
    plan.crown
        .as_mut()
        .unwrap()
        .profile
        .set_selection_trigger(CrownSelectionTrigger::TouchTap);
    plan
}

#[tokio::test]
async fn crown_motion_uses_the_confirmation_boundary_in_the_ordered_input_stream() {
    let (outputs, mut buttons, events, mut scroll) = test_outputs(Arc::default());
    let mut dispatcher = super::super::super::InputDispatcher::new(outputs);
    let parent = HidppSessionId::new("craft");
    let policy = plan(None);
    let motion = CapturedInput::CrownRotation(openlogi_hid::crown::CrownMotion::new(2, 0, true));
    // Movement admitted before the confirmation retains the armed ratchet policy.
    dispatcher.dispatch(&parent, &policy, motion);
    dispatcher.dispatch(
        &parent,
        &policy,
        CapturedInput::CrownModeApplied {
            mode: openlogi_core::rotary::TactileMode::Smooth,
            request: 0,
        },
    );
    dispatcher.dispatch(&parent, &policy, motion);
    let completed = (0..4)
        .map(|_| events.recv_timeout(Duration::from_secs(1)).unwrap())
        .collect::<Vec<_>>();
    dispatcher.dispatch(
        &parent,
        &policy,
        CapturedInput::CrownModeApplied {
            mode: openlogi_core::rotary::TactileMode::Ratchet,
            request: 0,
        },
    );
    dispatcher.dispatch(&parent, &policy, motion);
    dispatcher.cancel_session(&parent);
    dispatcher.retire_crown_mode(&parent);
    assert!(dispatcher.crown_confirmed.is_empty());
    assert!(buttons.shutdown());
    assert!(events.try_iter().next().is_none());
    let events = completed;
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event,
        ButtonRuntimeEvent::Started(press) if press.start_action() == Some(&Action::VolumeUp)))
            .count(),
        2
    );
    scroll.shutdown();
}

#[tokio::test]
async fn a_replaced_function_session_uses_the_raw_current_press_state() {
    let (outputs, mut buttons, events, mut scroll) = test_outputs(Arc::default());
    let mut dispatcher = super::super::super::InputDispatcher::new(outputs);
    let parent = HidppSessionId::new("craft");
    let mut policy = plan(Some("editor"));
    let profile = &mut policy.crown.as_mut().unwrap().profile;
    let mut functions = profile.functions().to_vec();
    functions[0]
        .set_binding(CrownControl::PressedRight, Action::NextTab.into())
        .unwrap();
    profile.replace_functions(functions).unwrap();
    let mut motion = openlogi_hid::crown::CrownMotion::new(6, 1, true);
    motion.pressed = true;
    motion.touched = true;
    dispatcher.dispatch(&parent, &policy, CapturedInput::CrownRotation(motion));
    let ButtonRuntimeEvent::Started(press) = events.recv_timeout(Duration::from_secs(1)).unwrap()
    else {
        panic!("current held rotation");
    };
    assert_eq!(press.start_action(), Some(&Action::NextTab));
    events.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(buttons.shutdown());
    scroll.shutdown();
}

#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "the ordered Craft stream and local fallback must share one capture and action runtime"
)]
async fn crown_application_inputs_preserve_motion_edges_and_restore_local_actions() {
    use crate::applications::tests::{context, next, provider, tools};
    use crate::applications::{
        ApplicationHost, ApplicationServer, ControlScope, crown_profile_for,
    };
    use openlogi_sdk::{ControlInput, RotationPhase, RotationResolution, ServerMessage};
    use openlogi_sdk::{client::Client, transport::Endpoint};
    let directory = tempfile::tempdir().unwrap();
    let endpoint = Endpoint::at(directory.path().join("sdk.sock"));
    let host = Arc::new(ApplicationHost::default());
    let _server = ApplicationServer::start_at(Arc::clone(&host), &endpoint).unwrap();
    let provider = provider("parameters");
    let mut client = Client::connect_at(&endpoint, provider.clone())
        .await
        .unwrap();
    client.set_context(1, Some(tools(50))).await.unwrap();
    let context = context(&host, &provider.id, 1).await;
    let (outputs, mut buttons, events, mut scroll) = test_outputs(Arc::clone(&host));
    let display = Arc::clone(&outputs.crown);
    let mut dispatcher = super::super::super::InputDispatcher::new(outputs);
    let parent = HidppSessionId::new("craft");
    let mut policy = touch_plan(Some("editor"));
    let fallback = policy.clone();
    dispatcher.dispatch(&parent, &fallback, CapturedInput::CrownTap);
    assert_eq!(
        display.observe(0).await.hud.unwrap().selected.as_ref(),
        "workspaces"
    );
    dispatcher.cancel_session(&parent);
    let crown = policy.crown.as_mut().unwrap();
    crown.profile = crown_profile_for(&crown.profile, &context).unwrap();
    crown.application = Some(context.clone());
    host.reconcile(
        [(
            ControlScope::new("craft".into(), Some("editor".into())),
            context,
        )]
        .into(),
    );
    dispatcher.dispatch(
        &parent,
        &policy,
        CapturedInput::CrownReady(openlogi_sdk::ControlCapabilities {
            kind: openlogi_sdk::ControlKind::Rotary,
            features: openlogi_sdk::ControlFeatures::all(),
            resolution: RotationResolution::new(1, 4),
        }),
    );
    dispatcher.dispatch(
        &parent,
        &policy,
        CapturedInput::ButtonDown(ButtonId::CrownTouch),
    );
    assert!(matches!(
        next(&mut client).await,
        ServerMessage::Activated(_)
    ));
    assert!(matches!(
        next(&mut client).await,
        ServerMessage::Input {
            event: ControlInput::Touch { down: true },
            ..
        }
    ));
    dispatcher.dispatch(
        &parent,
        &policy,
        CapturedInput::ButtonDown(ButtonId::CrownPress),
    );
    assert!(matches!(
        next(&mut client).await,
        ServerMessage::Input {
            event: ControlInput::Press {
                down: true,
                reserved: false
            },
            ..
        }
    ));
    let mut motion = openlogi_hid::crown::CrownMotion::new(2, 0, false);
    motion.resolution = RotationResolution::new(1, 4);
    motion.phase = openlogi_hid::crown::RotationState::Start;
    motion.pressed = true;
    dispatcher.dispatch(&parent, &policy, CapturedInput::CrownRotation(motion));
    let ServerMessage::Input {
        event: ControlInput::Rotation(rotation),
        ..
    } = next(&mut client).await
    else {
        panic!("raw motion");
    };
    assert_eq!(
        (
            rotation.amount,
            rotation.raw,
            rotation.detents,
            rotation.phase,
            rotation.pressed
        ),
        (0.5, 2, 0, RotationPhase::Start, true)
    );
    motion.slots = 1;
    motion.phase = openlogi_hid::crown::RotationState::Stop;
    dispatcher.dispatch(&parent, &policy, CapturedInput::CrownRotation(motion));
    dispatcher.dispatch(
        &parent,
        &policy,
        CapturedInput::ButtonUp(ButtonId::CrownPress),
    );
    assert!(
        matches!(next(&mut client).await, ServerMessage::Input { event: ControlInput::Rotation(rotation), .. } if rotation.raw == 1 && rotation.phase == RotationPhase::Stop)
    );
    assert!(matches!(
        next(&mut client).await,
        ServerMessage::Input {
            event: ControlInput::Press { down: false, .. },
            ..
        }
    ));
    dispatcher.dispatch(&parent, &policy, CapturedInput::CrownTap);
    assert!(matches!(
        next(&mut client).await,
        ServerMessage::Input {
            event: ControlInput::Tap { reserved: true },
            ..
        }
    ));
    assert!(
        matches!(next(&mut client).await, ServerMessage::Selected { tool, .. } if tool.as_ref() == "command")
    );
    assert_eq!(
        display.observe(0).await.hud.unwrap().selected.as_ref(),
        "command"
    );
    dispatcher.cancel_session(&parent);
    assert!(matches!(
        next(&mut client).await,
        ServerMessage::Deactivated { .. }
    ));
    dispatcher.dispatch(
        &parent,
        &fallback,
        CapturedInput::CrownRotation(openlogi_hid::crown::CrownMotion::new(6, 1, true)),
    );
    let ButtonRuntimeEvent::Started(press) = events.recv_timeout(Duration::from_secs(1)).unwrap()
    else {
        panic!("local fallback");
    };
    assert_eq!(press.start_action(), Some(&Action::NextDesktop));
    assert!(matches!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        ButtonRuntimeEvent::Ended { .. }
    ));
    assert!(buttons.shutdown());
    assert!(events.try_iter().next().is_none());
    scroll.shutdown();
}

#[tokio::test]
async fn press_selects_on_release_without_firing_a_binding_and_rotation_consumes_selection() {
    let (outputs, mut buttons, events, mut scroll) = test_outputs(Arc::default());
    let display = Arc::clone(&outputs.crown);
    let mut dispatcher = super::super::super::InputDispatcher::new(outputs);
    let session = HidppSessionId::new("craft");
    let mut policy = plan(None);
    let profile = &mut policy.crown.as_mut().unwrap().profile;
    let mut functions = profile.functions().to_vec();
    functions[1]
        .set_binding(CrownControl::PressedRight, Action::NextTab.into())
        .unwrap();
    *profile = CrownProfile::new(true, functions).unwrap();
    for button in [ButtonId::CrownTouch, ButtonId::CrownPress] {
        dispatcher.dispatch(&session, &policy, CapturedInput::ButtonDown(button));
    }
    assert_eq!(
        display.observe(0).await.hud.unwrap().selected.as_ref(),
        "volume"
    );
    dispatcher.dispatch(
        &session,
        &policy,
        CapturedInput::ButtonUp(ButtonId::CrownPress),
    );
    assert_eq!(
        display.observe(0).await.hud.unwrap().selected.as_ref(),
        "workspaces"
    );
    assert!(
        events.try_recv().is_err(),
        "the stored Mute press binding is reserved for selection"
    );
    dispatcher.dispatch(&session, &policy, CapturedInput::CrownTap);
    assert_eq!(
        display.observe(0).await.hud.unwrap().selected.as_ref(),
        "workspaces"
    );
    for input in [
        CapturedInput::ButtonDown(ButtonId::CrownPress),
        CapturedInput::CrownRotation(openlogi_hid::crown::CrownMotion::new(6, 1, true)),
        CapturedInput::ButtonUp(ButtonId::CrownPress),
    ] {
        dispatcher.dispatch(&session, &policy, input);
    }
    assert_eq!(
        display.observe(0).await.hud.unwrap().selected.as_ref(),
        "workspaces"
    );
    let ButtonRuntimeEvent::Started(press) = events.recv_timeout(Duration::from_secs(1)).unwrap()
    else {
        panic!("pressed rotation started");
    };
    assert_eq!(press.start_action(), Some(&Action::NextTab));
    assert!(matches!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        ButtonRuntimeEvent::Ended {
            reason: EndReason::Released,
            ..
        }
    ));
    assert!(
        events.try_recv().is_err(),
        "pressed rotation must not switch or run a press action"
    );
    for input in [
        CapturedInput::ButtonDown(ButtonId::CrownPress),
        CapturedInput::CrownRotation(openlogi_hid::crown::CrownMotion::new(1, 0, true)),
        CapturedInput::ButtonUp(ButtonId::CrownPress),
    ] {
        dispatcher.dispatch(&session, &policy, input);
    }
    assert_eq!(
        display.observe(0).await.hud.unwrap().selected.as_ref(),
        "workspaces"
    );
    assert!(
        events.try_recv().is_err(),
        "sub-detent movement consumes selection without expanding a pulse"
    );
    dispatcher.cancel_session(&session);
    assert!(buttons.shutdown());
    scroll.shutdown();
}

#[tokio::test]
async fn tap_selects_once_rotation_uses_selected_function_and_profiles_remember_separately() {
    let (outputs, mut buttons, events, mut scroll) = test_outputs(Arc::default());
    let display = Arc::clone(&outputs.crown);
    let mut dispatcher = super::super::super::InputDispatcher::new(outputs);
    let session = HidppSessionId::new("craft");
    let default = touch_plan(None);
    dispatcher.dispatch(
        &session,
        &default,
        CapturedInput::ButtonDown(ButtonId::CrownTouch),
    );
    let observed = display.observe(0).await;
    assert_eq!(observed.hud.unwrap().selected.as_ref(), "volume");
    dispatcher.dispatch(&session, &default, CapturedInput::CrownTap);
    dispatcher.dispatch(
        &session,
        &default,
        CapturedInput::CrownRotation(openlogi_hid::crown::CrownMotion::new(1, 1, true)),
    );
    let ButtonRuntimeEvent::Started(press) = events.recv_timeout(Duration::from_secs(1)).unwrap()
    else {
        panic!("pulse started");
    };
    assert_eq!(press.start_action(), Some(&Action::NextDesktop));
    assert!(matches!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        ButtonRuntimeEvent::Ended {
            reason: EndReason::Released,
            ..
        }
    ));
    assert!(
        events.try_recv().is_err(),
        "direct Copy must not fire as well"
    );
    dispatcher.cancel_session(&session);
    let zen = touch_plan(Some("zen"));
    dispatcher.dispatch(
        &session,
        &zen,
        CapturedInput::ButtonDown(ButtonId::CrownTouch),
    );
    assert_eq!(
        display.observe(0).await.hud.unwrap().selected.as_ref(),
        "volume"
    );
    dispatcher.cancel_session(&session);
    dispatcher.dispatch(
        &session,
        &default,
        CapturedInput::ButtonDown(ButtonId::CrownTouch),
    );
    assert_eq!(
        display.observe(0).await.hud.unwrap().selected.as_ref(),
        "workspaces"
    );
    dispatcher.cancel_session(&session);
    assert!(buttons.shutdown());
    scroll.shutdown();
}

#[tokio::test]
async fn press_release_is_balanced_and_stale_cancel_cannot_hide_a_replacement() {
    let (outputs, mut buttons, events, mut scroll) = test_outputs(Arc::default());
    let display = Arc::clone(&outputs.crown);
    let mut dispatcher = CrownFunctions::default();
    let old = HidppSessionId::new("craft");
    let current = HidppSessionId::new("craft");
    let policy = touch_plan(None);
    dispatcher.dispatch(
        &old,
        &policy,
        CapturedInput::ButtonDown(ButtonId::CrownPress),
        KeyModifiers::default(),
        &outputs,
    );
    let ButtonRuntimeEvent::Started(press) = events.recv_timeout(Duration::from_secs(1)).unwrap()
    else {
        panic!("press started");
    };
    assert_eq!(press.start_action(), Some(&Action::MuteVolume));
    dispatcher.dispatch(
        &old,
        &policy,
        CapturedInput::CrownTap,
        KeyModifiers::default(),
        &outputs,
    );
    assert_eq!(
        display.observe(0).await.hud.unwrap().selected.as_ref(),
        "volume",
        "pressed Crown cannot tap-select"
    );
    dispatcher.dispatch(
        &old,
        &policy,
        CapturedInput::ButtonUp(ButtonId::CrownPress),
        KeyModifiers::default(),
        &outputs,
    );
    let ButtonRuntimeEvent::Ended {
        press: released,
        reason: EndReason::Released,
    } = events.recv_timeout(Duration::from_secs(1)).unwrap()
    else {
        panic!("released");
    };
    assert_eq!(released.token(), press.token());
    dispatcher.dispatch(
        &current,
        &policy,
        CapturedInput::ButtonDown(ButtonId::CrownTouch),
        KeyModifiers::default(),
        &outputs,
    );
    dispatcher.cancel(&old, &outputs);
    assert!(display.observe(0).await.hud.is_some());
    dispatcher.cancel(&current, &outputs);
    assert!(display.observe(0).await.hud.is_none());
    assert!(buttons.shutdown());
    scroll.shutdown();
}

#[tokio::test]
async fn crown_thresholds_and_held_output_use_balanced_upstream_lifecycles() {
    use crate::runtime::testing::CancelReason;
    use openlogi_core::binding::{Binding, LongPressBinding};
    let (outputs, mut buttons, events, mut scroll) = test_outputs(Arc::default());
    let mut dispatcher = CrownFunctions::default();
    let mut policy = touch_plan(None);
    let profile = &mut policy.crown.as_mut().unwrap().profile;
    let mut functions = profile.functions().to_vec();
    let held = Action::HoldShortcut("Ctrl+L".parse().unwrap());
    functions[0]
        .set_binding(
            CrownControl::Press,
            Binding::LongPress(LongPressBinding::new(Action::Copy, held.clone())),
        )
        .unwrap();
    profile.replace_functions(functions).unwrap();
    let session = HidppSessionId::new("craft");
    dispatcher.dispatch(
        &session,
        &policy,
        CapturedInput::ButtonDown(ButtonId::CrownPress),
        KeyModifiers::default(),
        &outputs,
    );
    let ButtonRuntimeEvent::Started(short) = events.recv_timeout(Duration::from_secs(1)).unwrap()
    else {
        panic!("press starts");
    };
    assert_eq!(short.start_action(), None);
    dispatcher.dispatch(
        &session,
        &policy,
        CapturedInput::ButtonUp(ButtonId::CrownPress),
        KeyModifiers::default(),
        &outputs,
    );
    let ButtonRuntimeEvent::Triggered { action, .. } =
        events.recv_timeout(Duration::from_secs(1)).unwrap()
    else {
        panic!("short action");
    };
    assert_eq!(action, Action::Copy);
    assert!(matches!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        ButtonRuntimeEvent::Ended {
            reason: EndReason::Released,
            ..
        }
    ));
    dispatcher.dispatch(
        &session,
        &policy,
        CapturedInput::ButtonDown(ButtonId::CrownPress),
        KeyModifiers::default(),
        &outputs,
    );
    assert!(matches!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        ButtonRuntimeEvent::Started(_)
    ));
    let ButtonRuntimeEvent::Triggered { action, .. } =
        events.recv_timeout(Duration::from_secs(2)).unwrap()
    else {
        panic!("threshold action");
    };
    assert_eq!(action, held);
    dispatcher.cancel(&session, &outputs);
    assert!(matches!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        ButtonRuntimeEvent::Ended {
            reason: EndReason::Canceled(CancelReason::SourceEnded),
            ..
        }
    ));
    assert!(buttons.shutdown());
    scroll.shutdown();
}

#[tokio::test]
async fn shift_selection_moves_backwards_and_keeps_press_time_direction() {
    use openlogi_core::config::KeyModifiers;

    let (outputs, mut buttons, _events, mut scroll) = test_outputs(Arc::default());
    let display = Arc::clone(&outputs.crown);
    let mut dispatcher = super::super::super::InputDispatcher::new(outputs);
    let session = HidppSessionId::new("craft");
    let policy = plan(None);
    let shift = KeyModifiers {
        shift: true,
        ..KeyModifiers::default()
    };
    for expected in ["tabs", "workspaces", "volume"] {
        dispatcher.dispatch_with_modifiers(
            &session,
            &policy,
            CapturedInput::ButtonDown(ButtonId::CrownPress),
            shift,
        );
        // Releasing Shift before Crown retains the direction chosen on press.
        dispatcher.dispatch(
            &session,
            &policy,
            CapturedInput::ButtonUp(ButtonId::CrownPress),
        );
        assert_eq!(
            display.observe(0).await.hud.unwrap().selected.as_ref(),
            expected
        );
    }
    let policy = touch_plan(None);
    dispatcher.dispatch_with_modifiers(&session, &policy, CapturedInput::CrownTap, shift);
    assert_eq!(
        display.observe(0).await.hud.unwrap().selected.as_ref(),
        "tabs"
    );
    dispatcher.dispatch(&session, &policy, CapturedInput::CrownTap);
    assert_eq!(
        display.observe(0).await.hud.unwrap().selected.as_ref(),
        "volume"
    );
    dispatcher.cancel_session(&session);
    assert!(buttons.shutdown());
    scroll.shutdown();
}

#[tokio::test]
async fn starter_functions_dispatch_real_rotation_actions() {
    let (outputs, mut buttons, events, mut scroll) = test_outputs(Arc::default());
    let mut dispatcher = super::super::super::InputDispatcher::new(outputs);
    let session = HidppSessionId::new("craft");
    let policy = plan(None);
    for (left, right) in [
        (Action::VolumeDown, Action::VolumeUp),
        (Action::PreviousDesktop, Action::NextDesktop),
        (Action::PrevTab, Action::NextTab),
    ] {
        for (direction, expected) in [(-1, left), (1, right)] {
            dispatcher.dispatch(
                &session,
                &policy,
                CapturedInput::CrownRotation(openlogi_hid::crown::CrownMotion::new(
                    direction * 6,
                    direction,
                    true,
                )),
            );
            let ButtonRuntimeEvent::Started(press) =
                events.recv_timeout(Duration::from_secs(1)).unwrap()
            else {
                panic!("rotation action started");
            };
            assert_eq!(press.start_action(), Some(&expected));
            assert!(matches!(
                events.recv_timeout(Duration::from_secs(1)).unwrap(),
                ButtonRuntimeEvent::Ended { .. }
            ));
        }
        dispatcher.dispatch(
            &session,
            &policy,
            CapturedInput::ButtonDown(ButtonId::CrownPress),
        );
        dispatcher.dispatch(
            &session,
            &policy,
            CapturedInput::ButtonUp(ButtonId::CrownPress),
        );
    }
    dispatcher.cancel_session(&session);
    assert!(buttons.shutdown());
    scroll.shutdown();
}

#[tokio::test]
async fn crown_fractional_steps_follow_speed_and_inversion_and_expire_with_the_function() {
    use openlogi_core::rotary::{RotarySensitivity, RotarySettings};
    let (outputs, mut buttons, events, mut scroll) = test_outputs(Arc::default());
    let mut dispatcher = super::super::super::InputDispatcher::new(outputs);
    let session = HidppSessionId::new("craft");
    let mut policy = plan(None);
    let profile = &mut policy.crown.as_mut().unwrap().profile;
    let mut functions = profile.functions().to_vec();
    functions[0]
        .set_binding(CrownControl::Left, Action::Copy.into())
        .unwrap();
    functions[0].set_rotation(
        RotarySettings::default()
            .with_sensitivity(RotarySensitivity::new(50).unwrap())
            .with_inverted(true),
    );
    profile.replace_functions(functions).unwrap();
    let movement = CapturedInput::CrownRotation(openlogi_hid::crown::CrownMotion::new(6, 1, true));
    dispatcher.dispatch(&session, &policy, movement);
    dispatcher.dispatch(&session, &policy, movement);
    let ButtonRuntimeEvent::Started(press) = events.recv_timeout(Duration::from_secs(1)).unwrap()
    else {
        panic!("two half steps produce one action");
    };
    assert_eq!(press.start_action(), Some(&Action::Copy));
    assert!(matches!(
        events.recv_timeout(Duration::from_secs(1)).unwrap(),
        ButtonRuntimeEvent::Ended { .. }
    ));
    dispatcher.dispatch(&session, &policy, movement);
    for _ in 0..3 {
        dispatcher.dispatch(
            &session,
            &policy,
            CapturedInput::ButtonDown(ButtonId::CrownPress),
        );
        dispatcher.dispatch(
            &session,
            &policy,
            CapturedInput::ButtonUp(ButtonId::CrownPress),
        );
    }
    dispatcher.dispatch(&session, &policy, movement);
    dispatcher.cancel_session(&session);
    assert!(buttons.shutdown());
    assert!(
        events.try_recv().is_err(),
        "a prior function's half step cannot fire after returning"
    );
    assert!(dispatcher.crown_steps.is_empty());
    scroll.shutdown();
}

#[tokio::test]
async fn direct_crown_rotation_honors_gshift() {
    let (outputs, mut buttons, events, mut scroll) = test_outputs(Arc::default());
    let mut dispatcher = super::super::super::InputDispatcher::new(outputs);
    let parent = HidppSessionId::new("craft");
    let mut policy = plan(None);
    policy.crown.as_mut().unwrap().profile.set_enabled(false);
    policy
        .bindings
        .insert(ButtonId::CrownTouch, Action::GShift.into());
    policy
        .bindings
        .insert(ButtonId::CrownRight, Action::Copy.into());
    policy
        .shift_bindings
        .insert(ButtonId::CrownRight, Action::Paste.into());
    dispatcher.dispatch(
        &parent,
        &policy,
        CapturedInput::ButtonDown(ButtonId::CrownTouch),
    );
    let mut motion = openlogi_hid::crown::CrownMotion::new(6, 1, true);
    motion.touched = true;
    dispatcher.dispatch(&parent, &policy, CapturedInput::CrownRotation(motion));
    let ButtonRuntimeEvent::Started(press) = events.recv_timeout(Duration::from_secs(1)).unwrap()
    else {
        panic!("shifted rotation action");
    };
    let actual = press.start_action().cloned();
    dispatcher.cancel_session(&parent);
    assert!(buttons.shutdown());
    scroll.shutdown();
    assert_eq!(actual, Some(Action::Paste));
}

#[tokio::test]
async fn direct_crown_fractional_steps_do_not_cross_binding_layers() {
    use openlogi_core::rotary::{RotarySensitivity, RotarySettings};
    let (outputs, mut buttons, events, mut scroll) = test_outputs(Arc::default());
    let mut dispatcher = super::super::super::InputDispatcher::new(outputs);
    let parent = HidppSessionId::new("craft");
    let mut policy = plan(None);
    let profile = &mut policy.crown.as_mut().unwrap().profile;
    profile.set_enabled(false);
    profile.set_rotation(
        RotarySettings::default().with_sensitivity(RotarySensitivity::new(50).unwrap()),
    );
    policy
        .bindings
        .insert(ButtonId::CrownTouch, Action::GShift.into());
    policy
        .bindings
        .insert(ButtonId::CrownRight, Action::Copy.into());
    policy
        .shift_bindings
        .insert(ButtonId::CrownRight, Action::Paste.into());
    dispatcher.dispatch(
        &parent,
        &policy,
        CapturedInput::CrownRotation(openlogi_hid::crown::CrownMotion::new(6, 1, true)),
    );
    dispatcher.dispatch(
        &parent,
        &policy,
        CapturedInput::ButtonDown(ButtonId::CrownTouch),
    );
    let mut motion = openlogi_hid::crown::CrownMotion::new(6, 1, true);
    motion.touched = true;
    dispatcher.dispatch(&parent, &policy, CapturedInput::CrownRotation(motion));
    dispatcher.cancel_session(&parent);
    assert!(buttons.shutdown());
    scroll.shutdown();
    assert!(
        events.try_iter().next().is_none(),
        "each layer has only half a step"
    );
}

#[test]
fn crown_step_percentages_accumulate_exactly() {
    use openlogi_core::rotary::RotarySensitivity;
    let output = HidppSessionId::new("craft");
    let binding = Action::Copy.into();
    for percent in [1, 10, 33, 50, 99, 100, 333, 1000] {
        let mut steps = super::super::CrownSteps {
            output: output.clone(),
            progress: HashMap::new(),
        };
        let sensitivity = RotarySensitivity::new(percent).unwrap();
        let pulses: u16 = (0..100)
            .map(|_| steps.advance(&output, ButtonId::CrownRight, &binding, 1, sensitivity))
            .sum();
        assert_eq!(pulses, percent, "100 reports at {percent}%");
    }
}

#[tokio::test]
async fn reconnect_cancels_crown_output_and_reconfirms_the_current_request() {
    use crate::hardware::crown::tests::Fixture;
    use crate::runtime::testing::CancelReason;
    use openlogi_core::rotary::TactileMode;

    for functions in [false, true] {
        let fixture = Fixture::new().await;
        let (handle, task, _capture, mut inputs) = fixture.start();
        let (outputs, mut buttons, events, mut scroll) = test_outputs(Arc::default());
        let mut dispatcher = super::super::super::InputDispatcher::new(outputs);
        let parent = HidppSessionId::new("craft");
        let mut policy = touch_plan(None);
        policy.bindings.clear();
        let profile = &mut policy.crown.as_mut().unwrap().profile;
        profile.set_enabled(functions);
        profile.set_feel(TactileMode::Smooth);
        let mut configured = profile.functions().to_vec();
        configured[0]
            .set_binding(
                CrownControl::Press,
                Action::HoldShortcut("F8".parse().unwrap()).into(),
            )
            .unwrap();
        profile.replace_functions(configured).unwrap();
        dispatcher.register_crown_mode(&parent, handle.clone(), &policy);
        let previous = tokio::time::timeout(Duration::from_secs(1), inputs.recv())
            .await
            .unwrap()
            .unwrap();
        dispatcher.dispatch(&parent, &policy, previous);
        assert_eq!(
            dispatcher.crown_confirmed.get(&parent.epoch()),
            Some(&TactileMode::Smooth)
        );
        if functions {
            dispatcher.dispatch(
                &parent,
                &policy,
                CapturedInput::ButtonDown(ButtonId::CrownPress),
            );
            assert!(matches!(
                events.recv_timeout(Duration::from_secs(1)).unwrap(),
                ButtonRuntimeEvent::Started(_)
            ));
        }
        fixture.power_cycle();
        dispatcher.dispatch(&parent, &policy, CapturedInput::Interrupted);
        if functions {
            assert!(matches!(
                events.recv_timeout(Duration::from_secs(1)).unwrap(),
                ButtonRuntimeEvent::Ended {
                    reason: EndReason::Canceled(CancelReason::SourceEnded),
                    ..
                }
            ));
        }
        dispatcher.dispatch(&parent, &policy, previous);
        assert!(
            !dispatcher.crown_confirmed.contains_key(&parent.epoch()),
            "pre-reconnect confirmation is stale"
        );
        let confirmed = tokio::time::timeout(Duration::from_secs(1), inputs.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            confirmed,
            CapturedInput::CrownModeApplied {
                mode: TactileMode::Smooth,
                request: 2
            }
        );
        dispatcher.dispatch(&parent, &policy, confirmed);
        assert_eq!(
            dispatcher.crown_confirmed.get(&parent.epoch()),
            Some(&TactileMode::Smooth)
        );
        assert!(events.try_iter().next().is_none());
        dispatcher.cancel_session(&parent);
        dispatcher.retire_crown_mode(&parent);
        assert!(task.finish().await.is_none());
        assert!(buttons.shutdown());
        scroll.shutdown();
    }
}
