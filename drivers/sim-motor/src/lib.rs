#![no_std]

use core::convert::Infallible;
use core::sync::atomic::{AtomicI32, Ordering};

pub struct SimMotor {
    position: i32,
    shared_position: &'static AtomicI32,
    enabled: bool,
}

impl SimMotor {
    pub const STEPS_PER_REV: u32 = 32_768;
    pub const MAX_OUTPUT: u16 = 600;

    pub fn new(shared_position: &'static AtomicI32) -> Self {
        Self {
            position: 0,
            shared_position,
            enabled: false,
        }
    }

    pub async fn enable(&mut self) -> Result<(), Infallible> {
        self.enabled = true;
        Ok(())
    }

    pub async fn disable(&mut self) -> Result<(), Infallible> {
        self.enabled = false;
        Ok(())
    }

    pub async fn home(&mut self) -> Result<(), Infallible> {
        self.position = 0;
        self.shared_position.store(0, Ordering::Relaxed);
        Ok(())
    }

    pub async fn set_absolute_position(&mut self, steps: i32) -> Result<(), Infallible> {
        self.position = steps;
        self.shared_position.store(steps, Ordering::Relaxed);
        Ok(())
    }

    pub fn position(&self) -> i32 {
        self.position
    }
}
