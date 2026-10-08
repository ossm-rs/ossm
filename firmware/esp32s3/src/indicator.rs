//! Select indicator configuration and startup without changing board config fields.

#[cfg(all(feature = "indicator-ws2812b", feature = "indicator-led"))]
compile_error!("Enable at most one indicator feature.");

#[cfg(feature = "indicator-ws2812b")]
#[path = "ws2812b.rs"]
mod ws2812b;

#[cfg(feature = "indicator-ws2812b")]
pub type Config = ossm_esp::indicator::ws2812b::Config<'static, 0, 1>;

#[cfg(feature = "indicator-ws2812b")]
pub use ws2812b::{build, start};

#[cfg(feature = "indicator-led")]
#[path = "led.rs"]
mod led;

#[cfg(feature = "indicator-led")]
pub type Config = ossm_esp::indicator::led::Config<'static>;

#[cfg(feature = "indicator-led")]
pub use led::{build, start};

#[cfg(not(any(feature = "indicator-ws2812b", feature = "indicator-led")))]
pub use disabled::{Config, build, start};

#[cfg(not(any(feature = "indicator-ws2812b", feature = "indicator-led")))]
mod disabled {
    use embassy_executor::Spawner;
    use ossm::MotionObserver;
    use pattern_engine::PatternObserver;

    // No value can populate Some when indicator hardware support is absent.
    pub type Config = core::convert::Infallible;

    pub fn build(config: Option<Config>) -> Option<Config> {
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
