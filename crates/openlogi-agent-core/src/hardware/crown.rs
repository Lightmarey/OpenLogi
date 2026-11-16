//! One capture epoch's tactile lease. Restoration completes before its successor.

use openlogi_core::rotary::TactileMode;
use openlogi_hid::{HidppOperation, crown::RatchetMode};
use tokio::sync::{oneshot, watch};

use super::DeviceOp;
use crate::receiver_access::SessionReceiverLease;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct CrownModeHandle {
    requested: watch::Sender<TactileMode>,
    #[cfg(test)]
    applied: watch::Receiver<Option<TactileMode>>,
}

impl CrownModeHandle {
    pub(crate) fn request(&self, mode: TactileMode) {
        self.requested.send_if_modified(|requested| {
            if *requested == mode {
                return false;
            }
            *requested = mode;
            true
        });
    }
}

pub(crate) struct CrownModeTask {
    stop: oneshot::Sender<()>,
    task: tokio::task::JoinHandle<Option<(openlogi_hid::SharedChannel, RatchetMode)>>,
}

impl CrownModeTask {
    pub(crate) fn spawn(
        op: DeviceOp,
        lease: Arc<SessionReceiverLease>,
        inputs: tokio::sync::mpsc::UnboundedSender<openlogi_hid::CapturedInput>,
    ) -> (CrownModeHandle, Self) {
        let (requested, request) = watch::channel(TactileMode::Inherit);
        #[cfg(test)]
        let (applied, observed) = watch::channel(None);
        let (stop, stopping) = oneshot::channel();
        let task = tokio::spawn(run(
            op,
            request,
            #[cfg(test)]
            applied,
            stopping,
            lease,
            inputs,
        ));
        (
            CrownModeHandle {
                requested,
                #[cfg(test)]
                applied: observed,
            },
            Self { stop, task },
        )
    }

    pub(crate) async fn finish(self) -> Option<(openlogi_hid::SharedChannel, RatchetMode)> {
        let _ = self.stop.send(());
        match self.task.await {
            Ok(pending) => pending,
            Err(error) => {
                tracing::warn!(%error, "Crown tactile lease ended unexpectedly");
                None
            }
        }
    }
}

fn tactile(mode: RatchetMode) -> Option<TactileMode> {
    match mode {
        RatchetMode::Free => Some(TactileMode::Smooth),
        RatchetMode::Ratchet => Some(TactileMode::Ratchet),
        _ => None,
    }
}

async fn run(
    op: DeviceOp,
    mut requested: watch::Receiver<TactileMode>,
    #[cfg(test)] applied: watch::Sender<Option<TactileMode>>,
    mut stop: oneshot::Receiver<()>,
    lease: Arc<SessionReceiverLease>,
    inputs: tokio::sync::mpsc::UnboundedSender<openlogi_hid::CapturedInput>,
) -> Option<(openlogi_hid::SharedChannel, RatchetMode)> {
    let mut original = None;
    let mut retry = false;
    loop {
        tokio::select! {
            biased;
            _ = &mut stop => break,
            () = tokio::time::sleep(std::time::Duration::from_secs(1)), if retry => {},
            changed = requested.changed() => {
                if changed.is_err() { break; }
            }
        }
        let desired = *requested.borrow_and_update();
        retry = false;
        if desired == TactileMode::Inherit && original.is_none() {
            continue;
        }
        if original.is_none() {
            match op
                .clone()
                .run_in_session(HidppOperation::ReadCrown, &lease, |shared| async move {
                    let mode = openlogi_hid::crown::get_mode_on(&shared).await?;
                    Ok((shared, mode))
                })
                .await
            {
                Ok(mode) => {
                    original = Some(mode);
                }
                Err(error) => {
                    tracing::warn!(%error, "could not acquire Crown tactile lease");
                    retry = !matches!(error, openlogi_hid::WriteError::FeatureUnsupported { .. });
                    continue;
                }
            }
        }
        let mode = match desired {
            TactileMode::Inherit => original.as_ref()?.1,
            TactileMode::Smooth => RatchetMode::Free,
            TactileMode::Ratchet => RatchetMode::Ratchet,
        };
        match write(&op, &lease, mode).await {
            Ok(mode) => {
                if let Some(mode) = tactile(mode) {
                    let _ = inputs.send(openlogi_hid::CapturedInput::CrownModeApplied(mode));
                    #[cfg(test)]
                    applied.send_replace(Some(mode));
                }
            }
            Err(error) => {
                tracing::warn!(%error, "Crown tactile mode was not confirmed");
                retry = !matches!(error, openlogi_hid::WriteError::FeatureUnsupported { .. });
            }
        }
    }
    if let Some((channel, mode)) = original
        && let Err(error) = write(&op, &lease, mode).await
    {
        tracing::warn!(%error, "could not restore Crown tactile lease");
        return Some((channel, mode));
    }
    None
}

async fn write(
    op: &DeviceOp,
    lease: &SessionReceiverLease,
    mode: RatchetMode,
) -> Result<RatchetMode, openlogi_hid::WriteError> {
    op.clone()
        .run_in_session(
            HidppOperation::WriteCrown,
            lease,
            move |shared| async move { openlogi_hid::crown::set_mode_on(&shared, mode).await },
        )
        .await
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::*;
    use crate::hardware::{DeviceAccess, TransportTurns};
    use crate::receiver_access::{ExclusiveAccessReason, ReceiverAccess};

    struct DeviceState {
        mode: u8,
        writes: Vec<u8>,
        reject_restore: bool,
        controls: u8,
        info_reads: usize,
    }

    struct Fixture {
        access: DeviceAccess,
        route: openlogi_hid::DeviceRoute,
        state: Arc<Mutex<DeviceState>>,
    }

    impl Fixture {
        async fn new() -> Self {
            let registry = openlogi_hid::ChannelRegistry::default();
            let route = openlogi_hid::DeviceRoute::Direct {
                vendor_id: 0x046d,
                product_id: 0xb350,
            };
            let state = Arc::new(Mutex::new(DeviceState {
                mode: 2,
                writes: Vec::new(),
                reject_restore: false,
                controls: 4,
                info_reads: 0,
            }));
            let hardware = Arc::clone(&state);
            openlogi_device::test_support::publish_scripted_channel(
                &registry,
                "crown-mode",
                route.clone(),
                move |request| {
                    let mut response = vec![0; 20];
                    response[..4].copy_from_slice(request.get(..4)?);
                    response[0] = 0x11;
                    let mut state = hardware.lock().unwrap();
                    match (request[2], request[3] >> 4) {
                        (0, 1) => response[4] = 4,
                        (0, 0) => response[4] = if request[4..6] == [0x46, 0] { 9 } else { 0 },
                        (9, 0) => {
                            state.info_reads += 1;
                            response[4..10].copy_from_slice(&[state.controls, 15, 0, 120, 0, 20]);
                        }
                        (9, 1) => response[4..9].copy_from_slice(&[1, state.mode, 17, 29, 41]),
                        (9, 2) => {
                            assert_eq!(
                                request[4], 0,
                                "a tactile write does not divert native HID input"
                            );
                            assert!(request[6..].iter().all(|byte| *byte == 0));
                            if request[5] == 2 && state.reject_restore {
                                response[2..6].copy_from_slice(&[0xff, 9, request[3], 2]);
                                return Some(response);
                            }
                            state.mode = request[5];
                            state.writes.push(request[5]);
                            return Some(request.to_vec());
                        }
                        _ => return None,
                    }
                    Some(response)
                },
            )
            .await;
            let (_signal, device_io) = openlogi_hid::device_io_channel();
            let access = DeviceAccess {
                channel: Arc::default(),
                registry,
                receiver_access: ReceiverAccess::default(),
                device_io,
                transport_turns: TransportTurns::default(),
            };
            Self {
                access,
                route,
                state,
            }
        }

        fn start(
            &self,
        ) -> (
            CrownModeHandle,
            CrownModeTask,
            Arc<SessionReceiverLease>,
            tokio::sync::mpsc::UnboundedReceiver<openlogi_hid::CapturedInput>,
        ) {
            let lease = Arc::new(
                self.access
                    .receiver_access
                    .try_acquire_for_session()
                    .unwrap(),
            );
            let (sink, inputs) = tokio::sync::mpsc::unbounded_channel();
            let (handle, task) =
                CrownModeTask::spawn(self.access.op(&self.route), Arc::clone(&lease), sink);
            (handle, task, lease, inputs)
        }
    }

    async fn select(handle: &CrownModeHandle, expected: TactileMode) {
        handle.request(expected);
        let mut observed = handle.applied.clone();
        tokio::time::timeout(
            Duration::from_secs(1),
            observed.wait_for(|mode| *mode == Some(expected)),
        )
        .await
        .unwrap()
        .unwrap();
    }

    #[tokio::test]
    async fn crown_modes_are_confirmed_and_the_original_mode_is_restored_before_completion() {
        let fixture = Fixture::new().await;
        let (handle, task, capture, mut inputs) = fixture.start();
        for expected in [
            TactileMode::Smooth,
            TactileMode::Ratchet,
            TactileMode::Smooth,
        ] {
            select(&handle, expected).await;
            assert_eq!(
                inputs.try_recv().unwrap(),
                openlogi_hid::CapturedInput::CrownModeApplied(expected)
            );
        }
        let mut requests = fixture.access.receiver_access.subscribe_requests();
        let pairing_access = fixture.access.receiver_access.clone();
        let pairing = tokio::spawn(async move {
            pairing_access
                .acquire_exclusive(ExclusiveAccessReason::Pairing)
                .await
        });
        requests.wait_for(|state| state.any()).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(1), task.finish())
                .await
                .expect("capture must restore before yielding to pairing")
                .is_none()
        );
        drop(capture);
        drop(pairing.await.unwrap());
        let state = fixture.state.lock().unwrap();
        assert_eq!(state.mode, 2);
        assert_eq!(state.writes, [1, 2, 1, 2]);
    }

    #[tokio::test]
    async fn nonmechanized_crowns_never_acquire_a_tactile_restore_debt() {
        let fixture = Fixture::new().await;
        fixture.state.lock().unwrap().controls = 0;
        let (handle, task, _capture, _inputs) = fixture.start();
        handle.request(TactileMode::Smooth);
        tokio::time::timeout(Duration::from_secs(1), async {
            while fixture.state.lock().unwrap().info_reads == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            task.finish().await.is_none(),
            "an unsupported write never owns a tactile restore"
        );
        assert!(fixture.state.lock().unwrap().writes.is_empty());
    }

    #[tokio::test]
    async fn crown_failed_restore_remains_owned_until_a_successful_retry() {
        let fixture = Fixture::new().await;
        let (handle, task, _capture, _inputs) = fixture.start();
        select(&handle, TactileMode::Smooth).await;
        fixture.state.lock().unwrap().reject_restore = true;
        let (channel, original) = task.finish().await.expect("a failed restore remains owned");
        let pending = openlogi_hid::PendingCaptureRestore::crown_mode(&channel, original);
        let pending = match pending.retry(&fixture.access.registry).await {
            openlogi_hid::CaptureSessionOutcome::RestorePending(pending) => pending,
            openlogi_hid::CaptureSessionOutcome::Restored => {
                panic!("rejected mode restore cannot complete")
            }
        };
        fixture.state.lock().unwrap().reject_restore = false;
        assert!(matches!(
            pending.retry(&fixture.access.registry).await,
            openlogi_hid::CaptureSessionOutcome::Restored
        ));
        assert_eq!(fixture.state.lock().unwrap().mode, 2);
    }
}
