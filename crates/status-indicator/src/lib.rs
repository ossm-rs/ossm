#![no_std]

//! Immediate indicator capabilities, steady status policy, and portable runtime.

use core::fmt::Debug;
pub use smart_leds::RGB8;

mod smartled;
pub use smartled::SmartLed;

#[cfg(feature = "runtime")]
pub mod policy;
#[cfg(feature = "runtime")]
pub mod runtime;

/// Brightness applied uniformly to normal colors and panic red.
pub const MAX_BRIGHTNESS: u8 = 255;
pub const PANIC_COLOR: RGB8 = RGB8::new(MAX_BRIGHTNESS, 0, 0);

/// Immediate on/off output. Turning off preserves the selected color, if any.
pub trait Indicator {
    type Error: Debug;

    fn set_on(&mut self, on: bool) -> Result<(), Self::Error>;
}

/// Color capability for indicators that remember their color while off.
pub trait ColorIndicator: Indicator {
    /// Apply immediately when on; otherwise remember without lighting up.
    fn set_color(&mut self, color: RGB8) -> Result<(), Self::Error>;
}
