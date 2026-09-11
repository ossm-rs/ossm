//! Opt-in WS2812B support. No status policy or application task is installed.
//!
//! For ossm-alt, supply `GPIO38` as [`Config::data`]. Construction takes ownership
//! of RMT and uses TX channel 0 plus channel 1 for panic output; it cannot share that peripheral with the current
//! step/dir motor adapter. Boards with no indicator leave `indicator-ws2812b`
//! disabled and do not depend on either this module or the WS2812B driver.

use esp_hal::{
    Async,
    gpio::{AnyPin, Level, NoPin, Pin},
    peripherals::RMT,
    rmt::{Channel, Error, PulseCode, Rmt, Tx, TxChannelConfig, TxChannelCreator},
    time::Rate,
};
pub use status_indicator::{ColorIndicator, Indicator, Rgb};
use ws2812b_indicator::{PixelWriter, Ws2812b};

mod panic;
pub use panic::{PanicWriteError, RmtPanicWriter};

/// Resources for a single WS2812B. The ossm-alt LED's data pin is GPIO38.
pub struct Config<'d> {
    pub rmt: RMT<'d>,
    pub data: AnyPin<'d>,
}

pub type Ws2812bIndicator<'d> = Ws2812b<RmtPixelWriter<'d>>;

/// Configure the transport and clear the LED, retaining the caller's on-color.
/// Peripheral configuration and initial transmission errors are returned.
pub async fn build(config: Config<'_>, color: Rgb) -> Result<Ws2812bIndicator<'_>, Error> {
    let rmt = Rmt::new(config.rmt, Rate::from_mhz(80))?.into_async();
    let tx_config = TxChannelConfig::default()
        .with_clk_divider(1)
        .with_idle_output(true)
        .with_idle_output_level(Level::Low);
    // Keep emergency output physically separate from normal TX, including a
    // normal write interrupted on the other core. Route it to the pin only on
    // panic. No duplicated HAL pin or channel ownership is needed.
    let panic_channel = rmt.channel1.configure_tx(NoPin, tx_config)?;
    let panic = RmtPanicWriter::new(panic_channel, config.data.number());
    let channel = rmt.channel0.configure_tx(config.data, tx_config)?;
    Ws2812b::new(
        RmtPixelWriter {
            channel,
            panic: Some(panic),
        },
        color,
    )
    .await
}

/// RMT transport configured by [`build`], with no timer or heap allocation.
pub struct RmtPixelWriter<'d> {
    channel: Channel<'d, Async, Tx>,
    panic: Option<RmtPanicWriter<'d>>,
}

// At 80 MHz / 1, each tick is 12.5 ns. These timings lie within both Worldsemi's
// original WS2812B and WS2812B-V5 ranges (including V5's longer T1L and reset):
// https://cdn-shop.adafruit.com/datasheets/WS2812B.pdf (page 3)
// https://www.world-semi.co.kr/_files/ugd/89cd03_1023b0e9d135431aa1e6491bfc318112.pdf (page 3)
// Zero: 350 ns high, 900 ns low. One: 700 ns high, 587.5 ns low.
const ZERO: PulseCode = PulseCode::new(Level::High, 28, Level::Low, 72);
const ONE: PulseCode = PulseCode::new(Level::High, 56, Level::Low, 47);
// Two 150 us low halves provide a 300 us reset (>280 us).
const RESET: PulseCode = PulseCode::new(Level::Low, 12_000, Level::Low, 12_000);

impl<'d> PixelWriter for RmtPixelWriter<'d> {
    type Error = Error;
    type Panic = RmtPanicWriter<'d>;

    fn take_panic_writer(&mut self) -> Option<Self::Panic> {
        self.panic.take()
    }

    async fn write(&mut self, grb: [u8; 3]) -> Result<(), Self::Error> {
        // A leading reset recovers framing after startup or a cancelled/failed
        // transmission; the trailing reset latches the pixel before returning.
        // 27 entries fit in a single RMT memory block on ESP32 and ESP32-S3.
        let frame = frame(grb);
        self.channel.transmit(&frame).await
    }
}

fn frame(grb: [u8; 3]) -> [PulseCode; 27] {
    let mut frame = [PulseCode::end_marker(); 27];
    frame[0] = RESET;
    for (byte_index, byte) in grb.into_iter().enumerate() {
        for bit_index in 0..8 {
            frame[1 + byte_index * 8 + bit_index] = if byte & (0x80 >> bit_index) == 0 {
                ZERO
            } else {
                ONE
            };
        }
    }
    frame[25] = RESET;
    frame
}
