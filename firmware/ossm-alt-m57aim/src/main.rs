#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use log::info;

use embassy_executor::Spawner;
use embassy_time::{Duration, Ticker};
use m57aim_motor::M57AIMMotor;
use ossm_alt_board::OssmAltBoard;
use sossm::{Board, MechanicalConfig, MotionLimits, Sossm};

use {esp_backtrace as _, esp_println as _};

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

const UPDATE_INTERVAL_MS: u64 = 10;

#[esp_rtos::main]
async fn main(_spawner: Spawner) {
    esp_println::logger::init_logger_from_env();

    let peripherals = esp_hal::init(esp_hal::Config::default());

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);

    let board = OssmAltBoard::<M57AIMMotor<_, _>>::new(peripherals, MechanicalConfig::default());
    let config = board.mechanical_config().clone();

    let mut sossm = Sossm::new(
        board.into_motor(),
        &config,
        MotionLimits::default(),
        UPDATE_INTERVAL_MS as f64 / 1000.0,
    );

    sossm.enable().expect("enable failed");

    sossm.home().expect("homing failed");

    sossm.set_speed(150.0);
    sossm.move_to(100.0);

    let mut tick: u32 = 0;
    let mut ticker = Ticker::every(Duration::from_millis(UPDATE_INTERVAL_MS));
    loop {
        sossm.update().expect("motion update failed");
        tick = tick.wrapping_add(1);
        if tick % 100 == 0 {
            info!("tick {}", tick);
        }
        ticker.next().await;
    }
}
