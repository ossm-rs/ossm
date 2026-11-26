#![no_std]
#![no_main]

use {esp_backtrace as _, esp_println as _};
use ossm::{Ossm, OssmMode, patterns};

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(spawner: embassy_executor::Spawner) {
    let p = esp_hal::init(esp_hal::Config::default());

    let config = esp32s3::Config {
        indicator: Some(ossm_esp::indicator::led::Config {
            pin: p.GPIO21.into(),
            on: esp_hal::gpio::Level::Low,
        }),
        motor: esp32s3::MotorConfig {
            uart1: p.UART1,
            uart_tx: p.GPIO5.into(),
            uart_rx: p.GPIO6.into(),
            rs485_de: p.GPIO3.into(),
        },
        wifi: p.WIFI,
        bt: p.BT,
        timg0: p.TIMG0,
        sw_int: p.SW_INTERRUPT,
        cpu_ctrl: p.CPU_CTRL,
    };

    // Start in pattern mode, matching how the firmware behaved before modes.
    Ossm::new(esp32s3::Esp32s3::new(spawner, config))
        .mode(OssmMode::Patterns, patterns)
        .start_in(OssmMode::Patterns)
        .run()
        .await
}
