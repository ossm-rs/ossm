use core::cell::Cell;

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::blocking_mutex::Mutex;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MotionPhase {
    Disabled,
    Enabled,
    Ready,
    Moving,
    Stopping,
    Paused,
}

events::declare_state!(MotionPhase, MotionPhase::Disabled);

#[derive(Debug, Clone, Copy)]
pub struct MotionState {
    pub phase: MotionPhase,
    /// Current position as a fraction of the machine range (0.0–1.0).
    pub position: f32,
    /// Current velocity as a fraction of max velocity (0.0–1.0).
    pub velocity: f32,
    /// Current acceleration as a fraction of max acceleration (0.0–1.0).
    pub acceleration: f32,
    /// Current torque limit as a fraction (0.0–1.0).
    pub torque: f32,
}

impl MotionState {
    pub(crate) const fn new() -> Self {
        Self {
            phase: MotionPhase::Disabled,
            position: 0.0,
            velocity: 0.0,
            acceleration: 0.0,
            torque: 0.0,
        }
    }
}

/// Holds the most recent [`MotionState`] snapshot. Phase transitions
/// are mirrored to `events::state::<MotionPhase>()` so subscribers
/// outside this crate don't need a handle to `Ossm`.
pub(crate) struct MotionStateChannels {
    state: Mutex<CriticalSectionRawMutex, Cell<MotionState>>,
}

impl MotionStateChannels {
    pub(crate) const fn new() -> Self {
        Self {
            state: Mutex::new(Cell::new(MotionState::new())),
        }
    }

    pub(crate) fn update(&self, new_state: MotionState) {
        self.state.lock(|cell| cell.set(new_state));
    }

    pub(crate) fn publish_phase(&self, phase: MotionPhase) {
        events::state::<MotionPhase>().write(phase);
    }

    pub fn get(&self) -> MotionState {
        self.state.lock(|cell| cell.get())
    }
}
