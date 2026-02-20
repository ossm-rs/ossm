use core::fmt::Debug;
use embassy_time::{Duration, Timer};

// Embassy tasks have static ownership of peripherals; motors are owned by a single task and
// their futures never cross core boundaries, so Send bounds on async trait methods are unnecessary.
#[allow(async_fn_in_trait)]
pub trait Motor: From<Self::Transport> {
    type Error: Debug;
    type Transport;

    const STEPS_PER_REV: u32;

    fn min_consecutive_write_delay() -> Duration;

    async fn enable(&mut self, enable: bool) -> Result<(), Self::Error>;
    async fn home(&mut self) -> Result<(), Self::Error>;

    async fn set_absolute_position(&mut self, steps: i32) -> Result<(), Self::Error>;
    async fn get_absolute_position(&mut self) -> Result<i32, Self::Error>;
    async fn get_remaining_steps(&mut self) -> Result<i32, Self::Error>;

    async fn wait_for_target_reached(&mut self, threshold: i32) {
        loop {
            match self.get_remaining_steps().await {
                Ok(steps) if steps.abs() < threshold => break,
                _ => Timer::after(Self::min_consecutive_write_delay() * 2).await,
            }
        }
    }

    async fn set_speed(&mut self, rpm: u16) -> Result<(), Self::Error>;
    async fn get_speed(&mut self) -> Result<u16, Self::Error>;

    async fn set_acceleration(&mut self, value: u16) -> Result<(), Self::Error>;
    async fn get_acceleration(&mut self) -> Result<u16, Self::Error>;

    async fn set_max_output(&mut self, output: u16) -> Result<(), Self::Error>;
    async fn get_max_output(&mut self) -> Result<u16, Self::Error>;

    async fn get_current_amps(&mut self) -> Result<f32, Self::Error>;
    async fn get_voltage_volts(&mut self) -> Result<f32, Self::Error>;
}
