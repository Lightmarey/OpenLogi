//! Crown firmware ownership: opt-in arming, wake rearm and native restoration.

use super::*;

async fn calibrated_crown_capture() -> GestureCapture {
    let (raw, _) = ScriptedRawHidChannel::with_dynamic_responder(|request| {
        let mut response = vec![0; 20];
        response[..4].copy_from_slice(&request[..4]);
        response[0] = 0x11;
        match (request[2], request[3] >> 4) {
            (0, 1) => response[4] = 4,
            (0, 0) if request[4..6] == [0x46, 0x00] => response[4] = 9,
            (0, 0) => {}
            (9, 0) => response[4..10].copy_from_slice(&[7, 15, 0, 120, 0, 20]),
            (9, 1) => response[4..9].copy_from_slice(&[2, 2, 17, 29, 41]),
            (9, 2) => return Some(request.to_vec()),
            _ => panic!("unexpected crown request: {request:02x?}"),
        }
        Some(response)
    });
    let channel = scripted_channel(raw).await;
    let device = Device::new(channel.clone(), 0xff).await.unwrap();
    let mut armed = ArmedControls::default();
    arm_controls_into(
        &device,
        &channel,
        0xff,
        &CaptureSpec {
            capture_crown: true,
            ..CaptureSpec::default()
        },
        &mut armed,
    )
    .await
    .unwrap();
    GestureCapture::new(armed)
}

fn crown_report(slots: i8, touch: u8, pressed: u8) -> hidpp::protocol::v20::Message {
    let mut payload = [0; 16];
    payload[1] = slots.cast_unsigned();
    payload[4] = touch;
    payload[6] = pressed;
    hidpp::protocol::v20::Message::Long(
        hidpp::protocol::v20::MessageHeader {
            device_index: 0xff,
            feature_index: 9,
            function_id: hidpp::nibble::U4::from_lo(0),
            software_id: hidpp::nibble::U4::from_lo(0),
        },
        payload,
    )
}

#[tokio::test]
async fn wake_keeps_crown_calibration() {
    let capture = calibrated_crown_capture().await;
    let (sink, mut inputs) = mpsc::unbounded_channel();
    let handler = capture.report_handler(0xff, sink);
    assert!(matches!(
        inputs.try_recv().unwrap(),
        CapturedInput::CrownReady(_)
    ));
    handler(&crown_report(6, 0, 0));
    let CapturedInput::CrownRotation(before) = inputs.try_recv().unwrap() else {
        panic!("motion");
    };
    assert_eq!(before.resolution.native_per_increment(), 1.0 / 6.0);
    capture.reset_input_state();
    assert_eq!(inputs.try_recv().unwrap(), CapturedInput::Interrupted);
    handler(&crown_report(6, 0, 0));
    let CapturedInput::CrownRotation(after) = inputs.try_recv().unwrap() else {
        panic!("motion");
    };
    assert_eq!(
        after.resolution, before.resolution,
        "wake must retain measured calibration"
    );
}

#[tokio::test]
async fn wake_cancels_crown_holds() {
    let capture = calibrated_crown_capture().await;
    let (sink, mut inputs) = mpsc::unbounded_channel();
    let handler = capture.report_handler(0xff, sink);
    assert!(matches!(
        inputs.try_recv().unwrap(),
        CapturedInput::CrownReady(_)
    ));
    handler(&crown_report(0, 1, 1));
    let down = std::iter::from_fn(|| inputs.try_recv().ok()).collect::<Vec<_>>();
    assert!(down.contains(&CapturedInput::ButtonDown(ButtonId::CrownPress)));
    capture.reset_input_state();
    handler(&crown_report(0, 0, 0));
    let ended = std::iter::from_fn(|| inputs.try_recv().ok()).collect::<Vec<_>>();
    assert!(
        ended.contains(&CapturedInput::Interrupted),
        "a power-cycle must cancel Crown output before forgetting its press, actual {ended:?}"
    );
}

#[tokio::test]
async fn wake_announces_reset_while_crown_reporting_remains_native() {
    let capture = GestureCapture::new(ArmedControls::default());
    let (sink, mut inputs) = mpsc::unbounded_channel();
    let _handler = capture.report_handler(0xff, sink);
    capture.reset_input_state();
    assert_eq!(inputs.try_recv().unwrap(), CapturedInput::Interrupted);
    inputs.try_recv().unwrap_err();
}

#[tokio::test]
async fn crown_capture_changes_only_reporting_and_restores_even_an_old_diversion() {
    let (raw, handle) = ScriptedRawHidChannel::with_dynamic_responder(|request| {
        let mut response = vec![0; 20];
        response[..4].copy_from_slice(&request[..4]);
        response[0] = 0x11;
        match (request[2], request[3] >> 4) {
            (0, 1) => response[4] = 4,
            (0, 0) if request[4..6] == [0x46, 0x00] => response[4] = 9,
            (0, 0) => {} // No mouse controls on this fixture.
            (9, 0) => response[4..10].copy_from_slice(&[4, 15, 0, 120, 0, 20]),
            (9, 1) => response[4..9].copy_from_slice(&[2, 2, 17, 29, 41]),
            (9, 2) => return Some(request.to_vec()),
            _ => panic!("unexpected crown request: {request:02x?}"),
        }
        Some(response)
    });
    let channel = scripted_channel(raw).await;
    let device = Device::new(channel.clone(), 0xff).await.unwrap();
    let route = DeviceRoute::Direct {
        vendor_id: 0x046d,
        product_id: 0xb350,
    };
    let shared = SharedChannel::new(channel.clone(), route.clone());
    let registry = ChannelRegistry::default();
    registry.replace_node(
        NodeId::from("crown-test".to_owned()),
        [route],
        channel.clone(),
    );

    let mut idle = ArmedControls::default();
    arm_controls_into(&device, &channel, 0xff, &CaptureSpec::default(), &mut idle)
        .await
        .unwrap();
    assert!(idle.crown.is_none());
    assert!(handle.written_reports().iter().all(|report| report[2] != 9));

    let mut armed = ArmedControls::default();
    arm_controls_into(
        &device,
        &channel,
        0xff,
        &CaptureSpec {
            capture_crown: true,
            ..CaptureSpec::default()
        },
        &mut armed,
    )
    .await
    .unwrap();
    assert!(armed.crown.as_ref().is_some_and(|crown| crown.ratcheted));
    let (sink, mut inputs) = tokio::sync::mpsc::unbounded_channel();
    let _reports = crate::session::gesture::crown::report_handler(
        armed.crown.as_ref(),
        Arc::default(),
        0xff,
        sink,
    );
    let CapturedInput::CrownReady(capabilities) = inputs.try_recv().unwrap() else {
        panic!("source discovery precedes physical inputs");
    };
    assert!(
        capabilities
            .features
            .contains(openlogi_core::application_control::ControlFeatures::TACTILE)
    );
    assert!(
        capabilities
            .features
            .contains(openlogi_core::application_control::ControlFeatures::TOUCH)
    );
    assert!(
        !capabilities
            .features
            .contains(openlogi_core::application_control::ControlFeatures::PRESS)
    );
    assert_eq!(capabilities.resolution, WheelResolution::new(20, 120));
    armed.rearm().await;
    let pending = armed
        .into_pending(&shared)
        .expect("crown alone must owe a restore");
    assert!(matches!(
        pending.allow_current_channel().retry(&registry).await,
        CaptureSessionOutcome::Restored
    ));
    let writes: Vec<_> = handle
        .written_reports()
        .into_iter()
        .filter(|report| report[2] == 9 && report[3] >> 4 == 2)
        .collect();
    assert_eq!(writes.len(), 3);
    assert_eq!(
        writes.iter().map(|report| report[4]).collect::<Vec<_>>(),
        [2, 2, 1]
    );
    for write in writes {
        assert!(
            write[5..].iter().all(|byte| *byte == 0),
            "ratchet and gesture timings must remain unchanged"
        );
    }
}

#[tokio::test]
async fn crown_restore_waits_for_inventory_replacement_and_uses_its_channel() {
    let route = DeviceRoute::Direct {
        vendor_id: 0x046d,
        product_id: 0xb350,
    };
    let (raw, old_handle) = ScriptedRawHidChannel::with_responder(|_| None);
    let old_channel = scripted_channel(raw).await;
    let old = SharedChannel::new(old_channel.clone(), route.clone());
    let registry = ChannelRegistry::default();
    let node = NodeId::from("crown-test".to_owned());
    registry.replace_node(node.clone(), [route.clone()], old_channel);
    let pending = PendingCaptureRestore::new(&old, None, None, None, None, None, Some(9))
        .expect("crown restore");
    let pending = match pending.retry(&registry).await {
        CaptureSessionOutcome::RestorePending(pending) => pending,
        CaptureSessionOutcome::Restored => {
            panic!("retired channel cannot release firmware ownership")
        }
    };
    assert!(old_handle.written_reports().is_empty());
    let (raw, new_handle) = ScriptedRawHidChannel::with_responder(|request| Some(request.to_vec()));
    registry.replace_node(node, [route], scripted_channel(raw).await);
    assert!(matches!(
        pending.retry(&registry).await,
        CaptureSessionOutcome::Restored
    ));
    let writes = new_handle.written_reports();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0][2], 9);
    assert_eq!(&writes[0][4..9], &[1, 0, 0, 0, 0]);
}
