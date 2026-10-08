//! Blink codes for faults that only a restart resolves.
//!
//! A code is a group of blinks followed by a longer pause, repeated until
//! restart. Counts stay small so they can be read at a glance. Keep the
//! README's "Status LED colors" table in sync with [`blinks`].

use embassy_time::Timer;
use ossm::fault::Fault;

use crate::Indicator;

pub const BLINK_ON_MS: u64 = 300;
pub const BLINK_OFF_MS: u64 = 300;
/// Added to the final off interval so groups read as separate codes.
pub const CODE_PAUSE_MS: u64 = 1500;

pub fn blinks(fault: Fault) -> u8 {
    match fault {
        Fault::MotorUnresponsive => 1,
        Fault::MotorPowerCycle => 2,
        Fault::BoardSetup => 3,
    }
}

/// Repeat the fault's code forever. A color indicator shows its current
/// color, so callers select `PANIC_COLOR` beforehand. Failed writes are
/// ignored; the next blink writes again.
pub async fn blink<I: Indicator>(indicator: &mut I, fault: Fault) -> ! {
    loop {
        for _ in 0..blinks(fault) {
            let _ = indicator.set_on(true);
            Timer::after_millis(BLINK_ON_MS).await;
            let _ = indicator.set_on(false);
            Timer::after_millis(BLINK_OFF_MS).await;
        }
        Timer::after_millis(CODE_PAUSE_MS).await;
    }
}
