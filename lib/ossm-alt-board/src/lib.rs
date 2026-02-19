#![no_std]

use sossm::Board;

pub struct OssmAltBoard;

impl Board for OssmAltBoard {
    fn new() -> Self {
        OssmAltBoard
    }
}
