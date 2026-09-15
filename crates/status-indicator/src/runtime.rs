//! Hardware-independent startup and polling for any color indicator.

use embassy_time::{Duration, Instant, Ticker};
use ossm::MotionObserver;
use pattern_engine::PatternObserver;

use crate::{
    ColorIndicator,
    policy::{Output, POLL_INTERVAL_MS, Status, select},
};

const FAILURE_LOG_INTERVAL: Duration = Duration::from_secs(5);

/// Apply booting immediately; normal status takes over on the first poll.
pub fn initialize<I: ColorIndicator>(indicator: I) -> Output<I> {
    let mut output = Output::new(indicator);
    if let Err(error) = output.apply(Status::Booting) {
        log::warn!("Status indicator initial output failed: {:?}", error);
    }
    output
}

/// Poll status and retry failed writes at the shared cadence.
/// The platform supplies the concrete task and a clock for Embassy time.
pub async fn run<I: ColorIndicator>(
    mut output: Output<I>,
    motion: MotionObserver,
    engine: PatternObserver,
) -> ! {
    let mut ticker = Ticker::every(Duration::from_millis(POLL_INTERVAL_MS));
    let mut next_failure_log = Instant::now() + FAILURE_LOG_INTERVAL;
    loop {
        ticker.next().await;
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
