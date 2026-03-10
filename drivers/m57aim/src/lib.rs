#![no_std]

mod rs485;
mod stepdir;

pub use ossm::{Modbus, ModbusTransport};

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u16)]
pub enum RwRegister {
    ModbusEnable = 0x00,
    DriverOutputEnable = 0x01,
    MotorTargetSpeed = 0x02,
    MotorAcceleration = 0x03,
    WeakMagneticAngle = 0x04,
    SpeedRingProportionalCoefficient = 0x05,
    SpeedLoopIntegrationTime = 0x06,
    PositionRingProportionalCoefficient = 0x07,
    SpeedFeedForward = 0x08,
    DirPolarity = 0x09,
    ElectronicGearNumerator = 0x0A,
    ElectronicGearDenominator = 0x0B,
    ParameterSaveFlag = 0x14,
    AbsolutePositionLowU16 = 0x16,
    AbsolutePositionHighU16 = 0x17,
    StandstillMaxOutput = 0x18,
    SpecificFunction = 0x19,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u16)]
pub enum RoRegister {
    TargetPositionLowU16 = 0x0C,
    TargetPositionHighU16 = 0x0D,
    AlarmCode = 0x0E,
    SystemCurrent = 0x0F,
    MotorCurrentSpeed = 0x10,
    SystemVoltage = 0x11,
    SystemTemperature = 0x12,
    SystemOutputPwm = 0x13,
    DeviceAddress = 0x15,
}

pub struct Motor57AIMConfig {
    pub steps_per_rev: u32,
    pub max_output: u16,
}

impl Default for Motor57AIMConfig {
    fn default() -> Self {
        Self {
            steps_per_rev: 32_768,
            max_output: 600,
        }
    }
}

/// 57AIM BLDC servo motor, generic over communication interface and delay.
pub struct Motor57AIM<I, D> {
    pub(crate) interface: I,
    pub(crate) config: Motor57AIMConfig,
    pub(crate) delay: D,
}

impl<I, D> Motor57AIM<I, D> {
    pub fn new(interface: I, config: Motor57AIMConfig, delay: D) -> Self {
        Self {
            interface,
            config,
            delay,
        }
    }
}
