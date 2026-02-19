use m57aim_motor::M57AIMMotor;
use ossm_alt_board::OssmAltBoard;
use sossm::{Board, Motor, Sossm};

fn main() {
    let motor = M57AIMMotor::new();
    let board = OssmAltBoard::new();
    let sossm = Sossm::new();

    loop {
        sossm.run();
    }
}
