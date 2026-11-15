//! Agent-owned Crown HUD publication and expiry. Clients only render its state.

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use openlogi_ipc::{CrownHud, CrownObservation, Generation, OBSERVE_HOLD};
use tokio::sync::watch;

use crate::runtime::HidppSessionId;

/// Watchdog for a lost touch-release or disconnected input source.
pub(crate) const CONTACT_LIFETIME: Duration = Duration::from_secs(15);
/// Brief feedback after release, tap selection, or an isolated rotation pulse.
pub(crate) const RELEASE_LIFETIME: Duration = Duration::from_millis(800);

struct Visible {
    owner: HidppSessionId,
    hud: CrownHud,
    expires: Instant,
}

#[derive(Clone)]
struct Publication {
    observation: CrownObservation,
    expires: Option<Instant>,
}

/// Read-only display channel shared by the capture owner and IPC server.
pub struct CrownDisplay {
    visible: Mutex<Option<Visible>>,
    published: watch::Sender<Publication>,
}

impl Default for CrownDisplay {
    fn default() -> Self {
        Self {
            visible: Mutex::new(None),
            published: watch::channel(Publication {
                observation: CrownObservation::new(1, None),
                expires: None,
            })
            .0,
        }
    }
}

impl CrownDisplay {
    pub(crate) fn show(&self, owner: &HidppSessionId, hud: CrownHud, lifetime: Duration) {
        let mut visible = self.visible.lock().unwrap_or_else(PoisonError::into_inner);
        *visible = Some(Visible {
            owner: owner.clone(),
            hud,
            expires: Instant::now() + lifetime,
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
        let hud = visible.map(|active| active.hud.clone());
        let expires = visible.map(|active| active.expires);
        self.published.send_if_modified(|published| {
            let changed = published.observation.hud != hud;
            if !changed && published.expires == expires {
                return false;
            }
            if changed {
                published.observation.hud = hud;
                published.observation.generation += 1;
            }
            // A release shortens the deadline even when presentation is unchanged.
            // Wake the agent's pending observe calls without sending a fake UI change.
            published.expires = expires;
            true
        });
    }

    /// Serve the level-triggered channel. Expiry stays in the agent, so a
    /// restarted overlay cannot revive an ended contact and needs no local timer.
    pub async fn observe(&self, since: Generation) -> CrownObservation {
        let until = Instant::now() + OBSERVE_HOLD;
        let mut rx = self.published.subscribe();
        loop {
            self.expire();
            let published = rx.borrow_and_update().clone();
            if published.observation.generation != since || Instant::now() >= until {
                return published.observation;
            }
            let deadline = published.expires.map_or(until, |expiry| expiry.min(until));
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

#[cfg(test)]
mod tests {
    use super::*;
    use openlogi_core::crown::CrownProfile;

    fn hud() -> CrownHud {
        let profile = CrownProfile::default();
        CrownHud::new(&profile, profile.functions()[0].id().clone())
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
