use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;

pub type CommandChannel = Channel<CriticalSectionRawMutex, Command, 8>;

#[derive(Debug, Clone, Copy)]
pub enum Command {
    Enable,
    Disable,
    MoveTo(f64),
    SetSpeed(f64),
}
