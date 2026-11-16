//! Bounded application delivery. Input dispatch only locks and enqueues.

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use openlogi_sdk::{ControlInput, RotationPhase, ServerMessage};
use tokio::sync::Notify;

/// Movement can merge, while edges and selection messages consume ordered slots.
const CAPACITY: usize = 64;

#[derive(Default)]
struct State {
    messages: VecDeque<ServerMessage>,
    closed: bool,
}

#[derive(Default)]
pub(super) struct Outbox {
    state: Mutex<State>,
    available: Notify,
}

impl Outbox {
    /// `false` is terminal. The owning host then ends the connection lease.
    pub(super) fn push(&self, message: ServerMessage) -> bool {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if state.closed {
            return false;
        }
        if let Some(last) = state.messages.back_mut()
            && merge(last, &message)
        {
            return true;
        }
        if state.messages.len() == CAPACITY {
            state.closed = true;
            state.messages.clear();
            self.available.notify_one();
            return false;
        }
        state.messages.push_back(message);
        self.available.notify_one();
        true
    }

    pub(super) async fn next(&self) -> Option<ServerMessage> {
        loop {
            let available = self.available.notified();
            {
                let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
                if state.closed {
                    return None;
                }
                if let Some(message) = state.messages.pop_front() {
                    return Some(message);
                }
            }
            available.await;
        }
    }

    pub(super) fn close(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.closed = true;
        state.messages.clear();
        self.available.notify_one();
    }
}

fn merge(previous: &mut ServerMessage, next: &ServerMessage) -> bool {
    let (
        ServerMessage::Input {
            session: old_session,
            last: old_last,
            time_us: old_time,
            tool: old_tool,
            event: ControlInput::Rotation(old),
            ..
        },
        ServerMessage::Input {
            session,
            first,
            last,
            time_us,
            tool,
            event: ControlInput::Rotation(next),
        },
    ) = (previous, next)
    else {
        return false;
    };
    if old_session != session
        || old_tool != tool
        || old_last.checked_add(1) != Some(*first)
        || old.phase != RotationPhase::Move
        || next.phase != RotationPhase::Move
        || old.amount.is_sign_positive() != next.amount.is_sign_positive()
        || old.pressed != next.pressed
        || old.touched != next.touched
        || old.tactile != next.tactile
        || old.resolution != next.resolution
    {
        return false;
    }
    let (Some(raw), Some(detents)) = (
        old.raw.checked_add(next.raw),
        old.detents.checked_add(next.detents),
    ) else {
        return false;
    };
    let amount = old.amount + next.amount;
    if !amount.is_finite() {
        return false;
    }
    old.raw = raw;
    old.detents = detents;
    old.amount = amount;
    old.speed = next.speed;
    *old_last = *last;
    *old_time = *time_us;
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use openlogi_sdk::{ControlId, Rotation, RotationResolution, SessionId, TactileMode};

    fn motion(sequence: u64, pressed: bool) -> ServerMessage {
        ServerMessage::Input {
            session: SessionId(1),
            first: sequence,
            last: sequence,
            time_us: sequence * 1000,
            tool: ControlId::new("zoom".into()).unwrap(),
            event: ControlInput::Rotation(Rotation {
                amount: 0.25,
                raw: 1,
                detents: 0,
                resolution: RotationResolution::new(1, 4),
                tactile: TactileMode::Smooth,
                phase: RotationPhase::Move,
                pressed,
                touched: false,
                speed: 4,
            }),
        }
    }

    #[tokio::test]
    async fn movement_retains_total_and_sequence_while_edges_and_contexts_stay_ordered() {
        let queue = Outbox::default();
        assert!(queue.push(motion(1, false)));
        assert!(queue.push(motion(2, false)));
        assert!(queue.push(motion(3, true)));
        let edge = ServerMessage::Input {
            session: SessionId(1),
            first: 4,
            last: 4,
            time_us: 4000,
            tool: ControlId::new("zoom".into()).unwrap(),
            event: ControlInput::Press {
                down: false,
                reserved: false,
            },
        };
        assert!(queue.push(edge.clone()));
        assert!(queue.push(motion(5, true)));
        let Some(ServerMessage::Input {
            first,
            last,
            event: ControlInput::Rotation(value),
            ..
        }) = queue.next().await
        else {
            panic!("rotation");
        };
        assert_eq!((first, last, value.raw, value.amount), (1, 2, 2, 0.5));
        assert_eq!(queue.next().await, Some(motion(3, true)));
        assert_eq!(queue.next().await, Some(edge));
        assert_eq!(queue.next().await, Some(motion(5, true)));
    }

    #[tokio::test]
    async fn slow_edge_consumers_end_the_queue_without_dropping_a_middle_edge() {
        let queue = Outbox::default();
        for index in 0..CAPACITY {
            assert!(queue.push(ServerMessage::ready(index as u64)));
        }
        assert!(!queue.push(ServerMessage::ready(1000)));
        assert!(queue.next().await.is_none());
        assert!(!queue.push(motion(1, false)));
    }

    #[tokio::test]
    async fn direction_reversals_keep_distinct_ordered_motion_ranges() {
        let queue = Outbox::default();
        let first = motion(1, false);
        let mut reverse = motion(2, false);
        let ServerMessage::Input {
            event: ControlInput::Rotation(rotation),
            ..
        } = &mut reverse
        else {
            panic!("motion");
        };
        rotation.amount = -rotation.amount;
        rotation.raw = -rotation.raw;
        assert!(queue.push(first.clone()));
        assert!(queue.push(reverse.clone()));
        assert_eq!(queue.next().await, Some(first));
        assert_eq!(queue.next().await, Some(reverse));
    }
}
