#![no_std]

pub trait Motor {
    fn new() -> Self;
}

pub trait Board {
    fn new() -> Self;
}

pub struct Sossm {}

impl Sossm {
    pub fn new() -> Self {
        Self {}
    }

    pub fn run(&self) {
        // Run the Sossm system
    }
}
