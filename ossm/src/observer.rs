use crate::Ossm;
use crate::state::{MotionPhase, MotionState};

/// Read-only observer of motion state.
///
/// Produced by [`Ossm::split`](crate::Ossm::split) alongside
/// [`MotionSender`](crate::MotionSender) and
/// [`MotionReceiver`](crate::MotionReceiver). Anyone with a borrow of
/// the observer can read the current motion state and subscribe to
/// phase transitions, but cannot move the motor or service motion
/// commands.
///
/// Not [`Clone`] and not publicly constructible.
pub struct MotionObserver {
    channels: &'static Ossm,
}

impl MotionObserver {
    pub(crate) fn new(channels: &'static Ossm) -> Self {
        Self { channels }
    }

    /// Read the current motion state (position, velocity, torque, phase).
    pub fn state(&self) -> MotionState {
        self.channels.motion_state.get()
    }

    /// Subscribe to [`MotionPhase`] transitions.
    ///
    /// The subscription yields the current phase on first
    /// [`changed()`](events::StateSubscription::changed), then each
    /// subsequent transition.
    pub fn subscribe(&self) -> events::StateSubscription<MotionPhase> {
        events::state::<MotionPhase>().subscribe()
    }
}
