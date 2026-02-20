#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use embassy_executor::Spawner;
use m57aim_motor::M57AIMMotor;
use ossm_alt_board::OssmAltBoard;
use sossm::{Board, MechanicalConfig};

use {esp_backtrace as _, esp_println as _};

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_spawner: Spawner) {
    let mechanical_config = MechanicalConfig::default();
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let mut board = OssmAltBoard::<M57AIMMotor<_>>::new(peripherals, mechanical_config);

    let _ = &board.move_to(10_f32).await;

    loop {}
}
