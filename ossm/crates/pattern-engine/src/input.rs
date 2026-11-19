use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::watch::Watch;

/// The depth, stroke, speed and sensation the user has chosen for the
/// running pattern.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PatternState {
    /// Maximum depth as a fraction of the machine range (0.0–1.0).
    pub depth: f64,
    /// Stroke as a fraction of depth (0.0–1.0).
    /// Shallowest point = `depth * (1.0 - stroke)`.
    pub stroke: f64,
    /// Speed as a fraction of max velocity (0.0–1.0).
    pub speed: f64,
    /// Sensation value (-1.0 to 1.0). Meaning is pattern-specific.
    pub sensation: f64,
}

impl PatternState {
    pub const DEFAULT: Self = Self {
        depth: 0.5,
        stroke: 0.5,
        speed: 0.0,
        sensation: 0.0,
    };

    /// Apply one update, clamped to its range.
    pub fn apply(&mut self, update: PatternUpdate) {
        match update {
            PatternUpdate::Speed(value) => self.speed = value.clamp(0.0, 1.0),
            PatternUpdate::Stroke(value) => self.stroke = value.clamp(0.0, 1.0),
            PatternUpdate::Depth(value) => self.depth = value.clamp(0.0, 1.0),
            PatternUpdate::Sensation(value) => self.sensation = value.clamp(-1.0, 1.0),
        }
    }
}

impl Default for PatternState {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A change to one part of the [`PatternState`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PatternUpdate {
    /// 0.0-1.0 (fraction of max velocity).
    Speed(f64),
    /// 0.0-1.0 (fraction of depth).
    Stroke(f64),
    /// 0.0-1.0 (fraction of machine range).
    Depth(f64),
    /// -1.0-1.0 (pattern-specific).
    Sensation(f64),
}

/// A change to playback. Unlike updates to the state, every event
/// counts, in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatternEvent {
    Play(usize),
    Pause,
    Resume,
    Stop,
    Home,
}

pub type SharedPatternState = Watch<CriticalSectionRawMutex, PatternState, 1>;
