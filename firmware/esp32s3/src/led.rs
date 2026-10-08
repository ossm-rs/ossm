//! Compose the ESP GPIO LED adapter with the portable diagnostic runtime.

use embassy_executor::Spawner;
use ossm::MotionObserver;
use ossm_esp::indicator::led;
use pattern_engine::PatternObserver;
use status_indicator::runtime;

pub fn build(config: Option<led::Config<'static>>) -> Option<led::Indicator> {
    config.map(led::build)
}

pub fn start(
    spawner: &Spawner,
    led: Option<led::Indicator>,
    _motion: MotionObserver,
    _engine: PatternObserver,
) {
    if let Some(led) = led {
        if let Err(error) = spawner.spawn(diagnostic_task(led)) {
            log::warn!("Diagnostic indicator task could not start: {:?}", error);
        }
    }
}

#[embassy_executor::task]
async fn diagnostic_task(led: led::Indicator) {
    runtime::run_diagnostic(led).await;
}
