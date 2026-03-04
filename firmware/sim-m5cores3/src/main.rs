#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use core::cell::Cell;
use core::sync::atomic::{AtomicI32, Ordering};

use embassy_executor::Spawner;
use embassy_futures::yield_now;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_time::{Delay, Duration, Instant, Ticker};
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::i2c::master::{Config as I2cConfig, I2c};
use esp_hal::interrupt::Priority;
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::spi::Mode as SpiMode;
use esp_hal::spi::master::{Config as SpiConfig, Spi};
use esp_hal::system::Stack;
use esp_hal::time::Rate;
use esp_hal::timer::timg::TimerGroup;
use log::info;
use ossm::{
    CommandChannel, HomingSignal, MechanicalConfig, MotionController, MotionLimits, Motor,
    MoveCompleteSignal, Ossm,
};
use pattern_engine::{Pattern, PatternCtx, PatternInput, SharedPatternInput, patterns::Deeper};
use sim_m5cores3_board::{Display, FrameState, create_terminal, render_ui};
use sim_motor::SimMotor;
use static_cell::StaticCell;

use esp_rtos::embassy::InterruptExecutor;

use {esp_backtrace as _, esp_println as _};

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

const UPDATE_INTERVAL_SECS: f64 = 1.0 / 30.0;

static COMMANDS: CommandChannel = CommandChannel::new();
static HOMING_DONE: HomingSignal = HomingSignal::new();
static MOVE_COMPLETE: MoveCompleteSignal = MoveCompleteSignal::new();
static PATTERN_INPUT: SharedPatternInput = SharedPatternInput::new(Cell::new(PatternInput {
    depth: 0.7,
    stroke: 0.5,
    velocity: 0.50,
    sensation: -0.20,
}));
static MOTOR_POSITION: AtomicI32 = AtomicI32::new(0);

static EXECUTOR_CORE_1: StaticCell<InterruptExecutor<2>> = StaticCell::new();
static APP_CORE_STACK: StaticCell<Stack<16384>> = StaticCell::new();
static MOTION_READY: Signal<CriticalSectionRawMutex, bool> = Signal::new();

#[embassy_executor::task]
async fn motion_task(mut controller: MotionController<'static, SimMotor>) {
    let interval_us = (UPDATE_INTERVAL_SECS * 1_000_000.0) as u64;
    let mut ticker = Ticker::every(Duration::from_micros(interval_us));

    loop {
        controller.update().await;
        ticker.next().await;
    }
}

#[embassy_executor::task]
async fn display_task(mut display: Display, steps_per_mm: f64, min_mm: f64, max_mm: f64) {
    let mut terminal = create_terminal(&mut display);
    let range = max_mm - min_mm;
    let mut last_frame = Instant::now();
    let mut fps: u32 = 0;

    loop {
        let steps = MOTOR_POSITION.load(Ordering::Relaxed);
        let mm = steps as f64 / steps_per_mm;
        let position = if range > 0.0 {
            let frac = (mm - min_mm) / range;
            if frac < 0.0 {
                0.0
            } else if frac > 1.0 {
                1.0
            } else {
                frac
            }
        } else {
            0.0
        };

        let input = PATTERN_INPUT.lock(|cell| cell.get());

        let state = FrameState {
            position,
            depth: input.depth,
            stroke: input.stroke,
            velocity: input.velocity,
            sensation: input.sensation,
            fps,
            state: "Running",
        };

        let _ = terminal.draw(|frame| {
            render_ui(frame, &state);
        });

        // Yield to let the pattern engine run on Core 0
        yield_now().await;

        let frame_time = last_frame.elapsed();
        fps = (1_000_000 / frame_time.as_micros().max(1)) as u32;
        last_frame = Instant::now();
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    esp_println::logger::init_logger_from_env();

    let config = esp_hal::Config::default().with_cpu_clock(esp_hal::clock::CpuClock::max());
    let p = esp_hal::init(config);

    esp_alloc::heap_allocator!(size: 200_000);

    let timg0 = TimerGroup::new(p.TIMG0);
    esp_rtos::start(timg0.timer0);

    info!("Initializing M5Stack CoreS3 board...");

    let i2c_config = I2cConfig::default().with_frequency(Rate::from_khz(400));
    let mut i2c = I2c::new(p.I2C0, i2c_config)
        .expect("Failed to initialize I2C")
        .with_sda(p.GPIO12)
        .with_scl(p.GPIO11);

    // Enable display backlight
    sim_m5cores3_board::pmu::init(&mut i2c);

    let mut delay = esp_hal::delay::Delay::new();
    sim_m5cores3_board::io_expander::init(&mut i2c, &mut delay);

    let spi_config = SpiConfig::default()
        .with_frequency(Rate::from_mhz(40))
        .with_mode(SpiMode::_0);
    let spi = Spi::new(p.SPI2, spi_config)
        .expect("Failed to initialize SPI")
        .with_mosi(p.GPIO37)
        .with_sck(p.GPIO36);
    let cs = Output::new(p.GPIO3, Level::High, OutputConfig::default());
    let dc = Output::new(p.GPIO35, Level::Low, OutputConfig::default());

    let display = sim_m5cores3_board::display::init(spi, cs, dc);

    info!("Board initialization complete");

    let motor = SimMotor::new(&MOTOR_POSITION);
    let config = MechanicalConfig {
        max_position_mm: 250.0,
        ..MechanicalConfig::default()
    };

    let steps_per_mm = config.steps_per_mm(SimMotor::STEPS_PER_REV) as f64;

    let (ossm, controller) = Ossm::new(
        motor,
        &config,
        MotionLimits::default(),
        UPDATE_INTERVAL_SECS,
        &COMMANDS,
        &HOMING_DONE,
        &MOVE_COMPLETE,
    );

    let sw_int = SoftwareInterruptControl::new(p.SW_INTERRUPT);
    let app_core_stack = APP_CORE_STACK.init(Stack::new());

    let second_core = move || {
        let executor = InterruptExecutor::new(sw_int.software_interrupt2);
        let executor = EXECUTOR_CORE_1.init(executor);
        let spawner = executor.start(Priority::Priority1);

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

    ossm.enable();
    ossm.home().await;

    spawner
        .spawn(display_task(
            display,
            steps_per_mm,
            config.min_position_mm,
            config.max_position_mm,
        ))
        .unwrap();

    let mut ctx = PatternCtx::new(&COMMANDS, &MOVE_COMPLETE, &PATTERN_INPUT, Delay);
    let mut pattern = Deeper;
    pattern.run(&mut ctx).await;
}
