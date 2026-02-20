#![no_std]

mod motor;
mod board;
mod mechanical;

pub use motor::Motor;
pub use board::Board;
pub use mechanical::MechanicalConfig;

pub struct Sossm<B: Board> {
    board: B
}

impl<B: Board> Sossm<B> {
    pub fn new(board: B) -> Self {
        Self { board }
    }
}