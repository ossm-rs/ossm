use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;

pub(crate) type MoveChannel = Channel<CriticalSectionRawMutex, MoveCommand, 1>;
pub(crate) type StateChannel = Channel<CriticalSectionRawMutex, StateCommand, 1>;
pub(crate) type StateResponseSignal = Signal<CriticalSectionRawMutex, StateResponse>;
pub(crate) type MoveResponseSignal = Signal<CriticalSectionRawMutex, Result<(), Cancelled>>;

pub(crate) struct OssmChannels {
    pub(crate) move_cmd: MoveChannel,
    pub(crate) state_cmd: StateChannel,
    pub(crate) state_resp: StateResponseSignal,
    pub(crate) move_resp: MoveResponseSignal,
}

impl OssmChannels {
    pub(crate) const fn new() -> Self {
        Self {
            move_cmd: MoveChannel::new(),
            state_cmd: StateChannel::new(),
            state_resp: StateResponseSignal::new(),
            move_resp: MoveResponseSignal::new(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MotionCommand {
    /// Target position as a fraction of the machine range (0.0–1.0).
    pub position: f64,
    /// Velocity as a fraction of max velocity (0.0–1.0).
    pub speed: f64,
    /// Torque limit as a fraction (0.0–1.0). `None` uses the motor default.
    pub torque: Option<f64>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum MoveCommand {
    MoveTo(f64),
    Motion(MotionCommand),
}

#[derive(Debug, Clone, Copy)]
pub enum StateCommand {
    Enable,
    Disable,
    Home,
    Pause,
    Resume,
    SetSpeed(f64),
    SetTorque(f64),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StateResponse {
    Completed,
    InvalidTransition,
}

/// Returned when an in-flight move is cancelled by a state command (e.g. disable, home).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cancelled;
