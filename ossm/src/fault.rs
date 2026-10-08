//! Faults that only a restart resolves, reported for the status indicator.

use core::sync::atomic::{AtomicU8, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Fault {
    /// The motor did not respond during setup.
    MotorUnresponsive = 1,
    /// The motor accepted new settings that apply only after a power cycle.
    MotorPowerCycle = 2,
    /// Board hardware, such as a UART, could not be set up.
    BoardSetup = 3,
}

static RAISED: AtomicU8 = AtomicU8::new(0);

/// Report a fault until restart. The first fault wins as the likely root
/// cause. The caller is responsible for stopping the machine; see [`halt`].
pub fn raise(fault: Fault) {
    let _ = RAISED.compare_exchange(0, fault as u8, Ordering::Relaxed, Ordering::Relaxed);
}

/// Raise the fault and park the calling task until restart, leaving the
/// executor free for the status indicator.
pub async fn halt(fault: Fault) -> ! {
    raise(fault);
    loop {
        core::future::pending::<()>().await;
    }
}

pub fn raised() -> Option<Fault> {
    match RAISED.load(Ordering::Relaxed) {
        1 => Some(Fault::MotorUnresponsive),
        2 => Some(Fault::MotorPowerCycle),
        3 => Some(Fault::BoardSetup),
        _ => None,
    }
}
