/// AW9523B I/O Expander initialization for M5Stack CoreS3.
///
/// Configures pin 9 (port 1, bit 1) as output and performs a reset pulse
/// for the ILI9342C LCD controller.
/// Uses raw I2C register writes to avoid dependency on external crates.
use embedded_hal::delay::DelayNs;
use embedded_hal::i2c::I2c;
use log::info;

const AW9523B_ADDR: u8 = 0x58;

// Register addresses
const REG_OUTPUT_PORT1: u8 = 0x03;
const REG_CONFIG_PORT1: u8 = 0x05;
const REG_CTL: u8 = 0x11; // Global control: push-pull mode

// Pin assignments on the AW9523B
const LCD_RESET_BIT: u8 = 1 << 1; // Port 1, pin 1 = physical pin 9

/// Initialize the AW9523B and perform the LCD reset sequence.
///
/// The LCD reset pin is on port 1, bit 1 (pin 9 of the AW9523B).
/// Reset sequence: drive low → wait 10ms → drive high → wait 50ms.
pub fn init<I: I2c, D: DelayNs>(i2c: &mut I, delay: &mut D) {
    // Set push-pull mode for port 1 (register 0x11, bit 4 = 0 for push-pull)
    let mut buf = [0u8; 1];
    let _ = i2c.write_read(AW9523B_ADDR, &[REG_CTL], &mut buf);
    let _ = i2c.write(AW9523B_ADDR, &[REG_CTL, buf[0] & !0x10]);

    // Configure port 1 pin 1 as output (0 = output in config register)
    let _ = i2c.write_read(AW9523B_ADDR, &[REG_CONFIG_PORT1], &mut buf);
    let _ = i2c.write(AW9523B_ADDR, &[REG_CONFIG_PORT1, buf[0] & !LCD_RESET_BIT]);

    // LCD reset sequence: pull low → delay → pull high → delay
    let _ = i2c.write_read(AW9523B_ADDR, &[REG_OUTPUT_PORT1], &mut buf);
    let _ = i2c.write(AW9523B_ADDR, &[REG_OUTPUT_PORT1, buf[0] & !LCD_RESET_BIT]);
    delay.delay_ms(10);
    let _ = i2c.write_read(AW9523B_ADDR, &[REG_OUTPUT_PORT1], &mut buf);
    let _ = i2c.write(AW9523B_ADDR, &[REG_OUTPUT_PORT1, buf[0] | LCD_RESET_BIT]);
    delay.delay_ms(50);

    info!("AW9523B: LCD reset complete");
}
