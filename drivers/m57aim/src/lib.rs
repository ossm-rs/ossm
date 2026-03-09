#![no_std]

mod rs485;
mod stepdir;

/// Async Modbus wire protocol. The only trait in the motor driver layer.
///
/// Implementations handle framing, CRC, and physical transport (RS485, TCP, etc.).
/// The motor struct uses this to read/write registers without knowing the wire details.
#[allow(async_fn_in_trait)]
pub trait ModbusTransport {
    type Error: core::fmt::Debug;

    /// Write a single holding register (function code 0x06).
    async fn write_holding(
        &mut self,
        device_addr: u8,
        register: u16,
        value: u16,
    ) -> Result<(), Self::Error>;

    /// Read one or more holding registers (function code 0x03).
    async fn read_holding(
        &mut self,
        device_addr: u8,
        register: u16,
        count: u16,
    ) -> Result<heapless::Vec<u16, 8>, Self::Error>;

    /// Send a raw frame and read the response.
    ///
    /// Used for vendor-specific function codes (e.g. the 57AIM's 0x7B
    /// absolute position command) that don't fit standard Modbus functions.
    async fn raw_transaction(
        &mut self,
        request: &[u8],
        response: &mut [u8],
    ) -> Result<usize, Self::Error>;
}

/// Modbus interface: wraps a transport + device address.
pub struct Modbus<T: ModbusTransport> {
    pub transport: T,
    pub device_addr: u8,
}

impl<T: ModbusTransport> Modbus<T> {
    pub fn new(transport: T, device_addr: u8) -> Self {
        Self {
            transport,
            device_addr,
        }
    }
}

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
    pub interface: I,
    pub config: Motor57AIMConfig,
    pub delay: D,
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

mod modbus_constants {
    pub const SET_ABSOLUTE_POSITION_FUNC: u8 = 0x7B;
    pub const HOME_SPEED_RPM: u16 = 80;
    pub const HOME_MAX_OUTPUT: u16 = 89;
    pub const HOME_STEP_THRESHOLD: i32 = 15;
    pub const HOME_POLL_INTERVAL_MS: u32 = 50;
    pub const POST_HOME_SETTLE_MS: u32 = 20;
    pub const POST_ENABLE_SETTLE_MS: u32 = 800;
    pub const OPERATING_SPEED_RPM: u16 = 3000;
    pub const OPERATING_ACCELERATION: u16 = 50000;
    pub const OPERATING_MAX_OUTPUT: u16 = 600;
}
