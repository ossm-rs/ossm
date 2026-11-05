#![no_std]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

#[cfg(not(feature = "motor-rs485"))]
compile_error!(
    "This crate currently requires the motor-rs485 feature. Add --features motor-sim to \
    overlay a simulated motor for bench testing."
);

mod board;
mod indicator;
mod motor;
mod radio;

pub use indicator::Config as IndicatorConfig;
pub use motor::Config as MotorConfig;

use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Duration, Ticker};
use esp_hal::{
    interrupt::{Priority, software::SoftwareInterruptControl},
    peripherals::{BT, CPU_CTRL, SW_INTERRUPT, TIMG0, WIFI},
    system::Stack,
    timer::timg::TimerGroup,
};
use esp_rtos::embassy::InterruptExecutor;
use log::info;
use ossm::motion::{MechanicalConfig, MotionController, MotionLimits, MotionObserver};
use ossm::platform::{Fault, Hardware, Platform};
use pattern_engine::{PatternObserver, PatternSender};
use static_cell::StaticCell;

extern crate alloc;

#[macro_export]
macro_rules! mk_static {
    ($t:ty, $val:expr) => {{
        static STATIC_CELL: ::static_cell::StaticCell<$t> = ::static_cell::StaticCell::new();
        STATIC_CELL.init($val)
    }};
}

const UPDATE_INTERVAL_SECS: f64 = 0.01;

static EXECUTOR_CORE_1: StaticCell<InterruptExecutor<2>> = StaticCell::new();
static APP_CORE_STACK: StaticCell<Stack<32768>> = StaticCell::new();
static MOTION_READY: Signal<CriticalSectionRawMutex, bool> = Signal::new();

pub struct Config {
    pub motor: MotorConfig,
    pub indicator: Option<IndicatorConfig>,
    pub wifi: WIFI<'static>,
    pub bt: BT<'static>,
    pub timg0: TIMG0<'static>,
    pub sw_int: SW_INTERRUPT<'static>,
    pub cpu_ctrl: CPU_CTRL<'static>,
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

/// OSSM on an ESP32-S3: motion on core 1, the indicator and radios on
/// core 0.
pub struct Esp32s3 {
    spawner: Spawner,
    config: Option<Config>,
    indicator: Option<indicator::Output>,
    core_1: Option<(SW_INTERRUPT<'static>, CPU_CTRL<'static>)>,
    radio: Option<(WIFI<'static>, BT<'static>)>,
}

impl Esp32s3 {
    pub fn new(spawner: Spawner, config: Config) -> Self {
        Self {
            spawner,
            config: Some(config),
            indicator: None,
            core_1: None,
            radio: None,
        }
    }
}

impl Platform for Esp32s3 {
    type Board = board::Board;

    async fn init(&mut self) -> Result<Hardware<board::Board>, Fault> {
        let config = self.config.take().expect("init runs once");

        ossm::platform::logging::init(log::LevelFilter::Info, |line| {
            esp_println::println!("{}", line);
        });

        ossm::platform::build_info!();

        esp_alloc::heap_allocator!(size: 128 * 1024);

        let timg0 = TimerGroup::new(config.timg0);
        esp_rtos::start(timg0.timer0);

        self.indicator = indicator::build(config.indicator);
        self.core_1 = Some((config.sw_int, config.cpu_ctrl));
        self.radio = Some((config.wifi, config.bt));

        let motor = motor::build(config.motor).await?;

        static MECHANICAL: MechanicalConfig = MechanicalConfig {
            pulley_teeth: 20,
            belt_pitch_mm: 2.0,
            reverse_direction: false,
        };

        Ok(Hardware {
            board: board::build(motor, &MECHANICAL),
            limits: MotionLimits::default(),
            update_interval_secs: UPDATE_INTERVAL_SECS,
        })
    }

    async fn start_motion(&mut self, controller: MotionController<'static, board::Board>) {
        let (sw_int, cpu_ctrl) = self.core_1.take().expect("start_motion runs once");
        let sw_int = SoftwareInterruptControl::new(sw_int);
        let app_core_stack = APP_CORE_STACK.init(Stack::new());

        let second_core = move || {
            let executor = InterruptExecutor::new(sw_int.software_interrupt2);
            let executor = EXECUTOR_CORE_1.init(executor);
            let spawner = executor.start(Priority::Priority2);

            spawner.spawn(motion_task(controller)).unwrap();

            MOTION_READY.signal(true);

            loop {}
        };

        esp_rtos::start_second_core(
            cpu_ctrl,
            sw_int.software_interrupt0,
            sw_int.software_interrupt1,
            app_core_stack,
            second_core,
        );

        MOTION_READY.wait().await;

        info!(
            "Motion task started on core 1 at {}ms interval",
            UPDATE_INTERVAL_SECS * 1000.0
        );
    }

    fn start_indicator(&mut self, motion: MotionObserver, patterns: PatternObserver) {
        indicator::start(&self.spawner, self.indicator.take(), motion, patterns);
    }

    fn start_remotes(&mut self, patterns: &'static PatternSender, limits: &MotionLimits) {
        let (wifi, bt) = self.radio.take().expect("start_remotes runs once");
        radio::start(&self.spawner, wifi, bt, patterns, limits);
    }
}
