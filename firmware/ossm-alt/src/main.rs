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
use embassy_sync::blocking_mutex::raw::{CriticalSectionRawMutex, NoopRawMutex};
use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;
use embassy_time::Delay;
use embassy_time::{Duration, Ticker};
use esp_hal::interrupt::Priority;
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::system::Stack;
use esp_hal::timer::timg::TimerGroup;
use esp_hal::{Blocking, gpio::Output, uart::Uart};
use esp_radio::esp_now::{EspNowManager, EspNowSender};
use esp_rtos::embassy::InterruptExecutor;
use log::info;
use m57aim_motor::M57AIMMotor;
use ossm::{MechanicalConfig, MotionController, MotionLimits, Ossm};
use ossm_alt_board::{OssmAltBoard, Rs485};
use ossm_m5_remote::{RemoteConfig, RemoteEvent, RemoteEventChannel};
use pattern_engine::{AnyPattern, PatternEngine};
use static_cell::StaticCell;

use {esp_backtrace as _, esp_println as _};

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

const UPDATE_INTERVAL_SECS: f64 = 0.01;

macro_rules! mk_static {
    ($t:ty, $val:expr) => {{
        static STATIC_CELL: StaticCell<$t> = StaticCell::new();
        STATIC_CELL.init($val)
    }};
}

type ConcreteMotor = M57AIMMotor<Rs485<Uart<'static, Blocking>, Output<'static>>, Delay>;
type ConcreteBoard = OssmAltBoard<ConcreteMotor>;

static OSSM: Ossm = Ossm::new();
static PATTERNS: PatternEngine = PatternEngine::new(&OSSM);

static REMOTE_EVENTS: RemoteEventChannel = RemoteEventChannel::new();

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

    let board = OssmAltBoard::<ConcreteMotor>::new(
        p.UART1,
        p.GPIO10,
        p.GPIO12,
        p.GPIO11,
        MechanicalConfig::default(),
    );
    let mech_config = board.mechanical_config().clone();
    let limits = MotionLimits::default();

    let controller = OSSM.controller(board, &mech_config, limits.clone(), UPDATE_INTERVAL_SECS);

    let sw_int = SoftwareInterruptControl::new(p.SW_INTERRUPT);
    let app_core_stack = APP_CORE_STACK.init(Stack::new());

    /// Run the motion controller interrupt on it's own core at high priority
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
        max_travel_mm: mech_config.max_position_mm - mech_config.min_position_mm,
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
