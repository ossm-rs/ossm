#![no_std]
#![no_main]

use {esp_backtrace as _, esp_println as _};

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(spawner: embassy_executor::Spawner) {
    let p = esp_hal::init(esp_hal::Config::default());

    let rmt = esp_hal::rmt::Rmt::new(p.RMT, esp_hal::time::Rate::from_mhz(80))
        .expect("Failed to initialize RMT");

    let config = esp32::Config {
        indicator: Some(ossm_esp::indicator::ws2812b::Config {
            channel: rmt.channel0,
            panic_channel: rmt.channel1,
            data: p.GPIO25.into(),
        }),
        motor: esp32::MotorConfig {
            pcnt: p.PCNT,
            channel: rmt.channel2,
            step: p.GPIO14.into(),
            dir: p.GPIO27.into(),
            enable: p.GPIO26.into(),
        },
        board: esp32::BoardConfig {
            adc1: p.ADC1,
            current_pin: p.GPIO36,
        },
        bt: p.BT,
        timg0: p.TIMG0,
        sw_int: p.SW_INTERRUPT,
        cpu_ctrl: p.CPU_CTRL,
    };

    esp32::run(spawner, config).await;
}
