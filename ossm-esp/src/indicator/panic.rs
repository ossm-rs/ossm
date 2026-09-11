//! Dedicated RMT channel 1 panic transport. No executor, ISR, allocator, or lock
//! participates in the emergency write. Channel 0 can continue running without
//! changing the LED once the GPIO matrix selects channel 1.

use esp_hal::{
    Async,
    gpio::OutputSignal,
    peripherals::{GPIO, RMT},
    rmt::{Channel, PulseCode, Tx},
};
use ws2812b_indicator::PanicPixelWriter;

// RMT channel memory layout from esp-hal's esp-metadata device definitions.
#[cfg(feature = "esp32s3")]
const CHANNEL_RAM: *mut PulseCode = (0x6001_6800 + 48 * 4) as *mut PulseCode;
#[cfg(feature = "esp32")]
const CHANNEL_RAM: *mut PulseCode = (0x3ff5_6800 + 64 * 4) as *mut PulseCode;

// A fixed number of peripheral reads bounds the attempt even if timers or RMT
// stop working. A normal 24-bit frame plus both latch intervals takes < 1 ms.
const POLL_LIMIT: usize = 100_000;

#[derive(Debug)]
pub enum PanicWriteError {
    AlreadyAttempted,
    Transmission,
    Timeout,
}

pub struct RmtPanicWriter<'d> {
    // Retain the HAL guard to keep channel 1 and the peripheral clock alive.
    _channel: Channel<'d, Async, Tx>,
    pin: u8,
    attempted: bool,
}

impl<'d> RmtPanicWriter<'d> {
    pub(super) fn new(channel: Channel<'d, Async, Tx>, pin: u8) -> Self {
        Self {
            _channel: channel,
            pin,
            attempted: false,
        }
    }
}

impl PanicPixelWriter for RmtPanicWriter<'_> {
    type Error = PanicWriteError;

    fn write_panic(&mut self, grb: [u8; 3]) -> Result<(), Self::Error> {
        if self.attempted {
            return Err(PanicWriteError::AlreadyAttempted);
        }
        self.attempted = true;
        let frame = super::frame(grb);
        for (index, code) in frame.into_iter().enumerate() {
            // SAFETY: this handle owns the otherwise unused TX channel 1.
            // Its single memory block holds all 27 entries. Normal TX uses
            // channel 0's separate block, and no TX has started on channel 1.
            unsafe { CHANNEL_RAM.add(index).write_volatile(code) };
        }

        // Channel 1 is idle low. The leading reset recovers from an interrupted
        // channel 0 bit before transmitting the panic pixel. Normal TX never
        // changes pin routing, so later writes cannot replace this output.
        GPIO::regs()
            .func_out_sel_cfg(self.pin as usize)
            .modify(|_, w| unsafe { w.out_sel().bits(OutputSignal::RMT_SIG_1 as _) });
        start();
        for _ in 0..POLL_LIMIT {
            if let Some(result) = completion() {
                return result;
            }
        }
        Err(PanicWriteError::Timeout)
    }
}

#[cfg(feature = "esp32s3")]
fn start() {
    let rmt = RMT::regs();
    rmt.int_clr()
        .write(|w| w.ch_tx_end(1).set_bit().ch_tx_err(1).set_bit());
    let config = rmt.ch_tx_conf0(1);
    // This channel has never transmitted: latch its divider, memory size and
    // idle settings before starting, matching esp-hal's start_send sequence.
    config.modify(|_, w| w.conf_update().set_bit());
    config.modify(|_, w| {
        w.mem_rd_rst()
            .set_bit()
            .apb_mem_rst()
            .set_bit()
            .tx_start()
            .set_bit()
    });
    config.modify(|_, w| w.conf_update().set_bit());
}

#[cfg(feature = "esp32")]
fn start() {
    let rmt = RMT::regs();
    rmt.int_clr()
        .write(|w| w.ch_tx_end(1).set_bit().ch_err(1).set_bit());
    rmt.chconf1(1).modify(|_, w| {
        w.mem_owner()
            .clear_bit()
            .mem_rd_rst()
            .set_bit()
            .apb_mem_rst()
            .set_bit()
            .tx_start()
            .set_bit()
    });
}

fn completion() -> Option<Result<(), PanicWriteError>> {
    let status = RMT::regs().int_raw().read();
    #[cfg(feature = "esp32s3")]
    let error = status.ch_tx_err(1).bit();
    #[cfg(feature = "esp32")]
    let error = status.ch_err(1).bit();
    if error {
        Some(Err(PanicWriteError::Transmission))
    } else if status.ch_tx_end(1).bit() {
        Some(Ok(()))
    } else {
        None
    }
}
