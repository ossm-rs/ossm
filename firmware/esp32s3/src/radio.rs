use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use esp_hal::peripherals::{BT, WIFI};
use esp_radio::ble::controller::BleConnector;
use esp_radio::esp_now::{EspNowManager, EspNowSender};
use log::info;
use ossm::MotionLimits;
use ossm_m5_remote::RemoteConfig;
use pattern_engine::PatternEngine;

use crate::mk_static;

pub fn start(
    spawner: &Spawner,
    wifi: WIFI<'static>,
    bt: BT<'static>,
    patterns: &'static PatternEngine,
    limits: &MotionLimits,
) {
    let radio = &*mk_static!(
        esp_radio::Controller<'static>,
        esp_radio::init().expect("Failed to initialize radio controller")
    );

    let (wifi_controller, interfaces) =
        esp_radio::wifi::new(radio, wifi, Default::default()).unwrap();

    let wifi_controller = mk_static!(esp_radio::wifi::WifiController<'static>, wifi_controller);

    wifi_controller
        .set_mode(esp_radio::wifi::WifiMode::Sta)
        .unwrap();
    wifi_controller.start().unwrap();

    let esp_now = interfaces.esp_now;
    info!("ESP-NOW version {}", esp_now.version().unwrap());

    let (manager, sender, receiver) = esp_now.split();
    let manager = mk_static!(EspNowManager<'static>, manager);
    let sender = mk_static!(
        Mutex::<NoopRawMutex, EspNowSender<'static>>,
        Mutex::<NoopRawMutex, _>::new(sender)
    );

    let remote_config = RemoteConfig {
        max_velocity_mm_s: limits.max_velocity_mm_s,
        max_travel_mm: limits.max_position_mm - limits.min_position_mm,
    };

    ossm_m5_remote::start(spawner, manager, sender, receiver, remote_config);

    let connector =
        BleConnector::new(radio, bt, Default::default()).expect("Could not create BleConnector");
    ble_remote::start(spawner, connector, patterns);
}
