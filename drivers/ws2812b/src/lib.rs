#![no_std]

//! A single WS2812B status indicator with remembered color and on/off control.

use core::fmt::Debug;
use status_indicator::{ColorIndicator, Indicator, PANIC_COLOR, PanicIndicator, Rgb};

/// The hardware boundary for one WS2812B pixel.
#[allow(async_fn_in_trait)]
pub trait PixelWriter {
    type Error: Debug;
    type Panic: PanicPixelWriter;

    /// Extract an independently owned emergency transport.
    fn take_panic_writer(&mut self) -> Option<Self::Panic>;

    /// Transmit one green/red/blue pixel, MSB first, using WS2812B timing.
    /// Success means the complete frame and reset/latch interval have finished.
    async fn write(&mut self, grb: [u8; 3]) -> Result<(), Self::Error>;
}

/// Emergency hardware boundary, independent of any outstanding normal write.
pub trait PanicPixelWriter {
    type Error: Debug;

    /// Override normal output with a complete GRB frame and latch interval.
    /// Must be bounded and synchronous, and prevent normal writes from
    /// replacing the panic pixel until reset, including in-flight writes.
    fn write_panic(&mut self, grb: [u8; 3]) -> Result<(), Self::Error>;
}

pub struct Ws2812bPanic<W> {
    writer: W,
}

impl<W: PanicPixelWriter> PanicIndicator for Ws2812bPanic<W> {
    type Error = W::Error;

    fn indicate_panic(&mut self) -> Result<(), Self::Error> {
        self.writer
            .write_panic([PANIC_COLOR.green, PANIC_COLOR.red, PANIC_COLOR.blue])
    }
}

/// Controls one LED. The caller supplies its transport and initial on-color.
///
/// Failed or cancelled writes preserve the last successfully requested logical
/// state; the physical output may be unknown. A subsequent `set_on` always writes
/// a complete frame, even if the requested on/off state has not changed.
pub struct Ws2812b<W: PixelWriter> {
    writer: W,
    panic: Option<Ws2812bPanic<W::Panic>>,
    color: Rgb,
    on: bool,
}

impl<W: PixelWriter> Ws2812b<W> {
    /// Clear the physical LED before returning an initially-off indicator.
    pub async fn new(mut writer: W, color: Rgb) -> Result<Self, W::Error> {
        writer.write([0, 0, 0]).await?;
        let panic = writer
            .take_panic_writer()
            .map(|writer| Ws2812bPanic { writer });
        Ok(Self {
            writer,
            panic,
            color,
            on: false,
        })
    }

    async fn write_color(&mut self, color: Rgb) -> Result<(), W::Error> {
        self.writer
            .write([color.green, color.red, color.blue])
            .await
    }
}

impl<W: PixelWriter> Indicator for Ws2812b<W> {
    type Error = W::Error;
    type Panic = Ws2812bPanic<W::Panic>;

    fn take_panic_indicator(&mut self) -> Option<Self::Panic> {
        self.panic.take()
    }

    async fn set_on(&mut self, on: bool) -> Result<(), Self::Error> {
        let color = if on { self.color } else { Rgb::BLACK };
        self.write_color(color).await?;
        self.on = on;
        Ok(())
    }
}

impl<W: PixelWriter> ColorIndicator for Ws2812b<W> {
    async fn set_color(&mut self, color: Rgb) -> Result<(), Self::Error> {
        if self.on {
            self.write_color(color).await?;
        }
        self.color = color;
        Ok(())
    }
}
