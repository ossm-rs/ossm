#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]
#![deny(clippy::large_stack_frames)]

use core::convert::Infallible;

use embassy_executor::Spawner;
use embassy_time::Delay;
use embassy_time::{Duration, Ticker};
use embedded_hal::pwm::SetDutyCycle;
use esp_hal::Blocking;
use esp_hal::analog::adc::{Adc, AdcConfig, AdcPin, Attenuation};
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::interrupt::Priority;
use esp_hal::interrupt::software::SoftwareInterruptControl;
use esp_hal::mcpwm::operator::PwmPinConfig;
use esp_hal::mcpwm::timer::PwmWorkingMode;
use esp_hal::mcpwm::{McPwm, PeripheralClockConfig};
use esp_hal::peripherals::{ADC1, MCPWM0};
use esp_hal::timer::timg::TimerGroup;
use esp_rtos::embassy::InterruptExecutor;
use log::info;
use ossm::stepdir::{StepDirConfig, StepDirMotor, StepOutput};
use ossm::{MechanicalConfig, MotionController, MotionLimits, Ossm};
use ossm_stock_board::{CurrentSensor, HomingConfig, OssmStock};
use pattern_engine::{AnyPattern, PatternEngine};
use static_cell::StaticCell;

use {esp_backtrace as _, esp_println as _};

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

const UPDATE_INTERVAL_SECS: f64 = 0.01;
/// MCPWM step pulse frequency in Hz.
const STEP_FREQ_HZ: u32 = 50_000;

struct McpwmStepOutput<'d> {
    pin: esp_hal::mcpwm::operator::PwmPin<'d, MCPWM0<'d>, 0, true>,
    step_period_us: u64,
}

impl StepOutput for McpwmStepOutput<'_> {
    type Error = Infallible;

    async fn step(&mut self, count: u32) -> Result<(), Self::Error> {
        let _ = self.pin.set_duty_cycle_percent(STEP_DUTY_PERCENT);
        let total_us = count as u64 * self.step_period_us;
        embassy_time::Timer::after_micros(total_us).await;
        self.pin.set_duty_cycle_fully_off().unwrap();
        Ok(())
    }
}

struct EspCurrentSensor<'d, P> {
    adc: Adc<'d, ADC1<'d>, Blocking>,
    pin: AdcPin<P, ADC1<'d>>,
}

/// 12-bit ADC maximum value.
const ADC_MAX: f32 = 4095.0;
/// Step pulse duty cycle percentage.
const STEP_DUTY_PERCENT: u8 = 50;

impl<P: esp_hal::analog::adc::AdcChannel> CurrentSensor for EspCurrentSensor<'_, P> {
    type Error = Infallible;

    fn read_fraction(&mut self, samples: u32) -> Result<f32, Self::Error> {
        let mut sum: u32 = 0;
        for _ in 0..samples {
            let raw: u16 = nb::block!(self.adc.read_oneshot(&mut self.pin)).unwrap();
            sum += raw as u32;
        }
        let average = sum as f32 / samples as f32;
        Ok(average / ADC_MAX)
    }
}

type ConcreteStepOutput = McpwmStepOutput<'static>;
type ConcreteMotor = StepDirMotor<ConcreteStepOutput, Output<'static>, Output<'static>>;
type ConcreteBoard = OssmStock<
    ConcreteMotor,
    EspCurrentSensor<'static, esp_hal::peripherals::GPIO36<'static>>,
    Delay,
>;

static OSSM: Ossm = Ossm::new();
static PATTERNS: PatternEngine = PatternEngine::new(&OSSM);
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
async fn main(_spawner: Spawner) {
    esp_println::logger::init_logger_from_env();

    let p = esp_hal::init(esp_hal::Config::default());

    esp_alloc::heap_allocator!(#[esp_hal::ram(reclaimed)] size: 73744);

    let timg0 = TimerGroup::new(p.TIMG0);
    esp_rtos::start(timg0.timer0);

    let peripheral_clock = PeripheralClockConfig::with_frequency(esp_hal::time::Rate::from_mhz(10))
        .expect("Failed to configure MCPWM peripheral clock");

    let mut mcpwm = McPwm::new(p.MCPWM0, peripheral_clock);

    let timer_period: u16 = (peripheral_clock.frequency().as_hz() / STEP_FREQ_HZ) as u16 - 1;
    let timer_config = peripheral_clock
        .timer_clock_with_frequency(
            timer_period,
            PwmWorkingMode::Increase,
            esp_hal::time::Rate::from_hz(STEP_FREQ_HZ),
        )
        .expect("Failed to configure MCPWM timer");

    mcpwm.operator0.set_timer(&mcpwm.timer0);
    let mut step_pin = mcpwm
        .operator0
        .with_pin_a(p.GPIO14, PwmPinConfig::UP_ACTIVE_HIGH);

    mcpwm.timer0.start(timer_config);
    step_pin.set_duty_cycle_fully_off().unwrap();

    let step_period_us = 1_000_000u64 / STEP_FREQ_HZ as u64;

    let step_output = McpwmStepOutput {
        pin: step_pin,
        step_period_us,
    };

    let dir_pin = Output::new(p.GPIO27, Level::Low, OutputConfig::default());
    let ena_pin = Output::new(p.GPIO26, Level::High, OutputConfig::default());

    let mut adc_config = AdcConfig::new();
    let current_pin = adc_config.enable_pin(p.GPIO36, Attenuation::_11dB);
    let adc = Adc::new(p.ADC1, adc_config);

    let current_sensor = EspCurrentSensor {
        adc,
        pin: current_pin,
    };

    let motor_config = StepDirConfig {
        steps_per_rev: 800,
        max_output: 1000,
        reverse_direction: false,
    };
    let motor = StepDirMotor::new(step_output, dir_pin, ena_pin, motor_config);

    static MECHANICAL: MechanicalConfig = MechanicalConfig {
        pulley_teeth: 20,
        belt_pitch_mm: 2.0,
    };

    let homing_config = HomingConfig::default();
    let limits = MotionLimits::default();
    let board = OssmStock::new(motor, current_sensor, Delay, &MECHANICAL, homing_config);
    let controller = OSSM.controller(board, limits, UPDATE_INTERVAL_SECS);

    let sw_ints = SoftwareInterruptControl::new(p.SW_INTERRUPT);
    let executor = EXECUTOR_HIGH.init(InterruptExecutor::new(sw_ints.software_interrupt1));
    let high_spawner = executor.start(Priority::Priority2);
    high_spawner.spawn(motion_task(controller)).unwrap();

    info!(
        "Motion task started at {}ms interval",
        UPDATE_INTERVAL_SECS * 1000.0
    );

    // --- Pattern engine ---

    let mut pattern_runner = PATTERNS.runner(AnyPattern::all_builtin());

    info!("Homing...");
    PATTERNS.home();

    pattern_runner.run(Delay).await;
}
