//! Compose the ESP WS2812B adapter with the portable status runtime.

use embassy_executor::Spawner;
use esp_hal::{
    Blocking,
    rmt::{ChannelCreator, TxChannelCreator},
};
use ossm::MotionObserver;
use ossm_esp::indicator::ws2812b;
use pattern_engine::PatternObserver;
use status_indicator::{policy::Output, runtime};

pub type StatusOutput = Output<ws2812b::Indicator>;

pub fn build<const CHANNEL: u8, const PANIC_CHANNEL: u8>(
    config: Option<ws2812b::Config<'static, CHANNEL, PANIC_CHANNEL>>,
) -> Option<StatusOutput>
where
    ChannelCreator<'static, Blocking, CHANNEL>: TxChannelCreator<'static, Blocking>,
    ChannelCreator<'static, Blocking, PANIC_CHANNEL>: TxChannelCreator<'static, Blocking>,
{
    match ws2812b::build(config?) {
        Ok(indicator) => Some(runtime::initialize(indicator)),
        Err(error) => {
            log::warn!("Status indicator initialization failed: {:?}", error);
            None
        }
    }
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
async fn status_task(output: StatusOutput, motion: MotionObserver, engine: PatternObserver) {
    runtime::run(output, motion, engine).await;
}
