#![no_std]

//! Protocol-independent control facade and owner motion policy.
//!
//! External integrations (XToys, M5, owner UI, future remote protocols) use
//! [`ControlSender`] rather than reaching into `pattern-engine` or `ossm`
//! directly. Protocol-specific state belongs in the protocol crate.

use core::sync::atomic::{AtomicBool, Ordering};
use portable_atomic::AtomicU64;
use ossm::{MotionCommand, MotionPhase, MotionSender, StateResponse};
use embassy_time::{Duration, Timer};
use pattern_engine::{EngineState, PatternInput, PatternSender};

pub const HARD_MAX_MACHINE_SPEED_MM_S: f64 = 900.0;
pub const HARD_MAX_MACHINE_TRAVEL_MM: f64 = 400.0;
pub const DEFAULT_MACHINE_SPEED_MM_S: f64 = 600.0;
pub const DEFAULT_MACHINE_TRAVEL_MM: f64 = 180.0;
pub const DEFAULT_OWNER_MAX_SPEED_MM_S: f64 = 150.0;
pub const DEFAULT_OWNER_MIN_STROKE_MM: f64 = 0.0;
pub const DEFAULT_OWNER_MAX_STROKE_MM: f64 = 100.0;
pub const DEFAULT_OWNER_MIN_DEPTH_MM: f64 = 0.0;
pub const DEFAULT_OWNER_MAX_DEPTH_MM: f64 = 160.0;
pub const FALLBACK_MAX_SPEED_MM_S: f64 = 10.0;
pub const FALLBACK_MIN_STROKE_MM: f64 = 0.0;
pub const FALLBACK_MAX_STROKE_MM: f64 = 20.0;
pub const FALLBACK_MIN_DEPTH_MM: f64 = 0.0;
pub const FALLBACK_MAX_DEPTH_MM: f64 = 20.0;

static MACHINE_MAX_SPEED: AtomicU64 = AtomicU64::new(DEFAULT_MACHINE_SPEED_MM_S.to_bits());
static MACHINE_TRAVEL: AtomicU64 = AtomicU64::new(DEFAULT_MACHINE_TRAVEL_MM.to_bits());
static OWNER_MAX_SPEED: AtomicU64 = AtomicU64::new(DEFAULT_OWNER_MAX_SPEED_MM_S.to_bits());
static OWNER_MIN_STROKE: AtomicU64 = AtomicU64::new(DEFAULT_OWNER_MIN_STROKE_MM.to_bits());
static OWNER_MAX_STROKE: AtomicU64 = AtomicU64::new(DEFAULT_OWNER_MAX_STROKE_MM.to_bits());
static OWNER_MIN_DEPTH: AtomicU64 = AtomicU64::new(DEFAULT_OWNER_MIN_DEPTH_MM.to_bits());
static OWNER_MAX_DEPTH: AtomicU64 = AtomicU64::new(DEFAULT_OWNER_MAX_DEPTH_MM.to_bits());
static MASTER_ENABLED: AtomicBool = AtomicBool::new(false);
static OWNER_SESSION: AtomicBool = AtomicBool::new(false);
static ESTOP: AtomicBool = AtomicBool::new(false);

static REQ_DEPTH: AtomicU64 = AtomicU64::new(PatternInput::DEFAULT.depth.to_bits());
static REQ_STROKE: AtomicU64 = AtomicU64::new(PatternInput::DEFAULT.stroke.to_bits());
static REQ_SPEED: AtomicU64 = AtomicU64::new(PatternInput::DEFAULT.velocity.to_bits());
static REQ_SENSATION: AtomicU64 = AtomicU64::new(PatternInput::DEFAULT.sensation.to_bits());

#[derive(Clone, Copy, Debug)]
pub struct OwnerLimits {
    pub max_speed: f64,
    pub min_stroke: f64,
    pub max_stroke: f64,
    pub min_depth: f64,
    pub max_depth: f64,
}

pub mod policy {
    use super::*;

    pub fn configure_machine(max_speed: f64, travel: f64) {
        MACHINE_MAX_SPEED.store(max_speed.clamp(1.0, HARD_MAX_MACHINE_SPEED_MM_S).to_bits(), Ordering::Release);
        MACHINE_TRAVEL.store(travel.clamp(1.0, HARD_MAX_MACHINE_TRAVEL_MM).to_bits(), Ordering::Release);
    }
    pub fn machine_max_speed_mm_s() -> f64 { f64::from_bits(MACHINE_MAX_SPEED.load(Ordering::Acquire)) }
    pub fn machine_travel_mm() -> f64 { f64::from_bits(MACHINE_TRAVEL.load(Ordering::Acquire)) }

    pub fn set_master_enabled(v: bool) {
        MASTER_ENABLED.store(v, Ordering::Release);
        if !v { OWNER_SESSION.store(false, Ordering::Release); }
    }
    pub fn master_enabled() -> bool { MASTER_ENABLED.load(Ordering::Acquire) }
    pub fn activate_owner_session() -> bool {
        if !master_enabled() { return false; }
        OWNER_SESSION.store(true, Ordering::Release);
        true
    }
    pub fn deactivate_owner_session() { OWNER_SESSION.store(false, Ordering::Release); }
    pub fn owner_session_active() -> bool { OWNER_SESSION.load(Ordering::Acquire) }

    pub fn set_estop_active(v: bool) {
        ESTOP.store(v, Ordering::Release);
        if v {
            OWNER_SESSION.store(false, Ordering::Release);
            REQ_SPEED.store(0.0f64.to_bits(), Ordering::Release);
        }
    }
    pub fn estop_active() -> bool { ESTOP.load(Ordering::Acquire) }

    pub fn set_owner_limits(max_speed: f64, min_stroke: f64, max_stroke: f64, min_depth: f64, max_depth: f64) {
        let machine_speed = machine_max_speed_mm_s();
        let travel = machine_travel_mm();
        let min_stroke = min_stroke.clamp(0.0, travel);
        let min_depth = min_depth.clamp(0.0, travel);
        OWNER_MAX_SPEED.store(max_speed.clamp(0.0, machine_speed).to_bits(), Ordering::Release);
        OWNER_MIN_STROKE.store(min_stroke.to_bits(), Ordering::Release);
        OWNER_MAX_STROKE.store(max_stroke.clamp(min_stroke, travel).to_bits(), Ordering::Release);
        OWNER_MIN_DEPTH.store(min_depth.to_bits(), Ordering::Release);
        OWNER_MAX_DEPTH.store(max_depth.clamp(min_depth, travel).to_bits(), Ordering::Release);
    }
    pub fn owner_limits() -> OwnerLimits {
        OwnerLimits {
            max_speed: f64::from_bits(OWNER_MAX_SPEED.load(Ordering::Acquire)),
            min_stroke: f64::from_bits(OWNER_MIN_STROKE.load(Ordering::Acquire)),
            max_stroke: f64::from_bits(OWNER_MAX_STROKE.load(Ordering::Acquire)),
            min_depth: f64::from_bits(OWNER_MIN_DEPTH.load(Ordering::Acquire)),
            max_depth: f64::from_bits(OWNER_MAX_DEPTH.load(Ordering::Acquire)),
        }
    }
    pub fn fallback_limits() -> OwnerLimits {
        OwnerLimits {
            max_speed: FALLBACK_MAX_SPEED_MM_S.min(machine_max_speed_mm_s()),
            min_stroke: FALLBACK_MIN_STROKE_MM.min(machine_travel_mm()),
            max_stroke: FALLBACK_MAX_STROKE_MM.min(machine_travel_mm()),
            min_depth: FALLBACK_MIN_DEPTH_MM.min(machine_travel_mm()),
            max_depth: FALLBACK_MAX_DEPTH_MM.min(machine_travel_mm()),
        }
    }
    pub fn active_limits() -> Option<OwnerLimits> { if master_enabled() { Some(owner_limits()) } else { None } }

    pub fn set_requested(input: PatternInput) {
        REQ_DEPTH.store(input.depth.clamp(0.0,1.0).to_bits(), Ordering::Release);
        REQ_STROKE.store(input.stroke.clamp(0.0,1.0).to_bits(), Ordering::Release);
        REQ_SPEED.store(input.velocity.clamp(0.0,1.0).to_bits(), Ordering::Release);
        REQ_SENSATION.store(input.sensation.clamp(-1.0,1.0).to_bits(), Ordering::Release);
    }
    pub fn requested() -> PatternInput {
        PatternInput {
            depth: f64::from_bits(REQ_DEPTH.load(Ordering::Acquire)),
            stroke: f64::from_bits(REQ_STROKE.load(Ordering::Acquire)),
            velocity: f64::from_bits(REQ_SPEED.load(Ordering::Acquire)),
            sensation: f64::from_bits(REQ_SENSATION.load(Ordering::Acquire)),
        }
    }
    pub fn set_requested_speed(v:f64){REQ_SPEED.store(v.clamp(0.0,1.0).to_bits(),Ordering::Release)}
    pub fn set_requested_depth(v:f64){REQ_DEPTH.store(v.clamp(0.0,1.0).to_bits(),Ordering::Release)}
    pub fn set_requested_stroke(v:f64){REQ_STROKE.store(v.clamp(0.0,1.0).to_bits(),Ordering::Release)}
    pub fn set_requested_sensation(v:f64){REQ_SENSATION.store(v.clamp(-1.0,1.0).to_bits(),Ordering::Release)}

    pub fn clamp_geometry_mm(stroke: f64, depth: f64, l: OwnerLimits) -> (f64, f64) {
        let width = (l.max_depth-l.min_depth).max(0.0);
        let mut s = stroke.clamp(l.min_stroke,l.max_stroke).min(width);
        let mut d = depth.clamp(l.min_depth,l.max_depth);
        if d-s < l.min_depth { d = l.min_depth+s; }
        if d > l.max_depth { d = l.max_depth; }
        if d-s < l.min_depth { s = (d-l.min_depth).max(0.0); }
        (s,d)
    }

    pub fn clamp_pattern(mut i: PatternInput) -> PatternInput {
        i.depth=i.depth.clamp(0.0,1.0); i.stroke=i.stroke.clamp(0.0,1.0);
        i.velocity=i.velocity.clamp(0.0,1.0); i.sensation=i.sensation.clamp(-1.0,1.0);
        if estop_active(){i.velocity=0.0; return i;}
        let Some(l)=active_limits() else{return i};
        let ms=machine_max_speed_mm_s().max(0.001); let tr=machine_travel_mm().max(0.001);
        i.velocity=(i.velocity*ms).min(l.max_speed)/ms;
        let (s,d)=clamp_geometry_mm(i.stroke*tr,i.depth*tr,l);
        i.stroke=s/tr; i.depth=d/tr; i
    }

    pub fn clamp_direct_position(position:f64)->f64 {
        let p=position.clamp(0.0,1.0); let tr=machine_travel_mm().max(0.001);
        let Some(l)=active_limits() else{return p};
        ((p*tr).clamp(l.min_depth,l.max_depth)/tr).clamp(0.0,1.0)
    }
    pub fn map_position_into_active_geometry(position:f64)->f64 {
        let p=position.clamp(0.0,1.0); let i=clamp_pattern(requested());
        let stroke=i.stroke.min(i.depth); let shallow=i.depth-stroke;
        (shallow+p*stroke).clamp(0.0,1.0)
    }
    pub fn clamp_speed_fraction(speed:f64)->f64 {
        if estop_active(){return 0.0;}
        let ms=machine_max_speed_mm_s().max(0.001);
        let ceiling=active_limits().map(|l|l.max_speed).unwrap_or(ms).clamp(0.0,ms);
        ((speed.clamp(0.0,1.0)*ms).min(ceiling)/ms).clamp(0.0,1.0)
    }
    pub fn timed_speed_fraction(current:f64,target:f64,time_ms:u32)->f64 {
        let ms=machine_max_speed_mm_s().max(0.001); let tr=machine_travel_mm().max(0.001);
        let requested=if time_ms==0 || time_ms>=100_000_000 {ms} else {(target-current).abs()*tr/(time_ms as f64/1000.0).max(0.001)};
        clamp_speed_fraction(requested/ms)
    }
}

/// Generic facade used by external control integrations.
///
/// `pattern-engine` remains protocol-agnostic: normal pattern controls are
/// delegated to `PatternSender`, while direct absolute targets use the OSSM
/// `MotionSender` through this policy boundary.
pub struct ControlSender {
    patterns: &'static PatternSender,
    motion: &'static MotionSender,
}

impl ControlSender {
    pub const fn new(patterns:&'static PatternSender,motion:&'static MotionSender)->Self{Self{patterns,motion}}
    pub fn engine_state(&self)->EngineState{self.patterns.state()}
    pub fn pattern_sender(&self)->&'static PatternSender{self.patterns}
    pub fn input(&self)->PatternInput{self.patterns.input()}
    pub fn motion_phase(&self)->MotionPhase{self.motion.state().phase}
    pub fn motion_position(&self)->f64{self.motion.state().position as f64}
    pub fn play(&self,idx:usize){if !policy::estop_active(){self.patterns.play(idx)}}
    pub fn pause(&self){self.patterns.pause()}
    pub fn resume(&self){if !policy::estop_active(){self.patterns.resume()}}
    pub fn stop(&self){self.patterns.stop()}
    pub fn home(&self){if !policy::estop_active(){self.patterns.home()}}

    fn apply_effective_pattern(&self){
        let e=policy::clamp_pattern(policy::requested());
        self.patterns.set_depth(e.depth); self.patterns.set_stroke(e.stroke);
        self.patterns.set_speed(e.velocity); self.patterns.set_sensation(e.sensation);
    }
    pub fn set_speed(&self,v:f64){policy::set_requested_speed(v);self.apply_effective_pattern()}
    pub fn set_depth(&self,v:f64){policy::set_requested_depth(v);self.apply_effective_pattern()}
    pub fn set_stroke(&self,v:f64){policy::set_requested_stroke(v);self.apply_effective_pattern()}
    pub fn set_sensation(&self,v:f64){policy::set_requested_sensation(v);self.apply_effective_pattern()}
    pub fn reapply_policy(&self){self.apply_effective_pattern()}

    pub async fn disable_motion(&self)->StateResponse{self.motion.disable().await}

    /// Direct-position lane for protocols that provide absolute/timed targets.
    /// This is deliberately generic and contains no XToys semantics.
    pub async fn direct_move(&self, normalized_position:f64, time_ms:u32) {
        if policy::estop_active(){return;}

        // Clean ownership handoff: the pattern engine explicitly yields while
        // the OSSM controller stays homed/Ready. If no home has occurred yet,
        // request the normal pattern-engine home sequence first.
        match self.patterns.state() {
            EngineState::Idle => {
                self.patterns.home();
                while self.patterns.state() != EngineState::Ready {
                    Timer::after(Duration::from_millis(10)).await;
                    if policy::estop_active(){ return; }
                }
            }
            EngineState::Playing(_) | EngineState::Paused(_) => {
                self.patterns.yield_motion().await;
                while self.patterns.state() != EngineState::Ready {
                    Timer::after(Duration::from_millis(10)).await;
                    if policy::estop_active(){ return; }
                    if self.patterns.state() == EngineState::Idle { return; }
                }
            },
            EngineState::Homing => {
                while self.patterns.state() == EngineState::Homing {
                    Timer::after(Duration::from_millis(10)).await;
                    if policy::estop_active(){ return; }
                }
                if self.patterns.state() != EngineState::Ready { return; }
            }
            EngineState::Ready => {}
        }

        let target=policy::map_position_into_active_geometry(normalized_position);
        let speed=policy::timed_speed_fraction(self.motion_position(),target,time_ms);
        let cmd=MotionCommand{position:target,speed,jerk:0.5,torque:None};
        if self.motion.state().phase==MotionPhase::Ready { self.motion.begin_motion(cmd); } else { self.motion.update_motion(cmd); }
    }

    pub async fn retract(&self){self.direct_move(0.0,0).await}
    pub async fn extend(&self){self.direct_move(1.0,0).await}
}
