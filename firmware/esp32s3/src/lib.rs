#![no_std]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

#[cfg(not(feature = "motor-rs485"))]
compile_error!("This crate currently requires the motor-rs485 feature. Add --features motor-sim to overlay a simulated motor for bench testing.");

mod board;
mod motor;
mod owner_auth;
mod owner_control;
mod owner_settings;
mod owner_web;
mod radio;
mod wifi_settings;

pub use motor::Config as MotorConfig;

extern crate alloc;

use control_interface::{ControlSender, policy};
use embassy_executor::Spawner;
use embassy_time::{Delay, Duration, Ticker};
use esp_hal::{
    peripherals::{BT, CPU_CTRL, FLASH, SW_INTERRUPT, TIMG0, USB_DEVICE, WIFI},
    timer::timg::TimerGroup,
    usb_serial_jtag::UsbSerialJtag,
};
use log::info;
use ossm::{MechanicalConfig, MotionController, MotionLimits, MotionSender, Ossm};
use pattern_engine::{AnyPattern, PatternEngine, PatternSender};
use static_cell::StaticCell;

#[macro_export]
macro_rules! mk_static {
    ($t:ty, $val:expr) => {{
        static STATIC_CELL: ::static_cell::StaticCell<$t> = ::static_cell::StaticCell::new();
        STATIC_CELL.init($val)
    }};
}

const UPDATE_INTERVAL_SECS: f64 = 0.01;
static OSSM_CELL: StaticCell<Ossm> = StaticCell::new();
static PATTERNS_CELL: StaticCell<PatternEngine> = StaticCell::new();

pub struct Config {
    pub motor: motor::Config,
    pub wifi: WIFI<'static>,
    pub bt: BT<'static>,
    pub flash: FLASH<'static>,
    pub timg0: TIMG0<'static>,
    pub sw_int: SW_INTERRUPT<'static>,
    pub cpu_ctrl: CPU_CTRL<'static>,
    pub usb_device: USB_DEVICE<'static>,
}

#[embassy_executor::task]
async fn motion_task(mut controller: MotionController<'static, board::Board>) {
    let interval_us = (UPDATE_INTERVAL_SECS * 1_000_000.0) as u64;
    let mut ticker = Ticker::every(Duration::from_micros(interval_us));
    loop {
        if let Err(e) = controller.update().await {
            log::error!("Motion controller fault: {:?}", e);
        }
        ticker.next().await;
    }
}

pub async fn run(spawner: Spawner, config: Config) {
    ossm::logging::init(log::LevelFilter::Info, |line| esp_println::println!("{}", line));
    ossm::build_info!();

    // Known-good ESP32-S3 radio/motion baseline from the prior fork.
    esp_alloc::heap_allocator!(size: 168 * 1024);

    let timg0 = TimerGroup::new(config.timg0);
    esp_rtos::start(timg0.timer0);
    let motor = motor::build(config.motor).await;

    static MECHANICAL: MechanicalConfig = MechanicalConfig {
        pulley_teeth: 20,
        belt_pitch_mm: 2.0,
        reverse_direction: false,
    };

    let mut limits = MotionLimits::default();
    policy::configure_machine(
        limits.max_velocity_mm_s,
        limits.max_position_mm - limits.min_position_mm,
    );

    let settings_flash: &'static wifi_settings::WifiFlash = mk_static!(
        wifi_settings::WifiFlash,
        embassy_sync::mutex::Mutex::new(esp_storage::FlashStorage::new(config.flash))
    );
    owner_settings::load_and_apply(settings_flash).await;

    // Persisted machine settings define the boot-time physical controller envelope.
    limits.max_velocity_mm_s = policy::machine_max_speed_mm_s();
    limits.max_position_mm = limits.min_position_mm + policy::machine_travel_mm();
    owner_auth::load(settings_flash).await;

    let (receiver, _motion_observer, motion_sender) = OSSM_CELL.init(Ossm::new()).split();
    let motion: &'static MotionSender = mk_static!(MotionSender, motion_sender);

    let board = board::build(motor, &MECHANICAL);
    let controller = receiver.into_controller(board, limits.clone(), UPDATE_INTERVAL_SECS);

    // Keep motion on the main Embassy executor. This was the stable Wi-Fi+BLE baseline.
    spawner.must_spawn(motion_task(controller));
    let _ = (config.sw_int, config.cpu_ctrl);
    info!("Motion task started on ProCpu/main executor at 10ms interval");

    let (runner, _pattern_observer, pattern_sender) = PATTERNS_CELL.init(PatternEngine::new()).split();
    let patterns: &'static PatternSender = mk_static!(PatternSender, pattern_sender);
    let control: &'static ControlSender = mk_static!(ControlSender, ControlSender::new(patterns, motion));

    // Owner control and every external protocol receive the generic facade,
    // never pattern-engine internals as their command surface.
    let owner_usb = UsbSerialJtag::new(config.usb_device);
    spawner.must_spawn(owner_control::owner_usb_task(owner_usb, control));
    spawner.must_spawn(owner_control::owner_supervisor_task(control));
    spawner.must_spawn(owner_web::bpm_control_task(control));

    radio::start(&spawner, config.wifi, config.bt, control, &limits, settings_flash).await;

    // Pattern engine remains untouched upstream code and owns only pattern execution.
    runner.run(motion, AnyPattern::all_builtin(), Delay).await
}
