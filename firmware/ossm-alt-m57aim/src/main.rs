#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use core::cell::RefCell;

use critical_section::Mutex;
use log::info;

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_hal::{
    Blocking,
    delay::Delay,
    gpio::Output,
    handler,
    interrupt::Priority,
    time::Duration as HalDuration,
    timer::{PeriodicTimer, timg::TimerGroup},
    uart::Uart,
};
use m57aim_motor::M57AIMMotor;
use ossm_alt_board::{OssmAltBoard, Rs485};
use sossm::{CommandChannel, MechanicalConfig, MotionController, MotionLimits, Sossm};

use {esp_backtrace as _, esp_println as _};

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

const UPDATE_INTERVAL_SECS: f64 = 0.01;

type ConcreteMotor = M57AIMMotor<Rs485<Uart<'static, Blocking>, Output<'static>>, Delay>;

static COMMANDS: CommandChannel = CommandChannel::new();
static UPDATE_TIMER: Mutex<RefCell<Option<PeriodicTimer<'static, Blocking>>>> =
    Mutex::new(RefCell::new(None));
static MOTION: Mutex<RefCell<Option<MotionController<'static, ConcreteMotor>>>> =
    Mutex::new(RefCell::new(None));

#[handler(priority = Priority::Priority2)]
fn motion_update_interrupt() {
    critical_section::with(|cs| {
        UPDATE_TIMER
            .borrow_ref_mut(cs)
            .as_mut()
            .unwrap()
            .clear_interrupt();

        if let Some(controller) = MOTION.borrow_ref_mut(cs).as_mut() {
            let _ = controller.update();
        }
    });
}

#[esp_rtos::main]
async fn main(_spawner: Spawner) {
    esp_println::logger::init_logger_from_env();

    let p = esp_hal::init(esp_hal::Config::default());

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);

    // Start Embassy runtime (moved from board)
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

    let (sossm, mut controller) = Sossm::new(
        board.into_motor(),
        &config,
        MotionLimits::default(),
        UPDATE_INTERVAL_SECS,
        &COMMANDS,
    );

    // Blocking setup while controller is still a local — before the interrupt owns it
    controller.enable().expect("enable failed");
    controller.home().expect("homing failed");

    // Move controller into the static for interrupt access
    critical_section::with(|cs| {
        MOTION.borrow_ref_mut(cs).replace(controller);
    });

    // Set up the periodic timer on TIMG1 for the motion control interrupt
    let timg1 = TimerGroup::new(p.TIMG1);
    let mut update_timer = PeriodicTimer::new(timg1.timer0);

    update_timer.set_interrupt_handler(motion_update_interrupt);
    update_timer.listen();

    let interval_us = (sossm.update_interval_secs() * 1_000_000.0) as u64;

    update_timer
        .start(HalDuration::from_micros(interval_us))
        .expect("failed to start motion timer");

    critical_section::with(|cs| {
        UPDATE_TIMER.borrow_ref_mut(cs).replace(update_timer);
    });

    info!(
        "Motion interrupt started at {}ms interval",
        sossm.update_interval_secs() * 1000.0
    );

    // Send initial commands — no critical section needed, sossm is a local
    sossm.set_speed(150.0);
    sossm.move_to(100.0);

    // Main async loop — free for BLE, telemetry, patterns, etc.
    loop {
        Timer::after(Duration::from_secs(5)).await;
        info!("main loop heartbeat");
    }
}
