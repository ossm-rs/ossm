use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::channel::Channel;
use embassy_sync::signal::Signal;

pub type CommandChannel = Channel<CriticalSectionRawMutex, Command, 8>;
pub type HomingSignal = Signal<CriticalSectionRawMutex, ()>;

#[derive(Debug, Clone, Copy)]
pub enum Command {
    Enable,
    Disable,
    Home,
    MoveTo(f64),
    SetSpeed(f64),
}
