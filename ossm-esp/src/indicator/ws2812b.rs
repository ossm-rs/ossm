//! WS2812B status and panic indication over blocking ESP RMT channels.
//! Board composition initializes RMT at 80 MHz and allocates the channels.

use core::{
    ptr,
    sync::atomic::{AtomicPtr, Ordering},
};
use esp_hal::{
    Blocking,
    delay::Delay,
    gpio::{AnyPin, Level, NoPin, Output, OutputConfig, OutputSignal},
    rmt::{ChannelCreator, PulseCode, TxChannelCreator},
};
use esp_hal_smartled::{LedAdapterError, SmartLedsAdapter, buffer_size, smart_led_buffer};
use static_cell::StaticCell;
use status_indicator::{Indicator as _, PANIC_COLOR, RGB8, SmartLed};

const RESET_INTERVAL_US: u32 = 600;

pub struct Config<'d, const RMT_CHANNEL: u8, const PANIC_RMT_CHANNEL: u8> {
    pub channel: ChannelCreator<'d, Blocking, RMT_CHANNEL>,
    pub panic_channel: ChannelCreator<'d, Blocking, PANIC_RMT_CHANNEL>,
    pub data: AnyPin<'d>,
}

pub type Indicator = SmartLed<SmartLedsAdapter<'static, { buffer_size(1) }>>;

static PANIC_STORAGE: StaticCell<PanicIndicator> = StaticCell::new();
static PANIC_OUTPUT: AtomicPtr<PanicIndicator> = AtomicPtr::new(ptr::null_mut());

// esp-backtrace invokes this only for Rust panics, before diagnostics and halt.
// Taking the pointer grants one caller exclusive access, including nested or
// simultaneous panics. Nothing here acquires an application lock.
#[unsafe(no_mangle)]
pub extern "Rust" fn custom_pre_backtrace() {
    let output = PANIC_OUTPUT.swap(ptr::null_mut(), Ordering::AcqRel);
    if !output.is_null() {
        // SAFETY: published after initialization, stored for the firmware's
        // lifetime, and removed atomically before creating the sole &mut.
        let _ = unsafe { &mut *output }.indicate_panic();
    }
}

/// Independent panic output, registered after initialization.
/// It uses upstream blocking completion, which has no timeout.
struct PanicIndicator {
    indicator: Indicator,
    data: Output<'static>,
    signal: OutputSignal,
}

impl PanicIndicator {
    fn indicate_panic(&mut self) -> Result<(), LedAdapterError> {
        // The panic channel is idle low. Take over the pin before resetting
        // framing; normal writes cannot reconnect it or replace panic red.
        self.signal.connect_to(&self.data);
        Delay::new().delay_micros(RESET_INTERVAL_US);
        self.indicator.set_on(true)
    }
}

/// Clear the board's WS2812B and register its independent panic output.
/// Firmware using this adapter enables `esp-backtrace/custom-pre-backtrace`.
/// Channel-configuration failures follow the upstream constructor's panic
/// behavior; returned clear/write errors remain recoverable by the caller.
pub fn build<const RMT_CHANNEL: u8, const PANIC_RMT_CHANNEL: u8>(
    config: Config<'static, RMT_CHANNEL, PANIC_RMT_CHANNEL>,
) -> Result<Indicator, LedAdapterError>
where
    ChannelCreator<'static, Blocking, RMT_CHANNEL>: TxChannelCreator<'static, Blocking>,
    ChannelCreator<'static, Blocking, PANIC_RMT_CHANNEL>: TxChannelCreator<'static, Blocking>,
{
    static NORMAL_BUFFER: StaticCell<[PulseCode; buffer_size(1)]> = StaticCell::new();
    static PANIC_BUFFER: StaticCell<[PulseCode; buffer_size(1)]> = StaticCell::new();

    // Configure both adapters without a pin so the panic handle can retain
    // exclusive GPIO ownership. Normal code owns only its RMT channel.
    let normal = SmartLedsAdapter::new(
        config.channel,
        NoPin,
        NORMAL_BUFFER.init(smart_led_buffer!(1)),
    );
    let panic = SmartLedsAdapter::new(
        config.panic_channel,
        NoPin,
        PANIC_BUFFER.init(smart_led_buffer!(1)),
    );
    let panic = SmartLed::new(panic, PANIC_COLOR)?;
    let data = Output::new(config.data, Level::Low, OutputConfig::default());
    output_signal(RMT_CHANNEL).connect_to(&data);
    let delay = Delay::new();
    delay.delay_micros(RESET_INTERVAL_US);
    let normal = SmartLed::new(normal, RGB8::default())?;
    // The runtime applies the initial color immediately after build; finish the
    // clear frame's latch interval before returning.
    delay.delay_micros(RESET_INTERVAL_US);
    let panic = PANIC_STORAGE.init(PanicIndicator {
        indicator: panic,
        data,
        signal: output_signal(PANIC_RMT_CHANNEL),
    });
    PANIC_OUTPUT.store(panic, Ordering::Release);
    Ok(normal)
}

// esp-hal keeps the channel-to-signal lookup private. Mirror the TX signals for
// the supported chips; the TxChannelCreator bounds reject non-TX channels.
fn output_signal(channel: u8) -> OutputSignal {
    match channel {
        0 => OutputSignal::RMT_SIG_0,
        1 => OutputSignal::RMT_SIG_1,
        2 => OutputSignal::RMT_SIG_2,
        3 => OutputSignal::RMT_SIG_3,
        #[cfg(feature = "esp32")]
        4 => OutputSignal::RMT_SIG_4,
        #[cfg(feature = "esp32")]
        5 => OutputSignal::RMT_SIG_5,
        #[cfg(feature = "esp32")]
        6 => OutputSignal::RMT_SIG_6,
        #[cfg(feature = "esp32")]
        7 => OutputSignal::RMT_SIG_7,
        _ => unreachable!("unsupported RMT transmit channel"),
    }
}
