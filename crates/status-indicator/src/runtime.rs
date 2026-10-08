//! Hardware-independent startup and polling for indicators.
//!
//! Each indicator runs exactly one runtime: [`run_status`] for color
//! indicators, which show status and diagnostic codes, or [`run_diagnostic`]
//! for on/off indicators, which show diagnostic codes only.

use embassy_time::{Duration, Instant, Ticker};
use ossm::{MotionObserver, fault};
use pattern_engine::PatternObserver;

use crate::{
    ColorIndicator, Indicator, PANIC_COLOR, diagnostic,
    policy::{Output, POLL_INTERVAL_MS, Status, select},
};

const FAILURE_LOG_INTERVAL: Duration = Duration::from_secs(5);

/// Prepare a color indicator for [`run_status`]: apply booting immediately;
/// normal status takes over on the first poll.
pub fn initialize<I: ColorIndicator>(indicator: I) -> Output<I> {
    let mut output = Output::new(indicator);
    if let Err(error) = output.apply(Status::Booting) {
        log::warn!("Status indicator initial output failed: {:?}", error);
    }
    output
}

/// Poll status and retry failed writes at the shared cadence. A raised fault
/// replaces status with its diagnostic code until restart.
/// The platform supplies the concrete task and a clock for Embassy time.
pub async fn run_status<I: ColorIndicator>(
    mut output: Output<I>,
    motion: MotionObserver,
    engine: PatternObserver,
) -> ! {
    let mut ticker = Ticker::every(Duration::from_millis(POLL_INTERVAL_MS));
    let mut next_failure_log = Instant::now() + FAILURE_LOG_INTERVAL;
    loop {
        ticker.next().await;
        if let Some(fault) = fault::raised() {
            log::error!("Showing diagnostic code for {:?}; restart required", fault);
            let mut indicator = output.into_inner();
            // Off first so the color change is remembered rather than shown.
            let _ = indicator.set_on(false);
            let _ = indicator.set_color(PANIC_COLOR);
            diagnostic::blink(&mut indicator, fault).await;
        }
        let desired = select(engine.state(), motion.state().phase);
        if let Err(error) = output.apply(desired) {
            let now = Instant::now();
            if now >= next_failure_log {
                log::warn!("Status indicator write failed; retrying: {:?}", error);
                next_failure_log = now + FAILURE_LOG_INTERVAL;
            }
        }
    }
}

/// Stay dark until a fault is raised, then show its diagnostic code until
/// restart. For indicators that cannot show steady status.
pub async fn run_diagnostic<I: Indicator>(mut indicator: I) -> ! {
    let mut ticker = Ticker::every(Duration::from_millis(POLL_INTERVAL_MS));
    loop {
        if let Some(fault) = fault::raised() {
            log::error!("Showing diagnostic code for {:?}; restart required", fault);
            diagnostic::blink(&mut indicator, fault).await;
        }
        ticker.next().await;
    }
}
