#[test]
#[ignore = "requires writable /dev/uinput and readable evdev keyboard nodes"]
fn readable_keyboard_reports_both_shift_keys_and_release() {
    use evdev::{AttributeSet, InputEvent, KeyCode};
    let keys = AttributeSet::from_iter([KeyCode::KEY_LEFTSHIFT, KeyCode::KEY_RIGHTSHIFT]);
    let mut keyboard =
        openlogi_inject::uinput::builder(openlogi_inject::uinput::Device::ActionInjector)
            .unwrap()
            .with_keys(&keys)
            .unwrap()
            .build()
            .unwrap();
    let node = keyboard
        .enumerate_dev_nodes_blocking()
        .unwrap()
        .next()
        .unwrap()
        .unwrap();
    // udev applies input-group access after the kernel publishes the node.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut reader = loop {
        match evdev::Device::open(&node) {
            Ok(reader) => break reader,
            Err(error)
                if error.kind() == std::io::ErrorKind::PermissionDenied
                    && std::time::Instant::now() < deadline =>
            {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(error) => panic!("read the virtual modifier device: {error}"),
        }
    };
    reader.grab().unwrap();
    let mut check = |keyboard: &mut evdev::uinput::VirtualDevice, code, value, expected| {
        keyboard.emit(&[InputEvent::new(1, code, value)]).unwrap();
        assert!(
            reader
                .fetch_events()
                .unwrap()
                .any(|event| event.code() == code)
        );
        assert_eq!(super::keyboard_modifiers(&reader).unwrap().shift, expected);
    };
    check(&mut keyboard, KeyCode::KEY_LEFTSHIFT.0, 1, true);
    check(&mut keyboard, KeyCode::KEY_RIGHTSHIFT.0, 1, true);
    check(&mut keyboard, KeyCode::KEY_LEFTSHIFT.0, 0, true);
    check(&mut keyboard, KeyCode::KEY_RIGHTSHIFT.0, 0, false);
}
