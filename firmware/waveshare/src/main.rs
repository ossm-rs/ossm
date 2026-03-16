#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use embassy_executor::Spawner;
use embassy_sync::blocking_mutex::raw::{CriticalSectionRawMutex, NoopRawMutex};
use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;
use embassy_time::Delay;
use embassy_time::{Duration, Ticker};
use esp_hal::{
    Blocking,
    gpio::{Level, Output, OutputConfig},
    interrupt::{Priority, software::SoftwareInterruptControl},
    system::Stack,
    timer::timg::TimerGroup,
    uart::{Config, Uart},
};
use esp_radio::esp_now::{EspNowManager, EspNowSender};
use esp_rtos::embassy::InterruptExecutor;
use log::info;
use m57aim_motor::{Modbus, Motor57AIM, Motor57AIMConfig};
use ossm::{MechanicalConfig, MotionController, MotionLimits, Ossm};

use rs485_board::{Rs485Board, Rs485, Rs485ModbusTransport};
use ossm_m5_remote::RemoteConfig;

use pattern_engine::{AnyPattern, PatternEngine};
use static_cell::StaticCell;

use {esp_backtrace as _, esp_println as _};

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

const UPDATE_INTERVAL_SECS: f64 = 0.01;
const MOTOR_BAUD_RATE: u32 = 115_200;
const DEVICE_ADDR: u8 = 0x01;

macro_rules! mk_static {
    ($t:ty, $val:expr) => {{
        static STATIC_CELL: StaticCell<$t> = StaticCell::new();
        STATIC_CELL.init($val)
    }};
}

type ConcreteTransport =
    Rs485ModbusTransport<Rs485<Uart<'static, Blocking>, Output<'static>>, Delay>;
type ConcreteMotor = Motor57AIM<Modbus<ConcreteTransport>, Delay>;
type ConcreteBoard = Rs485Board<ConcreteMotor>;

static OSSM: Ossm = Ossm::new();
static PATTERNS: PatternEngine = PatternEngine::new(&OSSM);

static EXECUTOR_CORE_1: StaticCell<InterruptExecutor<2>> = StaticCell::new();
static APP_CORE_STACK: StaticCell<Stack<16384>> = StaticCell::new();
static MOTION_READY: Signal<CriticalSectionRawMutex, bool> = Signal::new();

#[embassy_executor::task]
async fn motion_task(mut controller: MotionController<'static, ConcreteBoard>) {
    let interval_us = (UPDATE_INTERVAL_SECS * 1_000_000.0) as u64;
    let mut ticker = Ticker::every(Duration::from_micros(interval_us));

    loop {
        controller.update().await;
        ticker.next().await;
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    esp_println::logger::init_logger_from_env();

    let p = esp_hal::init(esp_hal::Config::default());

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);

    let timg0 = TimerGroup::new(p.TIMG0);
    esp_rtos::start(timg0.timer0);

    let uart_config = Config::default().with_baudrate(MOTOR_BAUD_RATE);
    let uart = Uart::new(p.UART1, uart_config)
        .expect("Failed to initialize UART")
        .with_tx(p.GPIO17)
        .with_rx(p.GPIO18);

    let de = Output::new(p.GPIO21, Level::Low, OutputConfig::default());
    let rs485 = Rs485::new(uart, de);

    let transport = Rs485ModbusTransport::new(rs485, Delay);
    let motor = Motor57AIM::new(
        Modbus::new(transport, DEVICE_ADDR),
        Motor57AIMConfig::default(),
        Delay,
    );

    static MECHANICAL: MechanicalConfig = MechanicalConfig {
        pulley_teeth: 20,
        belt_pitch_mm: 2.0,
    };
    let limits = MotionLimits::default();

    let board = Rs485Board::new(motor, &MECHANICAL);
    let controller = OSSM.controller(board, limits.clone(), UPDATE_INTERVAL_SECS);

    let sw_int = SoftwareInterruptControl::new(p.SW_INTERRUPT);
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
        p.CPU_CTRL,
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

    let radio = &*mk_static!(
        esp_radio::Controller<'static>,
        esp_radio::init().expect("Failed to initialize radio controller")
    );

    let (mut wifi_controller, interfaces) =
        esp_radio::wifi::new(radio, p.WIFI, Default::default()).unwrap();
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

    ossm_m5_remote::start(&spawner, manager, sender, receiver, &PATTERNS, remote_config);

    let mut pattern_runner = PATTERNS.runner(AnyPattern::all_builtin());
    pattern_runner.run(Delay).await;
}
