use core::{
    ptr,
    sync::atomic::{AtomicPtr, Ordering},
};
use embassy_executor::Spawner;
use embassy_time::{Duration, Instant, Ticker};
use ossm::MotionObserver;
use ossm_esp::indicator::{self, Ws2812bIndicator};
use pattern_engine::PatternObserver;
use static_cell::StaticCell;
use status_indicator::policy::{Output, POLL_INTERVAL_MS, Status, color, select};
use status_indicator::{Indicator, PanicIndicator};

type PanicOutput = <Ws2812bIndicator<'static> as Indicator>::Panic;
static PANIC_STORAGE: StaticCell<PanicOutput> = StaticCell::new();
static PANIC_OUTPUT: AtomicPtr<PanicOutput> = AtomicPtr::new(ptr::null_mut());

// esp-backtrace invokes this only for Rust panics, before diagnostics and halt.
// Taking the pointer grants one caller exclusive access, including nested or
// simultaneous panics. Nothing here acquires an application lock.
#[unsafe(no_mangle)]
pub extern "Rust" fn custom_pre_backtrace() {
    let output = PANIC_OUTPUT.swap(ptr::null_mut(), Ordering::AcqRel);
    if !output.is_null() {
        // SAFETY: published after initialization, stored for the firmware's
        // lifetime, and removed atomically before creating the sole &mut.
        let _ = unsafe { &mut *output }.indicate_panic();
    }
}

// Other boards may leave this absent even when the package feature is enabled.
pub type Config = Option<indicator::Config<'static>>;
pub type StatusOutput = Output<Ws2812bIndicator<'static>>;
const FAILURE_LOG_INTERVAL: Duration = Duration::from_secs(5);

pub async fn build(config: Config) -> Option<StatusOutput> {
    let config = config?;
    let mut indicator = match indicator::build(config, color(Status::Idle)).await {
        Ok(indicator) => indicator,
        Err(error) => {
            log::warn!("Status indicator initialization failed: {:?}", error);
            return None;
        }
    };
    if let Some(panic) = indicator.take_panic_indicator() {
        PANIC_OUTPUT.store(PANIC_STORAGE.init(panic), Ordering::Release);
    }
    let mut output = Output::new(indicator);
    if let Err(error) = output.apply(Status::Idle).await {
        // Retain the initialized transport so the task can retry turn-on.
        log::warn!("Status indicator initial output failed: {:?}", error);
    }
    Some(output)
}

pub fn start(
    spawner: &Spawner,
    output: Option<StatusOutput>,
    motion: MotionObserver,
    engine: PatternObserver,
) {
    if let Some(output) = output {
        if let Err(error) = spawner.spawn(status_task(output, motion, engine)) {
            log::warn!("Status indicator task could not start: {:?}", error);
        }
    }
}

#[embassy_executor::task]
async fn status_task(mut output: StatusOutput, motion: MotionObserver, engine: PatternObserver) {
    let mut ticker = Ticker::every(Duration::from_millis(POLL_INTERVAL_MS));
    let mut next_failure_log = Instant::now() + FAILURE_LOG_INTERVAL;
    loop {
        ticker.next().await;
        let desired = select(engine.state(), motion.state().phase);
        if let Err(error) = output.apply(desired).await {
            let now = Instant::now();
            if now >= next_failure_log {
                log::warn!("Status indicator write failed; retrying: {:?}", error);
                next_failure_log = now + FAILURE_LOG_INTERVAL;
            }
        }
    }
}
