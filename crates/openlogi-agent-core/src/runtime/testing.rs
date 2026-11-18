//! Real button input with observed output for input-producer tests.

use std::sync::{Arc, RwLock, mpsc};

use super::ActionDispatcher;
pub(crate) use super::button::{ButtonRuntimeEvent, ButtonRuntimeOwner, CancelReason, EndReason};

pub(crate) fn test_dispatcher() -> (
    ActionDispatcher,
    ButtonRuntimeOwner,
    mpsc::Receiver<super::button::ButtonRuntimeEvent>,
) {
    let (events, received) = mpsc::channel();
    let owner = ButtonRuntimeOwner::spawn(move |event| {
        events
            .send(event)
            .expect("test receiver should stay connected");
    })
    .expect("button worker should start");
    let (action_ring, _ring_events) = tokio::sync::mpsc::unbounded_channel();
    let dispatcher = ActionDispatcher {
        executor: super::ActionExecutor {
            dpi_cycle: Arc::new(RwLock::new(crate::DpiCycles::default())),
            pointer_speeds: Arc::default(),
            access: crate::hardware::DeviceAccess {
                channel: Arc::new(RwLock::new(None)),
                registry: openlogi_hid::ChannelRegistry::default(),
                receiver_access: crate::receiver_access::ReceiverAccess::default(),
                device_io: openlogi_hid::device_io_channel().1,
                transport_turns: crate::hardware::TransportTurns::default(),
            },
            requests: action_ring,
        },
        buttons: owner.input(),
    };
    (dispatcher, owner, received)
}
