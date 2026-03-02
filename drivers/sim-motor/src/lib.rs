#![no_std]

use core::convert::Infallible;
use core::sync::atomic::{AtomicI32, Ordering};

use sossm::{Motor, MotorTelemetry};

const NOMINAL_VOLTAGE: f32 = 24.0;

pub struct SimMotor {
    position: i32,
    shared_position: &'static AtomicI32,
    speed: u16,
    acceleration: u16,
    max_output: u16,
    enabled: bool,
}

impl SimMotor {
    pub fn new(shared_position: &'static AtomicI32) -> Self {
        Self {
            position: 0,
            shared_position,
            speed: 0,
            acceleration: 0,
            max_output: 0,
            enabled: false,
        }
    }
}

impl Motor for SimMotor {
    type Error = Infallible;

    const STEPS_PER_REV: u32 = 32_768;

    async fn enable(&mut self) -> Result<(), Self::Error> {
        self.enabled = true;
        Ok(())
    }

    async fn disable(&mut self) -> Result<(), Self::Error> {
        self.enabled = false;
        Ok(())
    }

    async fn home(&mut self) -> Result<(), Self::Error> {
        self.position = 0;
        self.shared_position.store(0, Ordering::Relaxed);
        Ok(())
    }

    async fn set_absolute_position(&mut self, steps: i32) -> Result<(), Self::Error> {
        self.position = steps;
        self.shared_position.store(steps, Ordering::Relaxed);
        Ok(())
    }

    async fn set_speed(&mut self, rpm: u16) -> Result<(), Self::Error> {
        self.speed = rpm;
        Ok(())
    }

    async fn set_acceleration(&mut self, value: u16) -> Result<(), Self::Error> {
        self.acceleration = value;
        Ok(())
    }

    async fn set_max_output(&mut self, output: u16) -> Result<(), Self::Error> {
        self.max_output = output;
        Ok(())
    }
}

impl MotorTelemetry for SimMotor {
    type Error = Infallible;

    async fn get_absolute_position(&mut self) -> Result<i32, Self::Error> {
        Ok(self.position)
    }

    async fn get_remaining_steps(&mut self) -> Result<i32, Self::Error> {
        Ok(0)
    }

    async fn get_speed(&mut self) -> Result<u16, Self::Error> {
        Ok(self.speed)
    }

    async fn get_acceleration(&mut self) -> Result<u16, Self::Error> {
        Ok(self.acceleration)
    }

    async fn get_max_output(&mut self) -> Result<u16, Self::Error> {
        Ok(self.max_output)
    }

    async fn get_current_amps(&mut self) -> Result<f32, Self::Error> {
        Ok(0.0)
    }

    async fn get_voltage_volts(&mut self) -> Result<f32, Self::Error> {
        Ok(NOMINAL_VOLTAGE)
    }
}
