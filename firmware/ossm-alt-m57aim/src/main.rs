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
use embassy_time::Delay;
use embassy_time::{Duration, Ticker, Timer};
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::{Blocking, gpio::Output, interrupt::Priority, uart::Uart};
use esp_rtos::embassy::InterruptExecutor;
use m57aim_motor::M57AIMMotor;
use ossm_alt_board::{OssmAltBoard, Rs485};
use sossm::{
    CommandChannel, HomingSignal, MechanicalConfig, MotionController, MotionLimits, Sossm,
};
use static_cell::StaticCell;

use {esp_backtrace as _, esp_println as _};

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

const UPDATE_INTERVAL_SECS: f64 = 0.01;

type ConcreteMotor = M57AIMMotor<Rs485<Uart<'static, Blocking>, Output<'static>>, Delay>;

static COMMANDS: CommandChannel = CommandChannel::new();
static HOMING_DONE: HomingSignal = HomingSignal::new();
static EXECUTOR_HIGH: StaticCell<InterruptExecutor<1>> = StaticCell::new();

#[embassy_executor::task]
async fn motion_task(mut controller: MotionController<'static, ConcreteMotor>) {
    let interval_us = (UPDATE_INTERVAL_SECS * 1_000_000.0) as u64;
    let mut ticker = Ticker::every(Duration::from_micros(interval_us));

    loop {
        controller.update().await;
        ticker.next().await;
    }
}

#[esp_rtos::main]
async fn main(_spawner: Spawner) {
    esp_println::logger::init_logger_from_env();

    let p = esp_hal::init(esp_hal::Config::default());

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);

    // Start Embassy runtime
    let timg0 = TimerGroup::new(p.TIMG0);
    esp_rtos::start(timg0.timer0);

    // Board initialises motor peripherals only
    let board = OssmAltBoard::<ConcreteMotor>::new(
        p.UART1,
        p.GPIO10,
        p.GPIO12,
        p.GPIO11,
        MechanicalConfig::default(),
    );
    let config = board.mechanical_config().clone();

    let (sossm, controller) = Sossm::new(
        board.into_motor(),
        &config,
        MotionLimits::default(),
        UPDATE_INTERVAL_SECS,
        &COMMANDS,
        &HOMING_DONE,
    );

    // Spawn the motion controller on a high-priority interrupt executor
    let sw_ints = SoftwareInterruptControl::new(p.SW_INTERRUPT);
    let executor = EXECUTOR_HIGH.init(InterruptExecutor::new(sw_ints.software_interrupt1));
    let high_spawner = executor.start(Priority::Priority2);
    high_spawner.spawn(motion_task(controller)).unwrap();

    info!(
        "Motion task started at {}ms interval",
        sossm.update_interval_secs() * 1000.0
    );

    sossm.enable();
    sossm.home().await;

    // Hello world
    sossm.set_speed(150.0);
    sossm.move_to(100.0);

    loop {
        Timer::after(Duration::from_secs(5)).await;
        info!("main loop heartbeat");
    }
}
