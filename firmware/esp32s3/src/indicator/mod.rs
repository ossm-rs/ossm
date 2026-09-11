//! Keep feature selection out of shared configuration and startup.
#[cfg(not(feature = "indicator-ws2812b"))]
mod absent;
#[cfg(feature = "indicator-ws2812b")]
mod ws2812b;

#[cfg(not(feature = "indicator-ws2812b"))]
pub use absent::{Config, build, start};
#[cfg(feature = "indicator-ws2812b")]
pub use ws2812b::{Config, build, start};
