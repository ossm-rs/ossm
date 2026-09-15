use crate::{ColorIndicator, Indicator};
use smart_leds::{RGB8, SmartLedsWrite, colors};

/// One smart LED with remembered color.
/// The supplied writer owns pixel encoding and hardware transmission.
/// Callers are responsible for leaving the protocol's reset/latch interval
/// between writes.
pub struct SmartLed<W> {
    writer: W,
    color: RGB8,
    on: bool,
}

impl<W> Indicator for SmartLed<W>
where
    W: SmartLedsWrite,
    W::Color: From<RGB8>,
    W::Error: core::fmt::Debug,
{
    type Error = W::Error;

    fn set_on(&mut self, on: bool) -> Result<(), Self::Error> {
        self.write(if on { self.color } else { colors::BLACK })?;
        self.on = on;
        Ok(())
    }
}

impl<W> ColorIndicator for SmartLed<W>
where
    W: SmartLedsWrite,
    W::Color: From<RGB8>,
    W::Error: core::fmt::Debug,
{
    fn set_color(&mut self, color: RGB8) -> Result<(), Self::Error> {
        if self.on {
            self.write(color)?;
        }
        self.color = color;
        Ok(())
    }
}

impl<W> SmartLed<W>
where
    W: SmartLedsWrite,
    W::Color: From<RGB8>,
{
    /// Clear the LED, then retain the initial color without displaying it.
    pub fn new(writer: W, color: RGB8) -> Result<Self, W::Error> {
        let mut led = Self {
            writer,
            color,
            on: false,
        };
        led.write(colors::BLACK)?;
        Ok(led)
    }

    fn write(&mut self, color: RGB8) -> Result<(), W::Error> {
        self.writer.write([color])
    }
}
