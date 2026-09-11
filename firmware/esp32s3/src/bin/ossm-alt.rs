#![no_std]
#![no_main]

use {esp_backtrace as _, esp_println as _};

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(spawner: embassy_executor::Spawner) {
    let p = esp_hal::init(esp_hal::Config::default());

    #[cfg(feature = "indicator-ws2812b")]
    let indicator = Some(ossm_esp::indicator::Config {
        rmt: p.RMT,
        data: p.GPIO38.into(),
    });
    #[cfg(not(feature = "indicator-ws2812b"))]
    let indicator = esp32s3::IndicatorConfig::default();

    let config = esp32s3::Config {
        indicator,
        motor: esp32s3::MotorConfig {
            uart1: p.UART1,
            uart_tx: p.GPIO10.into(),
            uart_rx: p.GPIO12.into(),
            rs485_de: p.GPIO11.into(),
        },
        wifi: p.WIFI,
        bt: p.BT,
        timg0: p.TIMG0,
        sw_int: p.SW_INTERRUPT,
        cpu_ctrl: p.CPU_CTRL,
    };

    esp32s3::run(spawner, config).await;
}
