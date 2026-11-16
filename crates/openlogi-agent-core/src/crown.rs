//! Agent-owned Crown HUD publication and expiry. Clients only render its state.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use openlogi_core::crown::CrownFeedback;
use openlogi_core::parameter::ParameterState;
use openlogi_ipc::{CrownHud, CrownObservation, Generation, OBSERVE_HOLD};
use tokio::sync::watch;

use crate::runtime::HidppSessionId;

/// Watchdog for a lost touch-release or disconnected input source.
pub(crate) const CONTACT_LIFETIME: Duration = Duration::from_secs(15);
/// Brief feedback after release, tap selection, or an isolated rotation pulse.
pub(crate) const RELEASE_LIFETIME: Duration = Duration::from_millis(800);

const FEEDBACK_REFRESH: Duration = Duration::from_millis(250);

#[derive(Clone)]
struct Sample {
    value: ParameterState,
    next: Option<Instant>,
    reading: Option<u64>,
}

struct Visible {
    owner: HidppSessionId,
    hud: CrownHud,
    expires: Instant,
    sample: Option<Sample>,
}

#[derive(Clone)]
struct Publication {
    observation: CrownObservation,
    expires: Option<Instant>,
    refresh_at: Option<Instant>,
}

/// Read-only display channel shared by the capture owner and IPC server.
#[derive(Clone)]
pub struct CrownDisplay {
    visible: Arc<Mutex<Option<Visible>>>,
    published: watch::Sender<Publication>,
    read_brightness: fn() -> ParameterState,
    next_feedback: Arc<AtomicU64>,
}

impl Default for CrownDisplay {
    fn default() -> Self {
        Self {
            visible: Arc::new(Mutex::new(None)),
            read_brightness: native_brightness,
            next_feedback: Arc::new(AtomicU64::new(1)),
            published: watch::channel(Publication {
                observation: CrownObservation::new(1, None),
                expires: None,
                refresh_at: None,
            })
            .0,
        }
    }
}

impl CrownDisplay {
    pub(crate) fn show(&self, owner: &HidppSessionId, hud: CrownHud, lifetime: Duration) {
        let mut visible = self.visible.lock().unwrap_or_else(PoisonError::into_inner);
        let sample = if brightness(&hud) {
            visible
                .as_ref()
                .filter(|active| active.owner == *owner && active.hud.selected == hud.selected)
                .and_then(|active| active.sample.clone())
                .or_else(|| {
                    Some(Sample {
                        value: ParameterState::Unavailable,
                        next: Some(Instant::now()),
                        reading: None,
                    })
                })
        } else {
            None
        };
        *visible = Some(Visible {
            owner: owner.clone(),
            hud,
            expires: Instant::now() + lifetime,
            sample,
        });
        self.publish(visible.as_ref());
    }

    pub(crate) fn cancel(&self, owner: &HidppSessionId) {
        let mut visible = self.visible.lock().unwrap_or_else(PoisonError::into_inner);
        if visible
            .as_ref()
            .is_some_and(|active| &active.owner == owner)
        {
            *visible = None;
            self.publish(None);
        }
    }

    fn expire(&self) {
        let mut visible = self.visible.lock().unwrap_or_else(PoisonError::into_inner);
        if visible
            .as_ref()
            .is_some_and(|active| active.expires <= Instant::now())
        {
            *visible = None;
            self.publish(None);
        }
    }

    fn publish(&self, visible: Option<&Visible>) {
        let hud = visible.map(|active| {
            let mut hud = active.hud.clone();
            if let Some(sample) = &active.sample
                && let Some(function) = hud
                    .functions
                    .iter_mut()
                    .find(|function| function.id == hud.selected)
            {
                function.feedback = Some(CrownFeedback::Parameter(sample.value.clone()));
            }
            hud
        });
        let refresh_at = visible.and_then(|active| active.sample.as_ref()?.next);
        let expires = visible.map(|active| active.expires);
        self.published.send_if_modified(|published| {
            let changed = published.observation.hud != hud;
            if !changed && published.expires == expires && published.refresh_at == refresh_at {
                return false;
            }
            if changed {
                published.observation.hud = hud;
                published.observation.generation += 1;
            }
            // A release shortens the deadline even when presentation is unchanged.
            // Wake the agent's pending observe calls without sending a fake UI change.
            published.expires = expires;
            published.refresh_at = refresh_at;
            true
        });
    }

    fn refresh_feedback(&self) {
        let mut visible = self.visible.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(active) = visible.as_mut() else {
            return;
        };
        let Some(sample) = active.sample.as_mut() else {
            return;
        };
        if sample.reading.is_some() || sample.next.is_none_or(|next| next > Instant::now()) {
            return;
        }
        let request = self.next_feedback.fetch_add(1, Ordering::Relaxed);
        sample.next = None;
        sample.reading = Some(request);
        let owner = active.owner.clone();
        self.publish(visible.as_ref());
        let display = self.clone();
        // Native reads stay outside input dispatch and the async executor.
        // This task owns completion even if an IPC observer disconnects.
        tokio::spawn(async move {
            let value = tokio::task::spawn_blocking(display.read_brightness)
                .await
                .unwrap_or(ParameterState::Unavailable);
            display.complete_feedback(&owner, request, value);
        });
    }

    fn complete_feedback(&self, owner: &HidppSessionId, request: u64, value: ParameterState) {
        let mut visible = self.visible.lock().unwrap_or_else(PoisonError::into_inner);
        let Some(active) = visible.as_mut().filter(|active| active.owner == *owner) else {
            return;
        };
        let Some(sample) = active
            .sample
            .as_mut()
            .filter(|sample| sample.reading == Some(request))
        else {
            return;
        };
        sample.value = value;
        sample.reading = None;
        sample.next = Some(Instant::now() + FEEDBACK_REFRESH);
        self.publish(visible.as_ref());
    }

    /// Serve the level-triggered channel. Expiry stays in the agent, so a
    /// restarted overlay cannot revive an ended contact and needs no local timer.
    pub async fn observe(&self, since: Generation) -> CrownObservation {
        let until = Instant::now() + OBSERVE_HOLD;
        let mut rx = self.published.subscribe();
        loop {
            self.expire();
            self.refresh_feedback();
            let published = rx.borrow_and_update().clone();
            if published.observation.generation != since || Instant::now() >= until {
                return published.observation;
            }
            let deadline = [published.expires, published.refresh_at]
                .into_iter()
                .flatten()
                .fold(until, Instant::min);
            // Internal deadline updates re-arm this wait even if the HUD's
            // generation stays the same. Closed sender is terminal.
            if matches!(
                tokio::time::timeout_at(deadline.into(), rx.changed()).await,
                Ok(Err(_))
            ) {
                return self.published.borrow().observation.clone();
            }
        }
    }
}

fn native_brightness() -> ParameterState {
    openlogi_inject::brightness::read()
        .map_or(ParameterState::Unavailable, ParameterState::Available)
}

fn brightness(hud: &CrownHud) -> bool {
    hud.functions.iter().any(|function| {
        function.id == hud.selected && function.feedback == Some(CrownFeedback::SystemBrightness)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use openlogi_core::crown::CrownProfile;

    fn hud() -> CrownHud {
        let profile = CrownProfile::default();
        CrownHud::new(&profile, profile.functions()[0].id().clone())
    }

    fn brightness_hud() -> CrownHud {
        let mut hud = hud();
        hud.functions[0].feedback = Some(CrownFeedback::SystemBrightness);
        hud
    }

    #[tokio::test]
    async fn visible_parameter_refresh_reads_external_changes_and_marks_failures() {
        use openlogi_core::parameter::ParameterValue;
        use std::sync::atomic::{AtomicI32, Ordering};
        static LEVEL: AtomicI32 = AtomicI32::new(25);
        fn read() -> ParameterState {
            let value = LEVEL.load(Ordering::SeqCst);
            if value < 0 {
                ParameterState::Unavailable
            } else {
                ParameterState::Available(
                    ParameterValue::new(value, 0, 100, format!("{value}%")).unwrap(),
                )
            }
        }
        let display = CrownDisplay {
            read_brightness: read,
            ..CrownDisplay::default()
        };
        let owner = HidppSessionId::new("craft");
        display.show(&owner, brightness_hud(), CONTACT_LIFETIME);
        let first = display.observe(0).await;
        let measured = display.observe(first.generation).await;
        assert_eq!(
            measured.hud.unwrap().functions[0].feedback,
            Some(CrownFeedback::Parameter(read()))
        );
        LEVEL.store(75, Ordering::SeqCst);
        let changed = display.observe(measured.generation).await;
        assert_eq!(
            changed.hud.unwrap().functions[0].feedback,
            Some(CrownFeedback::Parameter(read()))
        );
        LEVEL.store(-1, Ordering::SeqCst);
        let failed = display.observe(changed.generation).await;
        assert_eq!(
            failed.hud.unwrap().functions[0].feedback,
            Some(CrownFeedback::Parameter(ParameterState::Unavailable))
        );
    }

    #[test]
    fn a_late_parameter_read_cannot_replace_a_new_function_or_capture() {
        let display = CrownDisplay::default();
        let owner = HidppSessionId::new("craft");
        display.show(&owner, brightness_hud(), CONTACT_LIFETIME);
        let request = 7;
        display
            .visible
            .lock()
            .unwrap()
            .as_mut()
            .unwrap()
            .sample
            .as_mut()
            .unwrap()
            .reading = Some(request);
        let next = HidppSessionId::new("craft");
        display.show(&next, hud(), CONTACT_LIFETIME);
        display.complete_feedback(&owner, request, ParameterState::Unavailable);
        assert_eq!(
            display.published.borrow().observation.hud.as_ref().unwrap(),
            &hud()
        );
    }

    #[tokio::test]
    async fn expired_contacts_are_not_revived_by_a_new_overlay_observer() {
        let display = CrownDisplay::default();
        let owner = HidppSessionId::new("craft");
        display.show(&owner, hud(), Duration::ZERO);
        let observation = display.observe(0).await;
        assert!(observation.hud.is_none());
        assert_eq!(observation.generation, 3);
        assert_eq!(
            display.observe(0).await,
            observation,
            "expiry publishes one dismissal"
        );
    }

    #[tokio::test]
    async fn release_rearms_a_pending_observe_even_when_the_function_has_not_changed() {
        use std::future::{Future as _, poll_fn};
        use std::task::Poll;

        let display = CrownDisplay::default();
        let owner = HidppSessionId::new("craft");
        display.show(&owner, hud(), CONTACT_LIFETIME);
        let initial = display.observe(0).await;
        let waiting = display.observe(initial.generation);
        tokio::pin!(waiting);
        assert!(poll_fn(|cx| Poll::Ready(waiting.as_mut().poll(cx).is_pending())).await);
        display.show(&owner, hud(), Duration::ZERO);
        let dismissed = tokio::time::timeout(Duration::from_secs(1), waiting)
            .await
            .expect("release wakes the old contact wait");
        assert!(dismissed.hud.is_none());
        assert_eq!(dismissed.generation, initial.generation + 1);
    }
}
