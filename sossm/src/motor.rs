use core::fmt::Debug;

#[allow(async_fn_in_trait)]
pub trait Motor {
    type Error: Debug;

    const STEPS_PER_REV: u32;

    async fn enable(&mut self) -> Result<(), Self::Error>;
    async fn disable(&mut self) -> Result<(), Self::Error>;

    /// Run the full homing sequence: trigger, poll until complete, settle,
    /// and restore operating parameters. Returns when the motor is ready.
    async fn home(&mut self) -> Result<(), Self::Error>;

    async fn set_absolute_position(&mut self, steps: i32) -> Result<(), Self::Error>;

    async fn set_speed(&mut self, rpm: u16) -> Result<(), Self::Error>;
    async fn set_acceleration(&mut self, value: u16) -> Result<(), Self::Error>;
    async fn set_max_output(&mut self, output: u16) -> Result<(), Self::Error>;
}

#[allow(async_fn_in_trait)]
pub trait MotorTelemetry {
    type Error: Debug;

    async fn get_absolute_position(&mut self) -> Result<i32, Self::Error>;
    async fn get_remaining_steps(&mut self) -> Result<i32, Self::Error>;

    async fn get_speed(&mut self) -> Result<u16, Self::Error>;
    async fn get_acceleration(&mut self) -> Result<u16, Self::Error>;
    async fn get_max_output(&mut self) -> Result<u16, Self::Error>;

    async fn get_current_amps(&mut self) -> Result<f32, Self::Error>;
    async fn get_voltage_volts(&mut self) -> Result<f32, Self::Error>;
}
