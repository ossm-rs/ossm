use rsruckig::prelude::*;

use crate::clock::Clock;
use crate::command::{Cancelled, MotionCommand, StateCommand, StateResponse};
use crate::state::MotionPhase;
use crate::{Board, MotionLimits, Ossm};

// Floor applied to velocity requests to prevent degenerate Ruckig inputs.
const MIN_VELOCITY: f64 = 0.001;

// If motion_control.tick has not called frequently enough, the resulting movement
// is stale and potentially dangeroud. We allow the controller to fall behind by
// this many ticks before faulting. With default values this is 12mm of travel.
const STALL_FAULT_STEPS: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq)]
enum MotionState {
    Disabled,
    Enabled,
    Ready,
    Moving,
    /// Ruckig is decelerating to a smooth stop for the given reason.
    Stopping(StopReason),
    /// Motor is stationary; the instructed target is preserved for resume.
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum StopReason {
    Pause,
    Disable,
    Home,
}

/// The last-commanded motion intent, independent of what ruckig is currently
/// planning. Pause/resume manipulates the ruckig input while leaving this
/// untouched.
#[derive(Debug, Clone, Copy)]
struct MotionTarget {
    /// Target position (mm).
    position: f64,
    /// Maximum velocity (mm/s).
    velocity: f64,
    /// Torque limit as a fraction (0.0–1.0). `None` uses the motor default.
    torque: Option<f64>,
}

/// Drives the motion state machine and enforces safe motion profiles.
///
/// The controller owns a ruckig instance and generates jerk-limited
/// trajectories. Each tick, it samples the trajectory and calls
/// `board.set_position(mm)` with the next point on the curve. The board
/// is a dumb position follower — it never plans its own trajectory.
///
/// # Safety
///
/// Ruckig enforces the acceleration and jerk limits from [`MotionLimits`].
/// No upstream code (patterns, UI, remote) can cause motion that exceeds
/// these limits. The motor's internal trajectory planner is bypassed by
/// configuring it for maximum tracking speed.
///
/// # Timing
///
/// Ruckig advances on a *fixed* timestep, but the control loop cannot be
/// trusted to call [`update`](Self::update) at exactly that rate. The
/// controller therefore pulls real elapsed time from its [`Clock`] and banks it
/// in an accumulator, running as many fixed Ruckig steps as the elapsed time has
/// earned. This makes `update` robust to the call interval: calling it faster
/// than the timestep is a no-op, calling it slower runs multiple
/// steps to stay on real time. If the gap so large it would move dangerously 
/// STALL_FAULT_STEPS * max velocity, the controller faults to a safe stop.
/// Correctness no longer depends on a precise tick call cadence.
pub struct MotionController<'a, B: Board, C: Clock> {
    board: B,
    channels: &'a Ossm,
    clock: C,
    state: MotionState,
    limits: MotionLimits,
    /// The last-instructed motion target. `Some` when a move has been commanded,
    /// `None` when there is no active motion intent (e.g. disabled, just homed).
    target: Option<MotionTarget>,
    ruckig: Ruckig<1, ThrowErrorHandler>,
    input: InputParameter<1>,
    output: OutputParameter<1>,
    /// The fixed Ruckig timestep in microseconds. One accumulator drain runs this
    /// much trajectory time.
    timestep_us: u64,
    /// Real elapsed time (µs) banked but not yet consumed by a Ruckig step.
    accumulator_us: u64,
    /// Clock reading (µs) at the previous motion tick; `None` whenever no
    /// trajectory is active, so idle/homing/paused gaps are never banked.
    last_micros: Option<u64>,
}

impl<'a, B: Board, C: Clock> MotionController<'a, B, C> {
    /// Create a new `MotionController` in the `Disabled` state.
    ///
    /// `timestep_secs` is Ruckig's fixed integration step. The control loop no
    /// longer has to hit it exactly — the accumulator reconciles real elapsed
    /// time (read from `clock`) against this step — but it should be a sensible
    /// target cadence (e.g. 0.01 for 100 Hz).
    pub(crate) fn new(
        board: B,
        limits: MotionLimits,
        timestep_secs: f64,
        clock: C,
        channels: &'a Ossm,
    ) -> Self {
        let mut input = InputParameter::new(None);
        input.current_position[0] = limits.min_position_mm;
        input.target_position[0] = limits.min_position_mm;
        input.max_velocity[0] = MIN_VELOCITY;
        input.max_acceleration[0] = limits.max_acceleration_mm_s2;
        input.max_jerk[0] = limits.max_jerk_mm_s3;
        input.synchronization = Synchronization::None;
        input.duration_discretization = DurationDiscretization::Discrete;

        Self {
            board,
            channels,
            clock,
            state: MotionState::Disabled,
            limits,
            target: None,
            ruckig: Ruckig::<1, ThrowErrorHandler>::new(None, timestep_secs),
            input,
            output: OutputParameter::new(None),
            timestep_us: ((timestep_secs * 1_000_000.0) as u64).max(1),
            accumulator_us: 0,
            last_micros: None,
        }
    }

    /// Advance the motion control loop by one step.
    ///
    /// Returns `Err` if the board reports a critical fault. The caller should
    /// treat this as an unrecoverable error for this control cycle — the
    /// controller will have already transitioned to `Disabled`.
    pub async fn update(&mut self) -> Result<(), B::Error> {
        if let Err(e) = self.board.tick().await {
            log::error!("Board tick fault: {:?}", e);
            self.enter_fault();
            return Err(e);
        }

        self.tick().await?;

        if let Ok(cmd) = self.channels.state_cmd.try_receive() {
            self.process_state_command(cmd).await?;
        }

        if let Ok(cmd) = self.channels.move_cmd.try_receive() {
            self.process_move_command(cmd).await;
        }

        Ok(())
    }

    async fn process_state_command(&mut self, cmd: StateCommand) -> Result<(), B::Error> {
        match (&self.state, cmd) {
            (MotionState::Disabled, StateCommand::Enable) => {
                match self.board.enable().await {
                    Ok(()) => {
                        self.transition(MotionState::Enabled);
                        self.respond(StateResponse::Completed);
                    }
                    Err(e) => {
                        log::error!("Board enable failed: {:?}", e);
                        self.respond(StateResponse::Fault);
                        return Err(e);
                    }
                }
            }
            // Idempotent: already in the target state, nothing to do.
            // BLE remote RADR thrashes sometimes causing the catch-all
            // to trigger.
            (MotionState::Enabled, StateCommand::Enable)
            | (MotionState::Disabled, StateCommand::Disable) => {
                self.respond(StateResponse::Completed);
            }
            (MotionState::Enabled | MotionState::Ready, StateCommand::Disable) => {
                self.disable().await;
                self.respond(StateResponse::Completed);
            }
            (MotionState::Paused, StateCommand::Disable) => {
                self.channels.move_resp.signal(Err(Cancelled));
                self.disable().await;
                self.respond(StateResponse::Completed);
            }
            (MotionState::Moving, StateCommand::Disable) => {
                self.channels.move_resp.signal(Err(Cancelled));
                self.stop(StopReason::Disable);
            }
            (MotionState::Stopping(_), StateCommand::Disable) => {
                self.state = MotionState::Stopping(StopReason::Disable);
            }

            (MotionState::Enabled | MotionState::Ready, StateCommand::Home) => {
                match self.home().await {
                    Ok(()) => self.respond(StateResponse::Completed),
                    Err(e) => {
                        self.respond(StateResponse::Fault);
                        return Err(e);
                    }
                }
            }
            (MotionState::Moving, StateCommand::Home) => {
                self.channels.move_resp.signal(Err(Cancelled));
                self.stop(StopReason::Home);
            }
            (MotionState::Paused, StateCommand::Home) => {
                self.channels.move_resp.signal(Err(Cancelled));
                match self.home().await {
                    Ok(()) => self.respond(StateResponse::Completed),
                    Err(e) => {
                        self.respond(StateResponse::Fault);
                        return Err(e);
                    }
                }
            }

            (MotionState::Moving, StateCommand::Pause) => {
                self.stop(StopReason::Pause);
                self.respond(StateResponse::Completed);
            }

            (MotionState::Paused, StateCommand::Resume) => {
                self.resume().await;
                self.respond(StateResponse::Completed);
            }

            _ => {
                self.respond(StateResponse::InvalidTransition);
            }
        }

        Ok(())
    }

    async fn process_move_command(&mut self, cmd: MotionCommand) {
        match self.state {
            MotionState::Ready => {
                self.set_motion_target(cmd);
                self.apply_torque().await;
                self.transition(MotionState::Moving);
            }

            MotionState::Moving => {
                self.set_motion_target(cmd);
                self.apply_torque().await;
            }

            _ => {}
        }
    }

    /// Advance the ruckig trajectory by however much real time has elapsed, then
    /// send the latest sampled position to the board.
    ///
    /// Time is pulled from [`self.clock`](Clock), not the caller, and banked in
    /// the accumulator. We run one fixed Ruckig step per `timestep` of banked
    /// time, so the trajectory tracks wall-clock regardless of how often this is
    /// called — fast calls bank too little to step (no-op), slow calls run
    /// several. A large enough gap faults instead of dangerously lurching.
    async fn tick(&mut self) -> Result<(), B::Error> {
        if !matches!(self.state, MotionState::Moving | MotionState::Stopping(_)) {
            // No active trajectory: stop the clock so the idle gap is never
            // banked into the next move, and start the next one from a clean slate.
            self.last_micros = None;
            self.accumulator_us = 0;
            return Ok(());
        }

        let now = self.clock.now_micros();
        let dt_us = match self.last_micros {
            // saturating_sub guards against a non-monotonic clock; the first tick
            // of a trajectory has no elapsed time.
            Some(prev) => now.saturating_sub(prev),
            None => 0,
        };
        self.last_micros = Some(now);

        if dt_us > STALL_FAULT_STEPS as u64 * self.timestep_us {
            log::error!(
                "Motion loop starved: {}ms since last tick exceeds {} steps; faulting",
                dt_us / 1_000,
                STALL_FAULT_STEPS
            );
            self.enter_fault();
            return Ok(());
        }

        self.accumulator_us += dt_us;

        // Run as many fixed steps as the banked time has earned. If less than one
        // timestep has elapsed, the loop body never runs and tick() is a no-op.
        let mut last_result = None;
        while self.accumulator_us >= self.timestep_us {
            let Ok(result) = self.ruckig.update(&self.input, &mut self.output) else {
                self.accumulator_us = 0;
                return Ok(());
            };
            self.accumulator_us -= self.timestep_us;
            if !matches!(result, RuckigResult::Working | RuckigResult::Finished) {
                return Ok(());
            }
            // Feed each sub-step's result back so the next sub-step continues
            // from it.
            self.output.pass_to_input(&mut self.input);
            let finished = matches!(result, RuckigResult::Finished);
            last_result = Some(result);
            if finished {
                break;
            }
        }

        let Some(result) = last_result else {
            return Ok(());
        };

        let mm = self.output.new_position[0]
            .clamp(self.limits.min_position_mm, self.limits.max_position_mm);
        if let Err(e) = self.board.set_position(mm).await {
            log::error!("Board set_position failed: {:?}", e);
            self.enter_fault();
            return Err(e);
        }
        self.publish_state();

        if result == RuckigResult::Finished {
            // Discard any sub-timestep remainder; the resulting state decides
            // what (if anything) moves next.
            self.accumulator_us = 0;
            match self.state {
                MotionState::Stopping(StopReason::Pause) => {
                    self.transition(MotionState::Paused);
                }
                MotionState::Stopping(StopReason::Disable) => {
                    self.disable().await;
                    self.respond(StateResponse::Completed);
                }
                MotionState::Stopping(StopReason::Home) => match self.home().await {
                    Ok(()) => self.respond(StateResponse::Completed),
                    Err(e) => {
                        self.respond(StateResponse::Fault);
                        return Err(e);
                    }
                },
                _ => {
                    self.target = None;
                    self.channels.move_resp.signal(Ok(()));
                    self.transition(MotionState::Ready);
                }
            }
        }

        Ok(())
    }

    /// Run the homing sequence. Transitions to `Ready` on success, stays
    /// `Disabled` on failure.
    async fn home(&mut self) -> Result<(), B::Error> {
        if let Err(e) = self.board.home().await {
            log::error!("Board home failed: {:?}", e);
            self.transition(MotionState::Disabled);
            return Err(e);
        }

        self.input.control_interface = ControlInterface::Position;
        self.input.current_position[0] = self.limits.min_position_mm;
        self.input.target_position[0] = self.limits.min_position_mm;
        self.input.current_velocity[0] = 0.0;
        self.input.current_acceleration[0] = 0.0;

        if let Err(e) = self.board.set_position(self.limits.min_position_mm).await {
            log::error!("Board set_position after home failed: {:?}", e);
            return Err(e);
        }

        self.target = None;
        self.transition(MotionState::Ready);
        Ok(())
    }

    /// Best-effort disable. Logs errors but always transitions to `Disabled`,
    /// because there is no useful recovery if the motor won't turn off.
    async fn disable(&mut self) {
        if let Err(e) = self.board.disable().await {
            log::error!("Board disable failed: {:?}", e);
        }
        self.input.control_interface = ControlInterface::Position;
        self.target = None;
        self.transition(MotionState::Disabled);
    }

    fn stop(&mut self, reason: StopReason) {
        // Switch to velocity control and target zero velocity. Ruckig handles
        // the jerk-limited deceleration trajectory — no manual math needed.
        self.input.control_interface = ControlInterface::Velocity;
        self.input.target_velocity[0] = 0.0;
        self.output.time = 0.0;
        self.transition(MotionState::Stopping(reason));
    }

    async fn resume(&mut self) {
        // Switch back to position control and restore the instructed target.
        self.input.control_interface = ControlInterface::Position;
        self.sync_ruckig();
        self.apply_torque().await;
        self.transition(MotionState::Moving);
    }

    /// Cancel any in-flight motion and transition to `Disabled`.
    ///
    /// Called when `board.tick()` reports a critical fault. Signals appropriate
    /// responses on the channels so callers aren't left waiting.
    fn enter_fault(&mut self) {
        match self.state {
            MotionState::Moving | MotionState::Paused => {
                self.channels.move_resp.signal(Err(Cancelled));
            }
            MotionState::Stopping(StopReason::Pause) => {
                self.channels.move_resp.signal(Err(Cancelled));
            }
            MotionState::Stopping(StopReason::Disable | StopReason::Home) => {
                self.respond(StateResponse::Fault);
            }
            _ => {}
        }
        self.target = None;
        self.transition(MotionState::Disabled);
    }

    fn respond(&self, resp: StateResponse) {
        self.channels.state_resp.signal(resp);
    }

    fn fraction_to_mm(&self, fraction: f64) -> f64 {
        let mm = self.limits.min_position_mm
            + fraction * (self.limits.max_position_mm - self.limits.min_position_mm);
        mm.clamp(self.limits.min_position_mm, self.limits.max_position_mm)
    }

    fn fraction_to_velocity(&self, fraction: f64) -> f64 {
        let mm_s = fraction * self.limits.max_velocity_mm_s;
        mm_s.clamp(MIN_VELOCITY, self.limits.max_velocity_mm_s)
    }

    fn set_motion_target(&mut self, cmd: MotionCommand) {
        self.target = Some(MotionTarget {
            position: self.fraction_to_mm(cmd.position),
            velocity: self.fraction_to_velocity(cmd.speed),
            torque: cmd.torque,
        });
        self.sync_ruckig();
    }

    /// Write the instructed target into ruckig's input parameters and reset
    /// the trajectory timer so ruckig replans.
    fn sync_ruckig(&mut self) {
        if let Some(target) = &self.target {
            self.input.target_position[0] = target.position;
            self.input.max_velocity[0] = target.velocity;
            self.output.time = 0.0;
            self.ruckig.reset();
        }
    }

    async fn apply_torque(&mut self) {
        let fraction = self.target.as_ref().and_then(|t| t.torque).unwrap_or(1.0);
        if let Err(e) = self.board.set_torque(fraction).await {
            log::error!("Board set_torque failed: {:?}", e);
            self.enter_fault();
        }
    }

    fn phase(&self) -> MotionPhase {
        match self.state {
            MotionState::Disabled => MotionPhase::Disabled,
            MotionState::Enabled => MotionPhase::Enabled,
            MotionState::Ready => MotionPhase::Ready,
            MotionState::Moving => MotionPhase::Moving,
            MotionState::Stopping(_) => MotionPhase::Stopping,
            MotionState::Paused => MotionPhase::Paused,
        }
    }

    fn mm_to_fraction(&self, mm: f64) -> f32 {
        let range = self.limits.max_position_mm - self.limits.min_position_mm;
        if range <= 0.0 {
            return 0.0;
        }
        ((mm - self.limits.min_position_mm) / range) as f32
    }

    fn velocity_to_fraction(&self, mm_s: f64) -> f32 {
        if self.limits.max_velocity_mm_s <= 0.0 {
            return 0.0;
        }
        (mm_s / self.limits.max_velocity_mm_s) as f32
    }

    fn acceleration_to_fraction(&self, mm_s2: f64) -> f32 {
        if self.limits.max_acceleration_mm_s2 <= 0.0 {
            return 0.0;
        }
        (mm_s2 / self.limits.max_acceleration_mm_s2) as f32
    }

    fn publish_state(&self) {
        let position_mm = self.output.new_position[0]
            .clamp(self.limits.min_position_mm, self.limits.max_position_mm);
        let velocity_mm_s = self.output.new_velocity[0];
        let acceleration_mm_s2 = self.output.new_acceleration[0];
        let torque = self.target.as_ref().and_then(|t| t.torque).unwrap_or(1.0);

        self.channels.motion_state.update(crate::state::MotionState {
            phase: self.phase(),
            position: self.mm_to_fraction(position_mm),
            velocity: self.velocity_to_fraction(velocity_mm_s.abs()),
            acceleration: self.acceleration_to_fraction(acceleration_mm_s2.abs()),
            torque: torque as f32,
        });
    }

    fn transition(&mut self, new_state: MotionState) {
        self.state = new_state;
        self.publish_state();
        self.channels.motion_state.publish_phase(self.phase());
    }
}
