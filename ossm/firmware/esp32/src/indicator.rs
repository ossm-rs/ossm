//! Select indicator configuration and startup without changing board config fields.

#[cfg(feature = "indicator-ws2812b")]
#[path = "ws2812b.rs"]
mod ws2812b;

#[cfg(feature = "indicator-ws2812b")]
pub type Config = ossm_esp::indicator::ws2812b::Config<'static, 0, 1>;

#[cfg(feature = "indicator-ws2812b")]
pub use ws2812b::{StatusOutput as Output, build, start};

#[cfg(not(feature = "indicator-ws2812b"))]
pub use disabled::{Config, Output, build, start};

#[cfg(not(feature = "indicator-ws2812b"))]
mod disabled {
    use embassy_executor::Spawner;
    use ossm::motion::MotionObserver;
    use pattern_engine::PatternObserver;

    // No value can populate Some when indicator hardware support is absent.
    pub type Config = core::convert::Infallible;
    pub type Output = Config;

    pub fn build<const RMT_CHANNEL: u8, const PANIC_RMT_CHANNEL: u8>(
        config: Option<Config>,
    ) -> Option<Config> {
        config
    }

    pub fn start(
        _spawner: &Spawner,
        _output: Option<Config>,
        _motion: MotionObserver,
        _engine: PatternObserver,
    ) {
    }
}
