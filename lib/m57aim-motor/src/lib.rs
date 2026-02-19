#![no_std]

use sossm::Motor;

pub struct M57AIMMotor;

impl Motor for M57AIMMotor {
    fn new() -> Self {
        M57AIMMotor
    }
}
