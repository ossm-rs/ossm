/// AXP2101 Power Management Unit initialization for M5Stack CoreS3.
///
/// Configures the DLDO1 rail to power the LCD backlight.
/// Uses raw I2C register writes to avoid dependency on external crates
/// that may not yet support embedded-hal 1.0.
use embedded_hal::i2c::I2c;
use log::info;

const AXP2101_ADDR: u8 = 0x34;

// Register addresses
const REG_DLDO1_VOLTAGE: u8 = 0x99;
const REG_DLDO_ENABLE: u8 = 0x90;

/// Initialize the AXP2101 PMU, enabling DLDO1 at ~3.3V for the LCD backlight.
pub fn init<I: I2c>(i2c: &mut I) {
    // Set DLDO1 voltage to 3.3V (0x1C = 3300mV in 100mV steps from 500mV)
    // Voltage = 500 + (value * 100) mV, so 0x1C (28) = 500 + 2800 = 3300mV
    let _ = i2c.write(AXP2101_ADDR, &[REG_DLDO1_VOLTAGE, 0x1C]);

    // Enable DLDO1: read current enable register, set bit 7
    let mut buf = [0u8; 1];
    let _ = i2c.write_read(AXP2101_ADDR, &[REG_DLDO_ENABLE], &mut buf);
    let _ = i2c.write(AXP2101_ADDR, &[REG_DLDO_ENABLE, buf[0] | 0x80]);

    info!("AXP2101: DLDO1 enabled at 3.3V (LCD backlight)");
}
