//! Faults that only a restart resolves, reported for the status indicator.

use core::sync::atomic::{AtomicU8, Ordering};

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Fault {
    /// The motor did not respond during setup.
    MotorUnresponsive = 1,
    /// The motor accepted new settings that apply only after a power cycle.
    MotorPowerCycle = 2,
    /// Board hardware, such as a UART, could not be set up.
    BoardSetup = 3,
    /// The motor or board reported an error while running.
    Runtime = 4,
}

static RAISED: AtomicU8 = AtomicU8::new(0);
static RAISED_SIGNAL: Signal<CriticalSectionRawMutex, Fault> = Signal::new();

/// Report a fault until restart. The first fault wins as the likely root
/// cause. The caller is responsible for stopping the machine; see [`halt`].
pub fn raise(fault: Fault) {
    if RAISED
        .compare_exchange(0, fault as u8, Ordering::Relaxed, Ordering::Relaxed)
        .is_ok()
    {
        RAISED_SIGNAL.signal(fault);
    }
}

/// Raise the fault and park the calling task until restart, leaving the
/// executor free for the status indicator.
pub async fn halt(fault: Fault) -> ! {
    raise(fault);
    loop {
        core::future::pending::<()>().await;
    }
}

/// Wait until a fault is raised, returning immediately if one already is.
///
/// Only one task should wait at a time, as the underlying signal wakes a
/// single waiter. That task is the mode supervisor.
pub async fn wait() -> Fault {
    loop {
        if let Some(fault) = raised() {
            return fault;
        }
        RAISED_SIGNAL.wait().await;
    }
}

pub fn raised() -> Option<Fault> {
    match RAISED.load(Ordering::Relaxed) {
        1 => Some(Fault::MotorUnresponsive),
        2 => Some(Fault::MotorPowerCycle),
        3 => Some(Fault::BoardSetup),
        4 => Some(Fault::Runtime),
        _ => None,
    }
}
