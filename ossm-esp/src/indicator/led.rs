//! Single-color GPIO LED for diagnostic codes and steady panic indication.

use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use esp_hal::gpio::{AnyPin, Level, Output, OutputConfig, Pin as _};

pub struct Config<'d> {
    pub pin: AnyPin<'d>,
    /// Level that lights the LED.
    pub on: Level,
}

pub struct Indicator {
    output: Output<'static>,
    on: Level,
}

impl status_indicator::Indicator for Indicator {
    type Error = core::convert::Infallible;

    fn set_on(&mut self, on: bool) -> Result<(), Self::Error> {
        self.output.set_level(if on { self.on } else { !self.on });
        Ok(())
    }
}

const NO_PIN: u8 = u8::MAX;
static PANIC_PIN: AtomicU8 = AtomicU8::new(NO_PIN);
static PANIC_ON_HIGH: AtomicBool = AtomicBool::new(false);

// esp-backtrace invokes this only for Rust panics, before diagnostics and halt.
// Light the LED steadily; nothing here acquires an application lock.
#[unsafe(no_mangle)]
pub extern "Rust" fn custom_pre_backtrace() {
    let pin = PANIC_PIN.swap(NO_PIN, Ordering::AcqRel);
    if pin != NO_PIN {
        let on = Level::from(PANIC_ON_HIGH.load(Ordering::Relaxed));
        // SAFETY: the pin was configured as this LED's output in `build`. The
        // panic takes it over once; normal code never runs again after halt,
        // and Output has no Drop that would reconfigure the pin.
        let _ = Output::new(unsafe { AnyPin::steal(pin) }, on, OutputConfig::default());
    }
}

/// Turn the LED off and register its panic output.
/// Firmware using this adapter enables `esp-backtrace/custom-pre-backtrace`.
pub fn build(config: Config<'static>) -> Indicator {
    let pin = config.pin.number();
    let output = Output::new(config.pin, !config.on, OutputConfig::default());
    PANIC_ON_HIGH.store(config.on == Level::High, Ordering::Relaxed);
    PANIC_PIN.store(pin, Ordering::Release);
    Indicator {
        output,
        on: config.on,
    }
}
