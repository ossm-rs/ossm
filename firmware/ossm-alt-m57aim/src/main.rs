#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_time::Delay;
use embassy_time::{Duration, Ticker};
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::{Blocking, gpio::{Level, Output, OutputConfig}, interrupt::Priority, uart::{Config, Uart}};
use esp_radio::esp_now::{EspNowManager, EspNowSender};
use esp_rtos::embassy::InterruptExecutor;
use log::info;
use m57aim_motor::{Modbus, Motor57AIM, Motor57AIMConfig};
use ossm::{MechanicalConfig, MotionController, MotionLimits, Ossm};
use ossm_alt_board::{OssmAlt, Rs485, Rs485ModbusTransport};
use ossm_m5_remote::{RemoteConfig, RemoteEvent, RemoteEventChannel};
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

type ConcreteTransport = Rs485ModbusTransport<Rs485<Uart<'static, Blocking>, Output<'static>>, Delay>;
type ConcreteMotor = Motor57AIM<Modbus<ConcreteTransport>, Delay>;
type ConcreteBoard = OssmAlt<ConcreteMotor>;

static OSSM: Ossm = Ossm::new();
static PATTERNS: PatternEngine = PatternEngine::new(&OSSM);

static REMOTE_EVENTS: RemoteEventChannel = RemoteEventChannel::new();
static EXECUTOR_HIGH: StaticCell<InterruptExecutor<1>> = StaticCell::new();

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
        .with_tx(p.GPIO10)
        .with_rx(p.GPIO12);

    // Manual DE control — hardware RS485 mode has inverted RTS polarity
    // on the OSSM Alt board, so we toggle a GPIO directly instead.
    let de = Output::new(p.GPIO11, Level::Low, OutputConfig::default());
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

    let board = OssmAlt::new(motor, &MECHANICAL);
    let controller = OSSM.controller(board, limits.clone(), UPDATE_INTERVAL_SECS);

    let sw_ints = SoftwareInterruptControl::new(p.SW_INTERRUPT);
    let executor = EXECUTOR_HIGH.init(InterruptExecutor::new(sw_ints.software_interrupt1));
    let high_spawner = executor.start(Priority::Priority2);
    high_spawner.spawn(motion_task(controller)).unwrap();

    info!(
        "Motion task started at {}ms interval",
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

    spawner
        .spawn(ossm_m5_remote::receiver_task(
            manager,
            sender,
            receiver,
            &PATTERNS.input(),
            &REMOTE_EVENTS,
            remote_config,
        ))
        .unwrap();
    spawner
        .spawn(ossm_m5_remote::heartbeat_send_task(
            manager,
            sender,
            remote_config,
        ))
        .unwrap();
    spawner
        .spawn(ossm_m5_remote::heartbeat_check_task(&REMOTE_EVENTS))
        .unwrap();

    info!("ESP-NOW remote tasks started, waiting for connection...");

    let mut pattern_runner = PATTERNS.runner(AnyPattern::all_builtin());

    join(pattern_runner.run(Delay), async {
        let mut current_pattern: usize = 0;

        loop {
            while !matches!(REMOTE_EVENTS.receive().await, RemoteEvent::Connected) {}

            info!("Remote connected, homing...");
            PATTERNS.home();

            loop {
                match REMOTE_EVENTS.receive().await {
                    RemoteEvent::Disconnected => {
                        PATTERNS.stop();
                        info!("Remote disconnected");
                        break;
                    }
                    RemoteEvent::Play => {
                        info!("Playing pattern {}", current_pattern);
                        PATTERNS.play(current_pattern);
                    }
                    RemoteEvent::Pause => {
                        PATTERNS.pause();
                        info!("Paused");
                    }
                    RemoteEvent::SwitchPattern(idx) => {
                        current_pattern = idx as usize;
                        info!("Switching to pattern {}", current_pattern);
                        PATTERNS.play(current_pattern);
                    }
                    RemoteEvent::Connected => {}
                }
            }
        }
    })
    .await;
}
