/// ILI9342C Display initialization for M5Stack CoreS3.
///
/// Sets up SPI and initializes the display via the mipidsi driver.
/// The display reset is handled externally by the AW9523B I/O expander
/// before this module is called.
use embedded_hal_bus::spi::ExclusiveDevice;
use esp_hal::Blocking;
use esp_hal::delay::Delay;
use esp_hal::gpio::Output;
use esp_hal::spi::master::Spi;
use log::info;
use mipidsi::Builder;
use mipidsi::interface::SpiInterface;
use mipidsi::models::ILI9342CRgb565;
use mipidsi::options::{ColorInversion, ColorOrder};
use static_cell::StaticCell;

type SpiDev = ExclusiveDevice<Spi<'static, Blocking>, Output<'static>, Delay>;

pub type Display = mipidsi::Display<
    SpiInterface<'static, SpiDev, Output<'static>>,
    ILI9342CRgb565,
    mipidsi::NoResetPin,
>;

static SPI_BUFFER: StaticCell<[u8; 4_096]> = StaticCell::new();

/// Initialize the ILI9342C display over SPI.
///
/// Assumes the LCD has already been reset via the AW9523B I/O expander.
///
/// Panics if called more than once (StaticCell enforces single initialization).
pub fn init(spi: Spi<'static, Blocking>, cs: Output<'static>, dc: Output<'static>) -> Display {
    let spi_device = ExclusiveDevice::new(spi, cs, Delay::new()).expect("SPI device creation");

    let buffer = SPI_BUFFER.init([0u8; 4_096]);
    let di = SpiInterface::new(spi_device, dc, buffer);

    let mut delay = Delay::new();

    let display = Builder::new(ILI9342CRgb565, di)
        .color_order(ColorOrder::Bgr)
        .invert_colors(ColorInversion::Inverted)
        .init(&mut delay)
        .expect("Failed to initialize ILI9342C display");

    info!("ILI9342C display initialized (320x240)");
    display
}
